//! Command palette (Ctrl+Shift+P): every action with its shortcut; typing filters, Enter runs it
//! on whatever had focus before the palette opened.

use std::cmp::Reverse;
use std::time::Duration;

use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, h_flex, kbd::Kbd};
use gpui_kit::*;

use crate::keymap::{Bindings, NAMES};
use crate::theme;

pub struct Command {
    pub action: &'static str,
    pub name: &'static str,
    area: &'static str,
    keys: Option<String>,
}

/// Actions run from the palette, most recent first.
#[derive(Default)]
pub struct Recent(Vec<&'static str>);

impl Global for Recent {}

/// The open palette's list; the shell drops it to close.
pub struct Open {
    pub list: Entity<ListState<Commands>>,
    /// Focused before the palette opened: refocused on close, and where the action goes.
    pub previous: Option<FocusHandle>,
    pub _events: Subscription,
}

pub struct Commands {
    all: Vec<Command>,
    recent: Vec<&'static str>,
    /// Indices into `all` in display order, with the matched byte offsets of each name.
    shown: Vec<(usize, Vec<usize>)>,
    query: String,
}

impl Commands {
    pub fn new(cx: &App) -> Self {
        let bound = cx
            .try_global::<Bindings>()
            .map(|b| b.0.as_slice())
            .unwrap_or_default();
        let all: Vec<Command> = NAMES
            .iter()
            .map(|&(action, name, area)| Command {
                action,
                name,
                area,
                keys: bound
                    .iter()
                    .find(|b| b.action == action)
                    .map(|b| b.keys.clone()),
            })
            .collect();
        let recent = cx
            .try_global::<Recent>()
            .map(|r| r.0.clone())
            .unwrap_or_default();
        let shown = ranked(&all, "", &recent);
        Self {
            all,
            recent,
            shown,
            query: String::new(),
        }
    }

    pub fn action(&self, row: usize) -> Option<&'static str> {
        self.shown.get(row).map(|&(ix, _)| self.all[ix].action)
    }
}

/// Moves `action` to the front of the palette's recent list.
pub fn remember(action: &'static str, cx: &mut App) {
    let recent = &mut cx.default_global::<Recent>().0;
    recent.retain(|a| *a != action);
    recent.insert(0, action);
}

/// Matches of `query` best first; an empty one lists `recent` first, then the rest by name.
fn ranked(all: &[Command], query: &str, recent: &[&str]) -> Vec<(usize, Vec<usize>)> {
    if query.trim().is_empty() {
        let mut order: Vec<usize> = (0..all.len()).collect();
        order.sort_by_key(|&ix| {
            let used = recent.iter().position(|a| *a == all[ix].action);
            (used.unwrap_or(usize::MAX), all[ix].name)
        });
        return order.into_iter().map(|ix| (ix, Vec::new())).collect();
    }
    let mut hits: Vec<(i32, usize, Vec<usize>)> = all
        .iter()
        .enumerate()
        .filter_map(|(ix, c)| crate::fuzzy::score(query, c.name).map(|(s, at)| (s, ix, at)))
        .collect();
    hits.sort_by_key(|(s, ix, _)| (Reverse(*s), all[*ix].name.len(), all[*ix].name));
    hits.into_iter().map(|(_, ix, at)| (ix, at)).collect()
}

impl ListDelegate for Commands {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.to_string();
        self.shown = ranked(&self.all, query, &self.recent);
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
        let (command, at) = self.shown.get(ix.row).map(|(c, at)| (&self.all[*c], at))?;
        let theme = cx.theme();
        let style = HighlightStyle {
            color: Some(theme.primary),
            ..Default::default()
        };
        let highlights: Vec<_> = at
            .iter()
            .map(|&i| {
                (
                    i..i + command.name[i..].chars().next().map_or(1, char::len_utf8),
                    style,
                )
            })
            .collect();
        let keys = command
            .keys
            .iter()
            .flat_map(|k| k.split_whitespace())
            .filter_map(|k| Keystroke::parse(k).ok());
        Some(
            ListItem::new(ix.row).child(
                h_flex()
                    .w_full()
                    .gap(theme::space_8())
                    .child(StyledText::new(command.name).with_highlights(highlights))
                    .child(
                        div()
                            .text_size(theme::ui_sm())
                            .text_color(theme.muted_foreground)
                            .child(command.area),
                    )
                    .child(div().flex_1())
                    .children(keys.map(Kbd::new)),
            ),
        )
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
        let text = format!("No commands match '{}'", self.query);
        h_flex()
            .debug_selector(|| format!("palette-state:{text}"))
            .size_full()
            .justify_center()
            .p(theme::space_8())
            .text_color(cx.theme().muted_foreground)
            .child(text)
    }
}

/// The palette over everything: a backdrop that `on_dismiss`es on click, and the list near the top.
pub fn overlay(
    list: &Entity<ListState<Commands>>,
    on_dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let card = div()
        .w(px(560.))
        .h(px(380.))
        .mt(px(80.))
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .shadow_lg()
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(List::new(list).search_placeholder("Run a command…"));
    let backdrop = div()
        .id("command-palette")
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
            "command-palette-in",
            Animation::new(Duration::from_millis(100)),
            |d, t| d.opacity(t),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests;
