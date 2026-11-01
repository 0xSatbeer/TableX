use std::collections::BTreeSet;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::*;

use crate::model::{QueryLogEntry, QuerySource, QueryStatus};

use super::BottomPanel;

impl BottomPanel {
    /// "All connections" or the chosen ones; each item toggles one connection.
    pub(super) fn connection_menu(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let items: Vec<_> = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .map(|c| {
                (
                    c.id.0,
                    c.name.clone(),
                    self.filter.connections.contains(&c.id.0),
                )
            })
            .collect();
        let label = self.connection_filter_label(cx);
        let panel = cx.entity().downgrade();
        Button::new("log-connections")
            .ghost()
            .small()
            .label(label)
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |mut menu, _, _| {
                for (id, name, on) in items.clone() {
                    let panel = panel.clone();
                    menu = menu.item(PopupMenuItem::new(name).checked(on).on_click(
                        move |_, _, cx| {
                            _ = panel.update(cx, |this, cx| {
                                if !this.filter.connections.remove(&id) {
                                    this.filter.connections.insert(id);
                                }
                                this.refilter(cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            })
    }

    /// Status and introspection filters; each item toggles one, like the connection menu.
    pub(super) fn filter_menu(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let toggle_status: fn(&mut LogFilter, StatusKind) = |f, kind| {
            if !f.statuses.remove(&kind) {
                f.statuses.insert(kind);
            }
        };
        let items: Vec<(&'static str, bool, std::rc::Rc<dyn Fn(&mut LogFilter)>)> = vec![
            (
                "Ok",
                self.filter.statuses.contains(&StatusKind::Ok),
                std::rc::Rc::new(move |f| toggle_status(f, StatusKind::Ok)),
            ),
            (
                "Error",
                self.filter.statuses.contains(&StatusKind::Error),
                std::rc::Rc::new(move |f| toggle_status(f, StatusKind::Error)),
            ),
            (
                "Cancelled",
                self.filter.statuses.contains(&StatusKind::Cancelled),
                std::rc::Rc::new(move |f| toggle_status(f, StatusKind::Cancelled)),
            ),
            (
                "Introspection",
                self.filter.introspection,
                std::rc::Rc::new(|f| f.introspection = !f.introspection),
            ),
        ];
        let panel = cx.entity().downgrade();
        Button::new("log-filters")
            .ghost()
            .xsmall()
            .icon(Icon::new(Lucide::Funnel))
            .tooltip("Filters")
            .dropdown_menu(move |mut menu, _, _| {
                for (ix, (label, on, toggle)) in items.iter().enumerate() {
                    if ix == 3 {
                        menu = menu.separator();
                    }
                    let (panel, toggle) = (panel.clone(), toggle.clone());
                    menu = menu.item(PopupMenuItem::new(*label).checked(*on).on_click(
                        move |_, _, cx| {
                            _ = panel.update(cx, |this, cx| {
                                toggle(&mut this.filter);
                                this.refilter(cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            })
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
    pub(super) fn of(status: &QueryStatus) -> Self {
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
pub(super) fn filter_rows<'a>(
    log: impl IntoIterator<Item = &'a QueryLogEntry>,
    filter: &LogFilter,
) -> Vec<usize> {
    let text = filter.text.to_lowercase();
    log.into_iter()
        .enumerate()
        .filter(|(_, e)| filter.statuses.contains(&StatusKind::of(&e.status)))
        .filter(|(_, e)| filter.introspection || e.source != QuerySource::Introspection)
        .filter(|(_, e)| {
            filter.connections.is_empty() || filter.connections.contains(&e.connection_id.0)
        })
        .filter(|(_, e)| text.is_empty() || e.sql.to_lowercase().contains(&text))
        .map(|(ix, _)| ix)
        .collect()
}

const NO_MATCHES: &str = "No log entries match these filters";
const NO_QUERIES: &str = "Queries you run will show up here";

/// Message shown instead of the log list when there are no rows, and whether the filters hid them.
pub(super) fn empty_message(rows: &[usize], log_len: usize) -> Option<(&'static str, bool)> {
    match (rows.is_empty(), log_len) {
        (false, _) => None,
        (true, 0) => Some((NO_QUERIES, false)),
        (true, _) => Some((NO_MATCHES, true)),
    }
}
