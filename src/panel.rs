//! Bottom panel (QX-014): Results / Logs / Messages.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, UNIX_EPOCH};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    v_flex, h_flex,
};
use gpui_kit::*;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::prelude::FluentBuilder as _;

use crate::model::{QueryLogEntry, QuerySource, QueryStatus, Workspace};
use crate::{ToggleBottomPanel, mock, theme};

const TABS: [&str; 3] = ["Results", "Logs", "Messages"];
const LOGS: usize = 1;

pub struct BottomPanel {
    tab: usize,
    log: Vec<QueryLogEntry>,
    filter: LogFilter,
    search: Entity<InputState>,
    /// Indexes into `log` that pass the filters.
    rows: Vec<usize>,
    /// Connection id to (name, color index, production).
    connections: BTreeMap<u32, (String, u8, bool)>,
}

impl BottomPanel {
    pub fn new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter SQL…"));
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.refilter(cx);
                cx.notify();
            }
        })
        .detach();
        let log = mock::mock_query_log(10_000);
        let filter = LogFilter::default();
        let rows = filter_rows(&log, &filter);
        Self {
            tab: LOGS,
            // ponytail: plain Vec; the capped ring buffer comes with live logging (QX-025)
            log,
            filter,
            search,
            rows,
            connections: workspace.connections.iter().map(|c| (c.id.0, (c.name.clone(), c.color, c.is_production))).collect(),
        }
    }

    fn refilter(&mut self, cx: &App) {
        self.filter.text = self.search.read(cx).value().to_string();
        self.rows = filter_rows(&self.log, &self.filter);
    }

    fn log_row(&self, ix: usize, cx: &App) -> Div {
        let theme = cx.theme();
        let entry = &self.log[ix];
        let (conn, color, production) = self.connections.get(&entry.connection_id.0).cloned().unwrap_or_default();
        let (icon, icon_color) = match entry.status {
            QueryStatus::Ok => (IconName::CircleCheck, theme.success),
            QueryStatus::Err(_) => (IconName::CircleX, theme.danger),
            QueryStatus::Cancelled => (IconName::Ban, theme.muted_foreground),
        };
        let source = match entry.source {
            QuerySource::Console => "Console",
            QuerySource::TableView => "Table",
            QuerySource::Introspection => "Introspection",
        };
        let col = |w: f32| div().flex_none().w(theme::SPACE_8 * w).truncate();
        let sql = entry.sql.clone();
        h_flex()
            .group("log-row")
            .w_full()
            .when(production, |d| d.bg(theme::production_tint(theme)))
            .h(theme::LIST_ROW)
            .px(theme::SPACE_12)
            .gap(theme::SPACE_12)
            .text_size(theme::UI_SM)
            .text_color(theme.muted_foreground)
            .child(col(9.).font_family(theme.mono_font_family.clone()).text_size(theme::MONO).child(time_of_day(entry)))
            .child(
                col(14.).child(
                    h_flex()
                        .gap(theme::SPACE_6)
                        .child(div().flex_none().size(theme::SPACE_6).rounded_full().bg(theme::connection_color(color, theme)))
                        .child(div().truncate().child(conn))
                        .when(production, |d| d.child(theme::prod_badge(theme))),
                ),
            )
            .child(col(8.).child(entry.database.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme::MONO)
                    .text_color(theme.foreground)
                    .child(sql_line(&entry.sql, theme)),
            )
            // Status next to the numbers it qualifies; machine facts in mono, words in Inter.
            .child(Icon::new(icon).small().text_color(icon_color))
            .child(col(8.).text_right().font_family(theme.mono_font_family.clone()).text_size(theme::MONO).child(format_duration(entry.duration)))
            .child(col(9.).text_right().font_family(theme.mono_font_family.clone()).text_size(theme::MONO).child(entry.rows.map(|r| format!("{r} rows")).unwrap_or_default()))
            .child(col(11.).child(source))
            .child(
                div().opacity(0.).group_hover("log-row", |s| s.opacity(1.)).child(
                    Button::new(("copy-sql", ix))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Copy)
                        .tooltip("Copy SQL")
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()))),
                ),
            )
    }
}

/// Which statuses a Logs row can have, for the status filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusKind {
    Ok,
    Error,
    Cancelled,
}

impl StatusKind {
    fn of(status: &QueryStatus) -> Self {
        match status {
            QueryStatus::Ok => Self::Ok,
            QueryStatus::Err(_) => Self::Error,
            QueryStatus::Cancelled => Self::Cancelled,
        }
    }
}

/// Logs filters; they combine with AND.
#[derive(Clone, Debug)]
pub struct LogFilter {
    /// Case-insensitive substring of the SQL.
    pub text: String,
    /// Connection ids; empty means all.
    pub connections: BTreeSet<u32>,
    pub statuses: BTreeSet<StatusKind>,
    pub introspection: bool,
}

impl Default for LogFilter {
    fn default() -> Self {
        Self {
            text: String::new(),
            connections: BTreeSet::new(),
            statuses: [StatusKind::Ok, StatusKind::Error, StatusKind::Cancelled].into(),
            introspection: false,
        }
    }
}

/// Indexes of `log` entries that pass `filter`.
fn filter_rows(log: &[QueryLogEntry], filter: &LogFilter) -> Vec<usize> {
    let text = filter.text.to_lowercase();
    (0..log.len())
        .filter(|&ix| filter.statuses.contains(&StatusKind::of(&log[ix].status)))
        .filter(|&ix| filter.introspection || log[ix].source != QuerySource::Introspection)
        .filter(|&ix| filter.connections.is_empty() || filter.connections.contains(&log[ix].connection_id.0))
        .filter(|&ix| text.is_empty() || log[ix].sql.to_lowercase().contains(&text))
        .collect()
}

/// What an expanded row shows beyond its single line.
#[derive(Debug, PartialEq)]
struct RowDetail {
    /// Full statement, newlines kept (rendered wrapped, mono).
    sql: String,
    /// Error message of a failed entry.
    error: Option<String>,
}

fn row_detail(entry: &QueryLogEntry) -> RowDetail {
    RowDetail { sql: entry.sql.clone(), error: None }
}

/// Click on a row: expand it, or collapse it when already open. Keyed by log index, so an
/// expanded row stays expanded while filters hide and show it.
fn toggle_expanded(_expanded: &mut BTreeSet<usize>, _ix: usize) {}

// ponytail: keyword list, not a parser; the log line only needs a hint of structure.
// Swap for the editor's tree-sitter highlighter if rows need full fidelity.
const KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "NULL", "IS", "IN", "LIKE", "AS", "ON", "JOIN", "LEFT", "INNER",
    "GROUP", "ORDER", "BY", "LIMIT", "OFFSET", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE",
    "TABLE", "INDEX", "DROP", "ALTER", "BEGIN", "COMMIT", "ROLLBACK", "ASC", "DESC", "RETURNING",
];

/// Byte ranges of keywords, strings and numbers in `sql`, tagged with their `highlight.syntax` key.
fn sql_spans(sql: &str) -> Vec<(std::ops::Range<usize>, &'static str)> {
    let bytes = sql.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let b = bytes[i];
        if b == b'\'' {
            i = sql[i + 1..].find('\'').map_or(bytes.len(), |end| i + end + 2);
            spans.push((start..i, "string"));
        } else if b.is_ascii_alphanumeric() || b == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &sql[start..i];
            if word.bytes().all(|c| c.is_ascii_digit()) {
                spans.push((start..i, "number"));
            } else if KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(word)) {
                spans.push((start..i, "keyword"));
            }
        } else {
            i += sql[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    spans
}

fn sql_line(sql: &str, theme: &gpui_kit::component::Theme) -> StyledText {
    let line = sql.replace('\n', " ");
    let spans: Vec<_> = sql_spans(&line)
        .into_iter()
        .filter_map(|(range, key)| Some((range, theme.highlight_theme.style(key)?)))
        .collect();
    StyledText::new(line).with_highlights(spans)
}

fn time_of_day(entry: &QueryLogEntry) -> String {
    let secs = entry.at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() % 86_400;
    format!("{:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

fn format_duration(d: Duration) -> String {
    match d.as_micros() {
        us if us < 1_000 => format!("{:.1} ms", us as f64 / 1000.),
        us if us < 1_000_000 => format!("{} ms", us / 1000),
        us => format!("{:.1} s", us as f64 / 1_000_000.),
    }
}

fn filter_toggle(
    id: &'static str,
    label: &'static str,
    on: bool,
    toggle: fn(&mut LogFilter),
    cx: &mut Context<BottomPanel>,
) -> Button {
    // A checkbox glyph, so the off state still reads as a toggle and not a label.
    let check = Icon::new(if on { Lucide::SquareCheck } else { Lucide::Square });
    Button::new(id).ghost().small().icon(check).label(label).selected(on).on_click(cx.listener(move |this, _, _, cx| {
        toggle(&mut this.filter);
        this.refilter(cx);
        cx.notify();
    }))
}

impl Render for BottomPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let errors_on = self.filter.statuses == [StatusKind::Error].into();
        let errors_toggle = filter_toggle("errors-only", "Errors", errors_on, |f| {
            f.statuses = if f.statuses == [StatusKind::Error].into() { LogFilter::default().statuses } else { [StatusKind::Error].into() };
        }, cx);
        let introspection_toggle =
            filter_toggle("show-introspection", "Introspection", self.filter.introspection, |f| f.introspection = !f.introspection, cx);
        let theme = cx.theme();
        // Flat, same height as the editor strip; the active view is underlined, not boxed.
        let header = h_flex()
            .flex_none()
            .h(theme::TAB)
            .pl(theme::SPACE_12)
            .pr(theme::SPACE_8)
            .gap(theme::SPACE_16)
            .border_b_1()
            .border_color(theme.border)
            .children(TABS.iter().enumerate().map(|(ix, label)| {
                let active = ix == self.tab;
                div()
                    .id(("panel-tab", ix))
                    .h_full()
                    .flex()
                    .items_center()
                    .text_size(theme::UI_SM)
                    .border_b_2()
                    .border_color(if active { theme.foreground } else { transparent_black() })
                    .text_color(if active { theme.foreground } else { theme.muted_foreground })
                    .hover(|d| d.text_color(theme.foreground))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = ix;
                        cx.notify();
                    }))
                    .child(*label)
            }))
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap(theme::SPACE_8)
                    .when(self.tab == LOGS, |this| {
                        this.child(div().w(theme::SPACE_24 * 8.).child(Input::new(&self.search).small()))
                            .child(errors_toggle)
                            .child(introspection_toggle)
                    })
                    .child(
                        Button::new("hide-panel")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip_with_action("Hide panel", &ToggleBottomPanel, None)
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleBottomPanel), cx)),
                    ),
            );
        let empty = |text: &'static str| {
            div().size_full().flex().items_center().justify_center().text_color(theme.muted_foreground).child(text)
        };
        let body = match self.tab {
            LOGS => uniform_list(
                "logs",
                self.rows.len(),
                // Newest first: during an incident the last query is the one you want.
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range.map(|ix| this.log_row(this.rows[this.rows.len() - 1 - ix], cx)).collect()
                }),
            )
            .size_full()
            .into_any_element(),
            0 => empty("Console results will show here").into_any_element(),
            _ => empty("No messages").into_any_element(),
        };
        v_flex().size_full().child(header).child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;


    use std::collections::BTreeSet;

    use super::{LogFilter, RowDetail, StatusKind, filter_rows, row_detail, toggle_expanded};
    use crate::model::{ConnectionId, QueryLogEntry, QuerySource, QueryStatus};

    #[test]
    fn status_filter() {
        let log = crate::mock::mock_query_log(500);
        let with = |statuses: &[StatusKind]| {
            let filter = LogFilter { statuses: statuses.iter().copied().collect(), introspection: true, ..LogFilter::default() };
            filter_rows(&log, &filter)
        };
        let kind = |ix: usize| StatusKind::of(&log[ix].status);
        // (statuses, expected kinds that may appear)
        let cases: [(&[StatusKind], &[StatusKind]); 5] = [
            (&[StatusKind::Error], &[StatusKind::Error]),
            (&[StatusKind::Ok], &[StatusKind::Ok]),
            (&[StatusKind::Cancelled], &[StatusKind::Cancelled]),
            (&[StatusKind::Ok, StatusKind::Cancelled], &[StatusKind::Ok, StatusKind::Cancelled]),
            (&[], &[]),
        ];
        for (statuses, allowed) in cases {
            let rows = with(statuses);
            assert!(rows.iter().all(|&ix| allowed.contains(&kind(ix))), "{statuses:?}");
            assert_eq!(rows.is_empty(), allowed.is_empty() || !log.iter().any(|e| allowed.contains(&StatusKind::of(&e.status))), "{statuses:?}");
        }
        assert!(!with(&[StatusKind::Error]).is_empty(), "mock log has errors");
        assert!(!with(&[StatusKind::Ok]).is_empty(), "mock log has ok rows");
        assert_eq!(with(&[StatusKind::Ok, StatusKind::Error, StatusKind::Cancelled]).len(), log.len());
    }

    fn entry(conn: u32, sql: &str, status: QueryStatus, source: QuerySource) -> QueryLogEntry {
        QueryLogEntry {
            at: std::time::UNIX_EPOCH,
            connection_id: ConnectionId(conn),
            database: "app_db".into(),
            sql: sql.into(),
            duration: Duration::from_millis(1),
            rows: None,
            status,
            source,
        }
    }

    /// 0 c1 SELECT ok | 1 c2 SELECT ok | 2 c1 delete ok | 3 c3 select err | 4 c1 Select cancelled | 5 c2 UPDATE err
    fn sample() -> Vec<QueryLogEntry> {
        let (ok, err, c) = (QueryStatus::Ok, || QueryStatus::Err("boom".into()), QuerySource::Console);
        vec![
            entry(1, "SELECT 1", ok.clone(), c),
            entry(2, "SELECT 2", ok.clone(), c),
            entry(1, "delete from t", ok, c),
            entry(3, "select 3", err(), c),
            entry(1, "Select 4", QueryStatus::Cancelled, c),
            entry(2, "UPDATE t SET a = 1", err(), c),
        ]
    }

    #[test]
    fn connection_and_text_filter() {
        let log = sample();
        let run = |conns: &[u32], text: &str, statuses: Option<&[StatusKind]>| {
            let mut f = LogFilter { text: text.into(), connections: conns.iter().copied().collect(), ..LogFilter::default() };
            if let Some(s) = statuses {
                f.statuses = s.iter().copied().collect();
            }
            filter_rows(&log, &f)
        };
        // (connections, text, statuses, expected rows)
        let cases: [(&[u32], &str, Option<&[StatusKind]>, &[usize]); 9] = [
            (&[], "", None, &[0, 1, 2, 3, 4, 5]),               // empty set = all connections
            (&[1], "select", None, &[0, 4]),                    // spec case, text is case-insensitive
            (&[1], "", None, &[0, 2, 4]),
            (&[1, 2], "", None, &[0, 1, 2, 4, 5]),              // several names
            (&[], "SELECT", None, &[0, 1, 3, 4]),               // upper-case query
            (&[], "select", None, &[0, 1, 3, 4]),
            (&[1, 2], "select", Some(&[StatusKind::Ok]), &[0, 1]), // text + status + connection
            (&[1], "select", Some(&[StatusKind::Cancelled]), &[4]), // cancelled is not ok
            (&[9], "", None, &[]),                              // unknown connection
        ];
        for (conns, text, statuses, expected) in cases {
            assert_eq!(run(conns, text, statuses), expected, "{conns:?} {text:?} {statuses:?}");
        }
    }

    #[test]
    fn introspection_hidden_by_default() {
        let c = QuerySource::Console;
        let i = QuerySource::Introspection;
        let log = vec![
            entry(1, "SELECT 1", QueryStatus::Ok, c),
            entry(1, "SELECT * FROM pg_class", QueryStatus::Ok, i),
            entry(1, "SELECT * FROM pg_class", QueryStatus::Err("x".into()), i),
            entry(1, "SELECT 2", QueryStatus::Ok, QuerySource::TableView),
        ];
        let f = |introspection: bool, text: &str| filter_rows(&log, &LogFilter { introspection, text: text.into(), ..LogFilter::default() });
        assert_eq!(filter_rows(&log, &LogFilter::default()), [0, 3], "default is off");
        assert_eq!(f(false, ""), [0, 3]);
        assert_eq!(f(true, ""), [0, 1, 2, 3]);
        assert_eq!(f(false, "pg_class"), Vec::<usize>::new(), "text can't bring hidden rows back");
        assert_eq!(f(true, "pg_class"), [1, 2]);
    }

    #[test]
    fn expanding_rows() {
        let log = sample();
        // error row: full SQL plus the message
        let multi = entry(1, "SELECT *\nFROM t\nWHERE a = 1", QueryStatus::Err("relation \"t\" does not exist".into()), QuerySource::Console);
        assert_eq!(
            row_detail(&multi),
            RowDetail { sql: "SELECT *\nFROM t\nWHERE a = 1".into(), error: Some("relation \"t\" does not exist".into()) }
        );
        // non-error rows show SQL only
        assert_eq!(row_detail(&log[0]), RowDetail { sql: "SELECT 1".into(), error: None });
        assert_eq!(row_detail(&log[4]).error, None, "cancelled has no error text");

        // click expands, click again collapses; rows are independent
        let mut open = BTreeSet::new();
        toggle_expanded(&mut open, 3);
        assert!(open.contains(&3));
        toggle_expanded(&mut open, 5);
        assert_eq!(open, [3, 5].into());
        toggle_expanded(&mut open, 3);
        assert_eq!(open, [5].into());

        // survives a filter change: hidden while filtered out, open again when it returns
        let only_ok = LogFilter { statuses: [StatusKind::Ok].into(), ..LogFilter::default() };
        assert!(!filter_rows(&log, &only_ok).contains(&5));
        assert!(open.contains(&5));
        assert!(filter_rows(&log, &LogFilter::default()).contains(&5));
    }

    #[test]
    fn sql_spans_tag_keywords_strings_and_numbers() {
        let sql = "select id FROM users WHERE name = 'jörg' LIMIT 50";
        let tagged: Vec<_> = super::sql_spans(sql).into_iter().map(|(r, k)| (&sql[r], k)).collect();
        assert_eq!(
            tagged,
            [("select", "keyword"), ("FROM", "keyword"), ("WHERE", "keyword"), ("'jörg'", "string"), ("LIMIT", "keyword"), ("50", "number")]
        );
    }

    #[test]
    fn durations_pick_a_readable_unit() {
        assert_eq!(super::format_duration(Duration::from_micros(400)), "0.4 ms");
        assert_eq!(super::format_duration(Duration::from_micros(12_345)), "12 ms");
        assert_eq!(super::format_duration(Duration::from_millis(1_250)), "1.2 s");
    }
}
