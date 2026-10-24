//! Quick open (Ctrl+P): every table, view, materialized view and function the explorer has
//! loaded on connected connections; typing filters, Enter opens it as a double-click would.

use std::cmp::Reverse;

use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme as _, Icon, IndexPath, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{ConnectionStatus, ObjectKind, Workspace};
use crate::ui::explorer::object_key;
use crate::{assets, theme};

pub struct Found {
    /// The explorer's tree key, for revealing and opening it.
    pub key: String,
    pub kind: ObjectKind,
    pub name: String,
    /// `connection / db / schema`
    pub place: String,
    pub color: u8,
    pub production: bool,
}

/// The open quick-open list; the shell drops it to close.
pub struct Open {
    pub list: Entity<ListState<Objects>>,
    /// Focused before it opened, refocused when it closes without opening anything.
    pub previous: Option<FocusHandle>,
    pub _events: Subscription,
    /// Re-indexes as the explorer's loads arrive.
    pub _loads: Subscription,
}

/// The loaded objects of every connected connection, in tree order.
pub fn index(ws: &Workspace) -> Vec<Found> {
    let mut all = Vec::new();
    let connected = ws
        .connections
        .iter()
        .filter(|c| c.status == ConnectionStatus::Connected);
    for conn in connected {
        for db in conn.databases.iter().flatten() {
            for schema in db.schemas.iter().flatten() {
                let objects = schema.objects.iter().flatten().filter(|o| {
                    matches!(
                        o.kind,
                        ObjectKind::Table
                            | ObjectKind::View
                            | ObjectKind::MaterializedView
                            | ObjectKind::Function
                    )
                });
                all.extend(objects.map(|o| Found {
                    key: object_key(conn.id, &db.name, &schema.name, o.kind, &o.name),
                    kind: o.kind,
                    name: o.name.clone(),
                    place: format!("{} / {} / {}", conn.name, db.name, schema.name),
                    color: conn.color,
                    production: conn.is_production,
                }));
            }
        }
    }
    all
}

/// Matches of `query` best first, shorter names first on a tie; an empty one lists all.
fn ranked(all: &[Found], query: &str) -> Vec<(usize, Vec<usize>)> {
    if query.trim().is_empty() {
        return (0..all.len()).map(|ix| (ix, Vec::new())).collect();
    }
    let mut hits: Vec<(i32, usize, Vec<usize>)> = all
        .iter()
        .enumerate()
        .filter_map(|(ix, f)| crate::fuzzy::score(query, &f.name).map(|(s, at)| (s, ix, at)))
        .collect();
    hits.sort_by(|a, b| {
        let (x, y) = (&all[a.1].name, &all[b.1].name);
        (Reverse(a.0), x.len(), x).cmp(&(Reverse(b.0), y.len(), y))
    });
    hits.into_iter().map(|(_, ix, at)| (ix, at)).collect()
}

pub struct Objects {
    all: Vec<Found>,
    /// Indices into `all` in display order, with the matched byte offsets of each name.
    shown: Vec<(usize, Vec<usize>)>,
    query: String,
    /// Databases and schemas still loading.
    pending: usize,
    /// Any connection connected.
    connected: bool,
}

impl Objects {
    pub fn new(ws: &Workspace, pending: usize) -> Self {
        let mut this = Self {
            all: Vec::new(),
            shown: Vec::new(),
            query: String::new(),
            pending: 0,
            connected: false,
        };
        this.update(ws, pending);
        this
    }

    /// Re-reads the loaded objects, keeping the query.
    pub fn update(&mut self, ws: &Workspace, pending: usize) {
        self.all = index(ws);
        self.shown = ranked(&self.all, &self.query);
        self.pending = pending;
        self.connected = ws
            .connections
            .iter()
            .any(|c| c.status == ConnectionStatus::Connected);
    }

    pub fn key(&self, row: usize) -> Option<String> {
        self.shown.get(row).map(|&(ix, _)| self.all[ix].key.clone())
    }

    fn loading(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        (self.pending > 0).then(|| {
            let text = format!("Loading {} schemas…", self.pending);
            div()
                .debug_selector(|| format!("quick-open-state:{text}"))
                .px(theme::space_8())
                .py(theme::space_6())
                .text_size(theme::ui_sm())
                .text_color(cx.theme().muted_foreground)
                .child(text.clone())
        })
    }
}

impl ListDelegate for Objects {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.to_string();
        self.shown = ranked(&self.all, query);
        Task::ready(())
    }

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.shown.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let (found, at) = self.shown.get(ix.row).map(|(f, at)| (&self.all[*f], at))?;
        let theme = cx.theme();
        let style = HighlightStyle {
            color: Some(theme.primary),
            ..Default::default()
        };
        let highlights: Vec<_> = at
            .iter()
            .map(|&i| {
                (
                    i..i + found.name[i..].chars().next().map_or(1, char::len_utf8),
                    style,
                )
            })
            .collect();
        Some(
            ListItem::new(ix.row).child(
                h_flex()
                    .w_full()
                    .gap(theme::space_6())
                    .child(
                        Icon::empty()
                            .path(assets::object_icon(found.kind.as_str(), false))
                            .text_color(theme::object_color(found.kind, theme)),
                    )
                    .child(
                        StyledText::new(SharedString::from(found.name.clone()))
                            .with_highlights(highlights),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(theme::ui_sm())
                            .text_color(theme.muted_foreground)
                            .child(found.place.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .size(theme::space_6())
                            .rounded_full()
                            .bg(theme::connection_color(found.color, theme)),
                    )
                    .when(found.production, |d| d.child(theme::prod_badge(theme))),
            ),
        )
    }

    fn render_section_footer(
        &mut self,
        _: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        self.loading(cx)
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        let text = if !self.connected {
            "Connect to a database to search its objects".to_string()
        } else if self.query.trim().is_empty() {
            "No objects loaded yet".to_string()
        } else {
            format!("No objects match '{}'", self.query)
        };
        let loading = self.loading(cx);
        gpui_kit::component::v_flex()
            .debug_selector(|| format!("quick-open-state:{text}"))
            .size_full()
            .items_center()
            .justify_center()
            .p(theme::space_8())
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .children(loading)
    }
}

/// Quick open over everything: a backdrop that `on_dismiss`es on click, and the list near the top.
pub fn overlay(
    list: &Entity<ListState<Objects>>,
    on_dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let card = div()
        .w(px(640.))
        .h(px(420.))
        .mt(px(80.))
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .shadow_lg()
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(List::new(list).search_placeholder("Go to a table, view or function…"));
    let backdrop = div()
        .id("quick-open")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .justify_center()
        .items_start()
        .on_mouse_down(MouseButton::Left, on_dismiss)
        .child(card);
    if cx.reduce_motion() {
        return backdrop.into_any_element();
    }
    backdrop
        .with_animation(
            "quick-open-in",
            Animation::new(std::time::Duration::from_millis(100)),
            |d, t| d.opacity(t),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests;
