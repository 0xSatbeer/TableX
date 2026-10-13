use std::collections::BTreeSet;
use std::time::{Duration, UNIX_EPOCH};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{QueryLogEntry, QuerySource, QueryStatus};
use crate::theme;

use super::sql::sql_line;
use super::{BottomPanel, TIME_W};

impl BottomPanel {
    pub(super) fn log_row(&self, ix: usize, cx: &Context<Self>) -> impl IntoElement {
        let on_click = cx.listener(move |this, _, _, cx| {
            toggle_expanded(&mut this.expanded, ix);
            // Newest first: list position is the mirror of the filtered position.
            if let Some(pos) = this.rows.iter().rev().position(|&r| r == ix) {
                this.list.splice(pos..pos + 1, 1);
            }
            cx.notify();
        });
        let theme = cx.theme();
        let entry = &self.log[ix];
        let (conn, color, production) = self.connection(entry.connection_id, cx);
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
        let col = |w: f32| div().flex_none().w(theme::space_8() * w).truncate();
        let sql = entry.sql.clone();
        let line = h_flex()
            .group("log-row")
            .w_full()
            .when(production, |d| d.bg(theme::production_tint(theme)))
            .h(theme::list_row())
            .px(theme::space_12())
            .gap(theme::space_12())
            .text_size(theme::ui_sm())
            .text_color(theme.muted_foreground)
            .child(
                col(TIME_W)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme::mono())
                    .child(time_of_day(entry)),
            )
            .child(
                col(14.).child(
                    h_flex()
                        .gap(theme::space_6())
                        .child(
                            div()
                                .flex_none()
                                .size(theme::space_6())
                                .rounded_full()
                                .bg(theme::connection_color(color, theme)),
                        )
                        .child(div().truncate().child(conn))
                        .when(production, |d| d.child(theme::prod_badge(theme))),
                ),
            )
            .child(col(8.).child(entry.database.clone()))
            .child(
                div()
                    .debug_selector(|| format!("log-sql-{ix}"))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme::mono())
                    .text_color(theme.foreground)
                    .child({
                        let line = SharedString::from(entry.sql.replace('\n', " "));
                        theme::truncating(("log-sql", ix), sql_line(&line, theme), line)
                    }),
            )
            // Status next to the numbers it qualifies; machine facts in mono, words in Inter.
            .child(Icon::new(icon).small().text_color(icon_color))
            .child(
                col(8.)
                    .text_right()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme::mono())
                    .child(format_duration(entry.duration)),
            )
            .child(
                col(9.)
                    .text_right()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme::mono())
                    .child(entry.rows.map(|r| format!("{r} rows")).unwrap_or_default()),
            )
            .child(col(11.).child(source))
            .child(
                div()
                    .opacity(0.)
                    .group_hover("log-row", |s| s.opacity(1.))
                    .child(
                        Button::new(("copy-sql", ix))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .tooltip("Copy SQL")
                            .on_click(move |_, _, cx| {
                                // Copying must not bubble to the row and toggle its expansion.
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()));
                            }),
                    ),
            );
        let line = div()
            .id(("log-row", ix))
            .w_full()
            .cursor_pointer()
            .on_click(on_click)
            .child(line);
        let mut row = v_flex().w_full().child(line);
        if self.expanded.contains(&ix) {
            let detail = row_detail(entry);
            row = row.child(
                v_flex()
                    .w_full()
                    .px(theme::space_12())
                    .py(theme::space_8())
                    .gap(theme::space_8())
                    .bg(theme.muted)
                    .border_b_1()
                    .border_color(theme.border)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme::mono())
                    .child(
                        div()
                            .w_full()
                            .text_color(theme.foreground)
                            .whitespace_normal()
                            .child(detail.sql),
                    )
                    .children(detail.error.map(|e| {
                        div()
                            .w_full()
                            .text_color(theme.danger)
                            .whitespace_normal()
                            .child(e)
                    })),
            );
        }
        row
    }
}

/// What an expanded row shows beyond its single line.
#[derive(Debug, PartialEq)]
pub(super) struct RowDetail {
    /// Full statement, newlines kept (rendered wrapped, mono).
    pub(super) sql: String,
    /// Error message of a failed entry.
    pub(super) error: Option<String>,
}

pub(super) fn row_detail(entry: &QueryLogEntry) -> RowDetail {
    RowDetail {
        sql: entry.sql.clone(),
        error: match &entry.status {
            QueryStatus::Err(message) => Some(message.clone()),
            _ => None,
        },
    }
}

/// Click on a row: expand it, or collapse it when already open. Keyed by log index, so an
/// expanded row stays expanded while filters hide and show it.
pub(super) fn toggle_expanded(expanded: &mut BTreeSet<usize>, ix: usize) {
    if !expanded.remove(&ix) {
        expanded.insert(ix);
    }
}

/// Logs column headers in row order (the status icon column has none).
pub(super) fn header_labels() -> [&'static str; 8] {
    [
        "Time (UTC)",
        "Connection",
        "Database",
        "SQL",
        "",
        "Duration",
        "Rows",
        "Source",
    ]
}

fn time_of_day(entry: &QueryLogEntry) -> String {
    let secs = entry
        .at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

pub(super) fn format_duration(d: Duration) -> String {
    match d.as_micros() {
        us if us < 1_000 => format!("{:.1} ms", us as f64 / 1000.),
        us if us < 1_000_000 => format!("{} ms", us / 1000),
        us => format!("{:.1} s", us as f64 / 1_000_000.),
    }
}

/// Column headers aligned with `log_row`'s columns.
pub(super) fn log_header(theme: &gpui_kit::component::Theme) -> impl IntoElement {
    let [time, conn, db, sql, _, duration, rows, source] = header_labels();
    let col = |w: f32, label: &'static str| {
        div()
            .flex_none()
            .w(theme::space_8() * w)
            .truncate()
            .child(label)
    };
    h_flex()
        .flex_none()
        .w_full()
        .h(theme::list_row())
        .px(theme::space_12())
        .gap(theme::space_12())
        .border_b_1()
        .border_color(theme.border)
        .text_size(theme::ui_sm())
        .text_color(theme.muted_foreground)
        .child(col(TIME_W, time))
        .child(col(14., conn))
        .child(col(8., db))
        .child(div().flex_1().min_w_0().child(sql))
        .child(div().flex_none().w(theme::space_16()))
        .child(col(8., duration).text_right())
        .child(col(9., rows).text_right())
        .child(col(11., source))
        // Room for the hover copy button.
        .child(div().flex_none().w(theme::space_24()))
}
