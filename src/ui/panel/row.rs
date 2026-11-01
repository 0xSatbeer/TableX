use std::time::{Duration, SystemTime};

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{QueryLogEntry, QueryStatus};
use crate::theme;

use super::BottomPanel;
use super::sql::sql_line;

impl BottomPanel {
    /// One entry as console output: `conn.db>` prompt and the full SQL, then a stamped outcome line.
    pub(super) fn log_row(&self, ix: usize, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let entry = &self.log[ix];
        let (conn, color, production) = self.connection(entry.connection_id, cx);
        let sql = entry.sql.clone();
        let outcome_color = match entry.status {
            QueryStatus::Err(_) => theme.danger,
            _ => theme.muted_foreground,
        };
        let statement = h_flex()
            .items_start()
            .w_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap(theme::space_6())
                    .pr(theme::space_8())
                    .italic()
                    .text_color(theme::connection_color(color, theme))
                    .child(format!("{conn}.{}>", entry.database))
                    // Text, not only color, so production reads without color vision.
                    .when(production, |d| d.child(theme::prod_badge(theme))),
            )
            .child(
                v_flex()
                    .debug_selector(|| format!("log-sql-{ix}"))
                    .flex_1()
                    .min_w_0()
                    .text_color(theme.foreground)
                    .children(entry.sql.lines().map(|line| {
                        div().child(sql_line(&SharedString::from(line.to_string()), theme))
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .opacity(0.)
                    .group_hover("log-row", |s| s.opacity(1.))
                    .child(
                        Button::new(("copy-sql", ix))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .tooltip("Copy SQL")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()))
                            }),
                    ),
            );
        v_flex()
            .id(("log-row", ix))
            .group("log-row")
            .w_full()
            .px(theme::space_12())
            .py(theme::space_4())
            .font_family(theme.mono_font_family.clone())
            .text_size(theme::mono())
            .child(statement)
            .child(div().text_color(outcome_color).child(format!(
                "[{}] {}",
                stamp(entry.at),
                outcome(entry)
            )))
    }
}

/// `YYYY-MM-DD HH:MM:SS` in local time.
pub(super) fn stamp(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// The line after the SQL: rows and time, the error, or the cancel.
pub(super) fn outcome(entry: &QueryLogEntry) -> String {
    let took = format_duration(entry.duration);
    match (&entry.status, entry.rows) {
        (QueryStatus::Ok, Some(1)) => format!("1 row in {took}"),
        (QueryStatus::Ok, Some(n)) => format!("{n} rows in {took}"),
        (QueryStatus::Ok, None) => format!("Completed in {took}"),
        (QueryStatus::Err(message), _) => message.clone(),
        (QueryStatus::Cancelled, _) => format!("Cancelled after {took}"),
    }
}

pub(super) fn format_duration(d: Duration) -> String {
    match d.as_micros() {
        us if us < 1_000 => format!("{:.1} ms", us as f64 / 1000.),
        us if us < 1_000_000 => format!("{} ms", us / 1000),
        us => format!("{:.1} s", us as f64 / 1_000_000.),
    }
}
