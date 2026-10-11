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
use crate::model::Value;
use crate::tabs::Origin;
use crate::{ToggleBottomPanel, ToggleExplorer, theme};

/// Larger selections show their shape only, so the bar never walks millions of cells.
const AGGREGATE_MAX: usize = 1_000_000;
const MEMORY_INTERVAL: Duration = Duration::from_secs(5);

pub struct StatusBar {
    /// Where the active tab's data comes from; `None` for sample data or no tab.
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
}

impl StatusBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
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
        let context = match &self.origin {
            Some(o) => h_flex()
                .min_w_0()
                .gap(theme::space_6())
                .child(div().size(theme::space_6()).rounded_full().bg(theme::connection_color(o.color, theme)))
                .child(
                    div()
                        .debug_selector(|| "status-path".into())
                        .min_w_0()
                        .truncate()
                        .text_color(theme.foreground)
                        .font_family(theme.mono_font_family.clone())
                        .child(theme::truncating("status-path", StyledText::new(o.path.clone()), o.path.clone())),
                )
                .when(o.production, |d| d.child(theme::prod_badge(theme))),
            None if self.has_tab => h_flex().child("Sample data · not from a connection"),
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
            // ponytail: tx state is fixed until QX-012/025 give it a model; hidden without a connection
            .when(self.origin.is_some(), |d| d.child("AUTO-COMMIT"))
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
                cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| super::StatusBar::new(window, cx))).expect("open test window"));
            let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
            bar.update(&mut cx, |bar, cx| bar.set_origin(Some(Origin { connection: crate::model::ConnectionId(0), color: 0, production: false, path: path.into() }), true, cx));
            cx.run_until_parked();
            assert_eq!(hover_shows_truncation_tip(&mut cx, "status-path".into()), tip, "{path}");
        }
    }
}
