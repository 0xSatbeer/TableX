//! Status bar (QX-018). Its own entity so the memory readout re-renders only this bar.

use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::EditorState,
    tooltip::Tooltip,
};
use gpui_kit::*;

use gpui_kit::prelude::FluentBuilder as _;

use crate::grid::{Grid, SelectionKey};
use crate::model::{Connection, ConnectionId, ConnectionParams, ConnectionStatus, Value, Workspace};
use crate::tabs::Origin;
use crate::{ToggleBottomPanel, ToggleExplorer, theme};

/// Larger selections show their shape only, so the bar never walks millions of cells.
const AGGREGATE_MAX: usize = 1_000_000;
const MEMORY_INTERVAL: Duration = Duration::from_secs(5);

pub struct StatusBar {
    /// Where the active origin's connection status comes from.
    workspace: Entity<Workspace>,
    /// Started with `--demo`, where a tab without an origin shows sample data.
    demo: bool,
    /// Where the active tab's data comes from; `None` for no connection (sample data in the demo) or no tab.
    origin: Option<Origin>,
    /// Some tab is open, so a missing origin means sample data rather than nothing.
    has_tab: bool,
    memory_mb: Option<u64>,
    /// 1-based line and column of the focused console editor, and its selection length in chars.
    cursor: Option<(u32, u32, usize)>,
    _editor: Option<Subscription>,
    /// Aggregate of the active grid's selection, and the selection it was computed for.
    selection: Option<String>,
    grid_focused: bool,
    selection_key: Option<SelectionKey>,
    _grid: Option<Subscription>,
    /// Runs only while the window is active; dropping it stops sampling.
    sampler: Option<Task<()>>,
    _activation: Subscription,
    _workspace: Subscription,
}

/// Connect or Retry clicked for connection `.0`: connect it as the tree's Retry does.
pub struct Connect(pub ConnectionId);

impl EventEmitter<Connect> for StatusBar {}

/// What the bar shows for the active tab's connection.
struct Readout {
    /// A solid dot when Connected, else a ring.
    solid: bool,
    tip: String,
    text: Option<&'static str>,
    danger: bool,
    button: Option<&'static str>,
    /// The reason and what to check, on hovering the text.
    hint: Option<String>,
}

fn readout(conn: &Connection) -> Readout {
    let (solid, tip) = crate::explorer::status_style(&conn.status);
    let (text, button, hint) = match &conn.status {
        ConnectionStatus::Connected => (None, None, None),
        ConnectionStatus::Connecting => (Some("Connecting…"), None, None),
        ConnectionStatus::Disconnected => (Some("Not connected"), Some("Connect"), None),
        ConnectionStatus::Error(reason) => {
            let check = match &conn.params {
                ConnectionParams::Postgres { host, port, .. } => format!("Check that {host}:{port} is reachable (VPN, firewall), then retry."),
                ConnectionParams::Sqlite { .. } => "Check that the file exists and is readable.".into(),
            };
            (Some("Could not connect"), Some("Retry"), Some(format!("{reason}\n{check}")))
        }
    };
    Readout { solid, tip, text, danger: matches!(conn.status, ConnectionStatus::Error(_)), button, hint }
}

impl StatusBar {
    pub fn new(workspace: Entity<Workspace>, demo: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            workspace: workspace.clone(),
            demo,
            origin: None,
            has_tab: false,
            memory_mb: None,
            cursor: None,
            _editor: None,
            selection: None,
            grid_focused: false,
            selection_key: None,
            _grid: None,
            sampler: None,
            _activation: cx.observe_window_activation(window, Self::sync_sampler),
            _workspace: cx.observe(&workspace, |_, _, cx| cx.notify()),
        };
        this.sync_sampler(window, cx);
        this
    }

    pub fn set_origin(&mut self, origin: Option<Origin>, has_tab: bool, cx: &mut Context<Self>) {
        if (&self.origin, self.has_tab) != (&origin, has_tab) {
            (self.origin, self.has_tab) = (origin, has_tab);
            cx.notify();
        }
    }

    /// Follow `editor`'s cursor, or stop showing one when `None`.
    pub fn set_editor(&mut self, editor: Option<&Entity<EditorState>>, cx: &mut Context<Self>) {
        let position = |editor: &Entity<EditorState>, cx: &App| {
            let editor = editor.read(cx);
            let p = editor.cursor_position();
            (p.line + 1, p.character + 1, editor.selected_text().len_chars())
        };
        self._editor = editor.map(|editor| {
            // The editor notifies for more than cursor moves; re-render only when the readout changes.
            cx.observe(editor, move |this, editor, cx| {
                let cursor = Some(position(&editor, cx));
                if this.cursor != cursor {
                    this.cursor = cursor;
                    cx.notify();
                }
            })
        });
        self.cursor = editor.map(|editor| position(editor, cx));
        cx.notify();
    }

    /// Follow `grid`'s selection aggregate, or hide it when `None`.
    pub fn set_grid(&mut self, grid: Option<&Entity<Grid>>, cx: &mut Context<Self>) {
        self.selection_key = None;
        self._grid = grid.map(|grid| {
            // The grid notifies on scroll too; re-aggregate only when the selection changes.
            cx.observe(grid, |this, grid, cx| this.sync_selection(&grid, cx))
        });
        match grid {
            Some(grid) => self.sync_selection(grid, cx),
            None => self.selection = None,
        }
        cx.notify();
    }

    /// The aggregate as drawn: only while the grid holds focus.
    pub fn aggregate_text(&self) -> Option<String> {
        self.selection.clone().filter(|_| self.grid_focused)
    }

    pub fn set_grid_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.grid_focused != focused {
            self.grid_focused = focused;
            cx.notify();
        }
    }

    fn sync_selection(&mut self, grid: &Entity<Grid>, cx: &mut Context<Self>) {
        let key = grid.read(cx).selection_key();
        if self.selection_key.as_ref() != Some(&key) {
            self.selection = grid.read(cx).selection_summary();
            self.selection_key = Some(key);
            cx.notify();
        }
    }

    fn sync_sampler(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.is_window_active() {
            self.sampler = None;
        } else if self.sampler.is_none() {
            self.sampler = Some(cx.spawn(async move |this, cx| {
                loop {
                    let mb = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| rss_anon_mb(&s));
                    let alive = this.update(cx, |this, cx| {
                        if this.memory_mb != mb {
                            this.memory_mb = mb;
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                    cx.background_executor().timer(MEMORY_INTERVAL).await;
                }
            }));
        }
    }
}

/// Private memory (`RssAnon`) in MB, the number our budgets use (TICKET_MASTER §1).
/// Linux only; elsewhere `/proc/self/status` is missing and the readout hides.
fn rss_anon_mb(proc_status: &str) -> Option<u64> {
    let line = proc_status.lines().find(|l| l.starts_with("RssAnon:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// Status-bar readout for a `rows` × `cols` selection; `None` for one cell or none.
pub fn aggregate(rows: usize, cols: usize, values: impl IntoIterator<Item = Value>) -> Option<String> {
    if rows * cols <= 1 {
        return None;
    }
    let plural = |n: usize, one: &str| if n == 1 { format!("{n} {one}") } else { format!("{n} {one}s") };
    let shape = format!("{} × {}", plural(rows, "row"), plural(cols, "col"));
    if rows.saturating_mul(cols) > AGGREGATE_MAX {
        return Some(format!("{shape} · aggregate skipped (too many cells)"));
    }
    // Ints sum exactly in i128 (no i64 overflow); floats join only when there is one.
    let (mut count, mut numeric, mut ints, mut floats) = (0, 0, 0i128, None);
    for v in values {
        match v {
            Value::Null => continue,
            Value::Int(n) => (numeric, ints) = (numeric + 1, ints + n as i128),
            Value::Float(x) => (numeric, floats) = (numeric + 1, Some(floats.unwrap_or(0.0) + x)),
            _ => {}
        }
        count += 1;
    }
    let mut out = format!("{shape} · count {count}");
    if numeric > 0 {
        let total = ints as f64 + floats.unwrap_or(0.0);
        let sum = if floats.is_some() { decimal(total) } else { ints.to_string() };
        out += &format!(" · sum {sum} · avg {}", decimal(total / numeric as f64));
    }
    Some(out)
}

/// At most 4 decimals, trailing zeros trimmed: `0.75`, `0.3333`, `4`.
fn decimal(x: f64) -> String {
    let s = format!("{x:.4}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s == "-0" { "0".into() } else { s.into() }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let toggle = |id: &'static str, icon: IconName, tip: &'static str, action: fn() -> Box<dyn Action>| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip_with_action(tip, &*action(), None)
                .on_click(move |_, window, cx| window.dispatch_action(action(), cx))
        };
        let readout = self.origin.as_ref().and_then(|o| self.workspace.read(cx).connections.iter().find(|c| c.id == o.connection)).map(readout);
        let connected = readout.as_ref().is_some_and(|r| r.solid);
        let tip = |selector: String, text: String| {
            move |window: &mut Window, cx: &mut App| {
                let (selector, text) = (selector.clone(), text.clone());
                Tooltip::element(move |_, _| div().debug_selector(|| selector.clone()).child(text.clone())).build(window, cx)
            }
        };
        let context = match &self.origin {
            Some(o) => h_flex()
                .min_w_0()
                .gap(theme::space_6())
                .child({
                    let color = theme::connection_color(o.color, theme);
                    let dot = div().id("status-dot").flex_none().size(theme::space_6()).rounded_full();
                    match &readout {
                        Some(r) if !r.solid => dot.debug_selector(|| "status-dot-ring".into()).border_2().border_color(color),
                        _ => dot.debug_selector(|| "status-dot-solid".into()).bg(color),
                    }
                    .when_some(readout.as_ref(), |d, r| d.tooltip(tip(format!("status-tip:{}", r.tip), r.tip.clone())))
                })
                .child(
                    div()
                        .debug_selector(|| "status-path".into())
                        .min_w_0()
                        .truncate()
                        .text_color(theme.foreground)
                        .font_family(theme.mono_font_family.clone())
                        .child(theme::truncating("status-path", StyledText::new(o.path.clone()), o.path.clone())),
                )
                .when(o.production, |d| d.child(theme::prod_badge(theme)))
                .when_some(readout.as_ref().and_then(|r| r.text.map(|t| (t, r))), |d, (text, r)| {
                    let selector = format!("{}:{text}", if r.danger { "status-text-danger" } else { "status-text" });
                    d.child(
                        div()
                            .id("status-text")
                            .flex_none()
                            .whitespace_nowrap()
                            .debug_selector(|| selector)
                            .when(r.danger, |d| d.text_color(theme.danger))
                            .when_some(r.hint.clone(), |d, hint| d.tooltip(tip(format!("status-hint:{hint}"), hint)))
                            .child(text),
                    )
                })
                .when_some(readout.as_ref().and_then(|r| r.button), |d, label| {
                    let id = o.connection;
                    d.child(
                        div()
                            .flex_none()
                            .debug_selector(|| format!("status-button:{label}"))
                            .child(Button::new("status-connect").xsmall().label(label).on_click(cx.listener(move |_, _, _, cx| cx.emit(Connect(id))))),
                    )
                }),
            None if self.has_tab => {
                let text = if self.demo { "Sample data · not from a connection" } else { "No connection" };
                h_flex().debug_selector(|| format!("status-origin:{text}")).child(text)
            }
            None => h_flex(),
        };
        h_flex()
            .flex_none()
            .h(theme::status_bar())
            .px(theme::space_4())
            .gap(theme::space_12())
            .relative()
            .bg(theme.status_bar)
            .when(self.origin.as_ref().is_some_and(|o| o.production), |d| {
                d.child(div().absolute().inset_0().bg(theme::production_tint(theme)))
            })
            .border_t_1()
            .border_color(theme.status_bar_border)
            .text_size(theme::ui_sm())
            .text_color(theme.muted_foreground)
            .child(
                h_flex()
                    .child(toggle("toggle-explorer", IconName::PanelLeft, "Toggle explorer", || Box::new(ToggleExplorer)))
                    .child(toggle("toggle-bottom", IconName::PanelBottom, "Toggle bottom panel", || Box::new(ToggleBottomPanel))),
            )
            .child(context)
            // ponytail: tx state is fixed until QX-012/025 give it a model; shown only while Connected
            .when(connected, |d| d.child(div().flex_none().debug_selector(|| "auto-commit".into()).child("AUTO-COMMIT")))
            .child(div().flex_1())
            .children(self.aggregate_text())
            .children(self.cursor.map(|(line, col, selected)| match selected {
                0 => format!("Ln {line}, Col {col}"),
                n => format!("Ln {line}, Col {col} ({n} selected)"),
            }))
            .children(self.memory_mb.map(|mb| {
                div()
                    .id("memory")
                    .font_family(theme.mono_font_family.clone())
                    .pr(theme::space_4())
                    .tooltip(|window, cx| Tooltip::new("QueryX private memory (RssAnon)").build(window, cx))
                    .child(format!("{mb} MB"))
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::aggregate;
    use crate::model::Value::{self, *};

    #[test]
    fn single_cell_or_no_selection_is_hidden() {
        // (rows, cols, values, expected shown)
        let cases: [(usize, usize, Vec<Value>, bool); 5] = [
            (1, 1, vec![Int(3)], false),
            (1, 1, vec![Null], false),
            (0, 0, vec![], false), // no grid focused
            (2, 1, vec![Int(3), Int(4)], true),
            (1, 2, vec![Null, Null], true),
        ];
        for (rows, cols, values, shown) in cases {
            assert_eq!(aggregate(rows, cols, values.clone()).is_some(), shown, "{rows}x{cols} {values:?}");
        }
    }

    /// Runs `(rows, cols, values, expected)` cases through `aggregate`.
    fn check(cases: impl IntoIterator<Item = (usize, usize, Vec<Value>, &'static str)>) {
        for (rows, cols, values, expected) in cases {
            assert_eq!(aggregate(rows, cols, values.clone()).as_deref(), Some(expected), "{rows}x{cols} {values:?}");
        }
    }

    #[test]
    fn int_selection_shows_shape_count_sum_avg() {
        check([
            (2, 1, vec![Int(3), Int(4)], "2 rows × 1 col · count 2 · sum 7 · avg 3.5"),
            (1, 2, vec![Int(3), Int(4)], "1 row × 2 cols · count 2 · sum 7 · avg 3.5"),
            (2, 2, vec![Int(-1), Int(-2), Int(0), Int(7)], "2 rows × 2 cols · count 4 · sum 4 · avg 1"),
        ]);
    }

    #[test]
    fn nulls_are_not_counted_and_non_numeric_cells_have_no_sum() {
        let text = |s: &str| Text(s.into());
        check([
            (3, 1, vec![Int(3), Null, Int(4)], "3 rows × 1 col · count 2 · sum 7 · avg 3.5"),
            (2, 1, vec![text("a"), text("b")], "2 rows × 1 col · count 2"),
            (2, 1, vec![Null, Null], "2 rows × 1 col · count 0"),
            // Non-numeric cells count, but sum and avg cover the numeric ones only.
            (3, 1, vec![text("a"), Int(2), Int(4)], "3 rows × 1 col · count 3 · sum 6 · avg 3"),
            (2, 1, vec![Bool(true), Bool(false)], "2 rows × 1 col · count 2"),
            (1, 3, vec![Null, Int(5), Null], "1 row × 3 cols · count 1 · sum 5 · avg 5"),
        ]);
    }

    #[test]
    fn floats_show_at_most_four_decimals_trimmed() {
        check([
            (2, 1, vec![Int(1), Float(0.5)], "2 rows × 1 col · count 2 · sum 1.5 · avg 0.75"),
            (2, 1, vec![Float(2.0), Float(2.0)], "2 rows × 1 col · count 2 · sum 4 · avg 2"),
            (3, 1, vec![Int(1), Int(0), Int(0)], "3 rows × 1 col · count 3 · sum 1 · avg 0.3333"),
            (3, 1, vec![Int(2), Int(0), Int(0)], "3 rows × 1 col · count 3 · sum 2 · avg 0.6667"),
            (2, 1, vec![Float(0.12345), Float(0.0)], "2 rows × 1 col · count 2 · sum 0.1235 · avg 0.0617"),
            // Rounds to zero: no "-0", no "0.0000".
            (2, 1, vec![Float(-0.00001), Float(0.0)], "2 rows × 1 col · count 2 · sum 0 · avg 0"),
            (2, 1, vec![Float(f64::INFINITY), Int(1)], "2 rows × 1 col · count 2 · sum inf · avg inf"),
            (3, 1, vec![Float(1.5), Null, Text("x".into())], "3 rows × 1 col · count 2 · sum 1.5 · avg 1.5"),
        ]);
    }

    #[test]
    fn over_a_million_cells_shows_shape_only_without_reading_values() {
        // (rows, cols, expected); no value may be read.
        let skipped = [
            (1_000_001, 1, "1000001 rows × 1 col · aggregate skipped (too many cells)"),
            (1, 1_000_001, "1 row × 1000001 cols · aggregate skipped (too many cells)"),
            (1001, 1000, "1001 rows × 1000 cols · aggregate skipped (too many cells)"),
        ];
        for (rows, cols, expected) in skipped {
            let reads = std::cell::Cell::new(0);
            let values = std::iter::repeat_n(Int(1), rows * cols).inspect(|_| reads.set(reads.get() + 1));
            assert_eq!(aggregate(rows, cols, values).as_deref(), Some(expected));
            assert_eq!(reads.get(), 0, "{rows}x{cols} read values");
        }
        let at_limit = aggregate(1_000_000, 1, std::iter::repeat_n(Int(1), 1_000_000));
        assert_eq!(at_limit.as_deref(), Some("1000000 rows × 1 col · count 1000000 · sum 1000000 · avg 1"));
    }

    #[test]
    fn int_sums_are_exact_past_i64() {
        check([
            (2, 1, vec![Int(i64::MAX), Int(1)], "2 rows × 1 col · count 2 · sum 9223372036854775808 · avg 4611686018427387904"),
            (2, 1, vec![Int(i64::MIN), Int(-1)], "2 rows × 1 col · count 2 · sum -9223372036854775809 · avg -4611686018427387904"),
            // Overflow in the middle cancels out; avg is f64, so only the sum is exact.
            (3, 1, vec![Int(i64::MAX), Int(i64::MAX), Int(i64::MIN)], "3 rows × 1 col · count 3 · sum 9223372036854775806 · avg 3074457345618258432"),
        ]);
    }

    #[test]
    fn reads_private_memory_from_proc_status() {
        let status = "Name:\tqueryx\nVmRSS:\t   92160 kB\nRssAnon:\t   35840 kB\nRssFile:\t   56320 kB\n";
        assert_eq!(super::rss_anon_mb(status), Some(35));
        assert_eq!(super::rss_anon_mb("Name:\tqueryx\n"), None);
    }

    /// A bar `width` px wide over a workspace with Postgres connection 1 (db.acme.internal:5433)
    /// and SQLite connection 2 (/x/local.db), started with or without `demo`.
    fn open_bar(cx: &mut gpui_kit::TestAppContext, width: f32, demo: bool) -> (gpui_kit::VisualTestContext, gpui_kit::Entity<super::StatusBar>, gpui_kit::Entity<crate::model::Workspace>) {
        use crate::model::{ConnectionParams, SslMode, Workspace};
        use gpui_kit::{AppContext as _, Bounds, WindowBounds, WindowOptions, point, px, size};
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let mut ws = Workspace { id: 1, name: "w".into(), connections: Vec::new() };
        let pg = ConnectionParams::Postgres { host: "db.acme.internal".into(), port: 5433, user: "app".into(), password: None, database: "app".into(), ssl: SslMode::Prefer };
        ws.add("app-pg".into(), 0, false, pg);
        ws.add("local.db".into(), 4, false, ConnectionParams::Sqlite { path: "/x/local.db".into() });
        let workspace = cx.new(|_| ws);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(width), px(100.))))),
            ..Default::default()
        };
        let (window, bar) = cx.update(|cx| {
            let workspace = workspace.clone();
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| super::StatusBar::new(workspace, demo, window, cx))).expect("open test window")
        });
        (gpui_kit::VisualTestContext::from_window(window.into(), cx), bar, workspace)
    }

    /// Points `bar` at connection `ix` of `workspace` with `status`, as the Shell does for the active tab.
    fn show(cx: &mut gpui_kit::VisualTestContext, bar: &gpui_kit::Entity<super::StatusBar>, workspace: &gpui_kit::Entity<crate::model::Workspace>, ix: usize, status: crate::model::ConnectionStatus) {
        let origin = workspace.update(cx, |ws, cx| {
            ws.connections[ix].status = status;
            cx.notify();
            crate::tabs::Origin::of(&ws.connections[ix])
        });
        bar.update(cx, |bar, cx| bar.set_origin(Some(origin), true, cx));
        cx.run_until_parked();
    }

    fn drawn(cx: &mut gpui_kit::VisualTestContext, selector: String) -> bool {
        cx.debug_bounds(selector.leak()).is_some()
    }

    /// Hovers the element drawn as `selector` past the tooltip delay and says whether `tip` came up.
    fn hover_shows(cx: &mut gpui_kit::VisualTestContext, selector: &str, tip: String) -> bool {
        use gpui_kit::{Modifiers, point, px};
        cx.simulate_mouse_move(point(px(1.), px(1.)), None, Modifiers::none());
        cx.run_until_parked();
        let target = cx.debug_bounds(selector.to_string().leak()).unwrap_or_else(|| panic!("{selector} drawn"));
        cx.simulate_mouse_move(target.center(), None, Modifiers::none());
        cx.executor().advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        drawn(cx, tip)
    }

    #[gpui_kit::test]
    fn the_status_bar_shows_the_active_connections_status_by_dot_text_button_hint_and_auto_commit(cx: &mut gpui_kit::TestAppContext) {
        use crate::model::ConnectionStatus::{self, *};
        let pg_hint = "password authentication failed for user \"app\"\nCheck that db.acme.internal:5433 is reachable (VPN, firewall), then retry.";
        // (connection, status, solid dot, dot tooltip, text, in danger colour, button, hover hint, AUTO-COMMIT)
        let cases: [(usize, ConnectionStatus, bool, &str, Option<&str>, bool, Option<&str>, Option<&str>, bool); 9] = [
            (0, Connected, true, "Connected", None, false, None, None, true),
            (0, Connecting, false, "Connecting…", Some("Connecting…"), false, None, None, false),
            (0, Disconnected, false, "Disconnected", Some("Not connected"), false, Some("Connect"), None, false),
            (0, Error("password authentication failed for user \"app\"".into()), false, "Could not connect: password authentication failed for user \"app\"", Some("Could not connect"), true, Some("Retry"), Some(pg_hint), false),
            (1, Error("No file at /x/local.db".into()), false, "Could not connect: No file at /x/local.db", Some("Could not connect"), true, Some("Retry"), Some("No file at /x/local.db\nCheck that the file exists and is readable."), false),
            (1, Connected, true, "Connected", None, false, None, None, true),
            (1, Connecting, false, "Connecting…", Some("Connecting…"), false, None, None, false),
            (1, Disconnected, false, "Disconnected", Some("Not connected"), false, Some("Connect"), None, false),
            // Our own timeout reason leads the hint the same way.
            (0, Error("timeout expired after 10 s".into()), false, "Could not connect: timeout expired after 10 s", Some("Could not connect"), true, Some("Retry"), Some("timeout expired after 10 s\nCheck that db.acme.internal:5433 is reachable (VPN, firewall), then retry."), false),
        ];
        for (ix, status, solid, tip, text, danger, button, hint, auto_commit) in cases {
            let (mut cx, bar, workspace) = open_bar(cx, 1200., false);
            let case = format!("{ix} {status:?}");
            show(&mut cx, &bar, &workspace, ix, status);
            assert!(drawn(&mut cx, format!("status-dot-{}", if solid { "solid" } else { "ring" })), "{case}: dot shape");
            for known in ["Connecting…", "Not connected", "Could not connect"] {
                let shown = text == Some(known);
                assert_eq!(drawn(&mut cx, format!("status-text:{known}")), shown && !danger, "{case}: text {known:?}");
                assert_eq!(drawn(&mut cx, format!("status-text-danger:{known}")), shown && danger, "{case}: text {known:?} in danger colour");
            }
            for known in ["Connect", "Retry"] {
                assert_eq!(drawn(&mut cx, format!("status-button:{known}")), button == Some(known), "{case}: button {known:?}");
            }
            assert_eq!(drawn(&mut cx, "auto-commit".into()), auto_commit, "{case}: AUTO-COMMIT");
            let dot = if solid { "status-dot-solid" } else { "status-dot-ring" };
            assert!(hover_shows(&mut cx, dot, format!("status-tip:{tip}")), "{case}: dot tooltip {tip:?}");
            if let (Some(hint), Some(text)) = (hint, text) {
                assert!(hover_shows(&mut cx, &format!("status-text-danger:{text}"), format!("status-hint:{hint}")), "{case}: hint {hint:?}");
            }
        }
    }

    #[gpui_kit::test]
    fn a_tab_without_a_connection_shows_no_connection_or_in_demo_mode_sample_data(cx: &mut gpui_kit::TestAppContext) {
        const SAMPLE: &str = "Sample data · not from a connection";
        // (demo, a tab is open, text shown)
        let cases = [(false, true, Some("No connection")), (true, true, Some(SAMPLE)), (false, false, None), (true, false, None)];
        for (demo, has_tab, text) in cases {
            let (mut cx, bar, _) = open_bar(cx, 1200., demo);
            bar.update(&mut cx, |bar, cx| bar.set_origin(None, has_tab, cx));
            cx.run_until_parked();
            let case = format!("demo {demo}, tab {has_tab}");
            for known in ["No connection", SAMPLE] {
                assert_eq!(drawn(&mut cx, format!("status-origin:{known}")), text == Some(known), "{case}: {known:?}");
            }
            for status in ["status-dot-solid", "status-dot-ring", "auto-commit", "status-button:Connect", "status-button:Retry"] {
                assert!(!drawn(&mut cx, status.into()), "{case}: no {status}");
            }
        }
    }

    #[gpui_kit::test]
    fn truncated_status_path_shows_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Bounds, Entity, WindowBounds, WindowOptions, point, px, size};
        use crate::{tabs::Origin, theme::tests::hover_shows_truncation_tip};
        cx.update(gpui_kit::init);
        let long = "prod-pg › analytics_warehouse_replica_eu_west › shipment_tracking_archive_partitioned_2026";
        // (path, tooltip shown)
        for (path, tip) in [("prod-pg › app_db › public", false), (long, true)] {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(420.), px(100.))))),
                ..Default::default()
            };
            let (window, bar): (_, Entity<super::StatusBar>) =
                cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| super::StatusBar::new(cx.new(|_| crate::mock::workspaces().remove(0)), true, window, cx))).expect("open test window"));
            let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
            bar.update(&mut cx, |bar, cx| bar.set_origin(Some(Origin { connection: crate::model::ConnectionId(0), color: 0, production: false, path: path.into(), tail: "".into() }), true, cx));
            cx.run_until_parked();
            assert_eq!(hover_shows_truncation_tip(&mut cx, "status-path".into()), tip, "{path}");
        }
    }
}
