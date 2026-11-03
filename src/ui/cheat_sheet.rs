//! Keyboard cheat sheet (Ctrl+K Ctrl+S): every action's effective keystrokes, grouped by area;
//! typing filters by name or keystroke.

use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, h_flex};
use gpui_kit::*;

use crate::keymap::{self, Binding, Bindings};
use crate::theme;

/// Section order on the sheet.
const GROUPS: &[&str] = &["Tabs", "Explorer", "Console", "Grid", "Panels", "App"];

/// The palette's area of an action folded into the sheet's coarser groups.
fn group(area: &str) -> usize {
    let name = match area {
        "Connections" => "Explorer",
        "Layout" => "Panels",
        "Tabs" | "Explorer" | "Console" | "Grid" => area,
        _ => "App",
    };
    GROUPS.iter().position(|g| *g == name).expect("known group")
}

pub struct Row {
    name: &'static str,
    group: usize,
    /// Space-separated chords, several bindings joined by ", "; "—" when unbound.
    keys: String,
}

/// One row per named action (demo-only ones only when `demo`), with its keystrokes from `bound`.
pub fn rows(bound: &[Binding], demo: bool) -> Vec<Row> {
    keymap::names(demo)
        .map(|&(action, name, area)| {
            let keys: Vec<_> = bound
                .iter()
                .filter(|b| b.action == action)
                .map(|b| b.keys.as_str())
                .collect();
            Row {
                name,
                group: group(area),
                keys: if keys.is_empty() {
                    "—".into()
                } else {
                    keys.join(", ")
                },
            }
        })
        .collect()
}

/// The open sheet's list; the shell drops it to close.
pub struct Open {
    pub list: Entity<ListState<Sheet>>,
    pub previous: Option<FocusHandle>,
    pub _events: Subscription,
}

pub struct Sheet {
    all: Vec<Row>,
    /// Non-empty groups in order, each with its matching rows (indices into `all`) by name.
    shown: Vec<(usize, Vec<usize>)>,
    query: String,
    /// Where `keymap.toml` lives, for the footer.
    pub path: String,
}

impl Sheet {
    pub fn new(all: Vec<Row>, path: String) -> Self {
        let mut sheet = Self {
            all,
            shown: Vec::new(),
            query: String::new(),
            path,
        };
        sheet.filter("");
        sheet
    }

    /// The bindings in effect, and `keymap.toml` under the resolved config dir.
    pub fn current(demo: bool, cx: &App) -> Self {
        let bound = cx
            .try_global::<Bindings>()
            .map(|b| b.0.as_slice())
            .unwrap_or_default();
        let path = crate::persist::ConfigDir::resolve(|k| std::env::var_os(k)).map_or_else(
            || "~/.config/tablex/keymap.toml".into(),
            |c| c.root.join("keymap.toml").display().to_string(),
        );
        Self::new(rows(bound, demo), path)
    }

    fn filter(&mut self, query: &str) {
        self.query = query.to_string();
        let query = query.trim().to_lowercase();
        let mut order: Vec<usize> = (0..self.all.len())
            .filter(|&ix| {
                let row = &self.all[ix];
                row.name.to_lowercase().contains(&query) || row.keys.contains(&query)
            })
            .collect();
        order.sort_by_key(|&ix| (self.all[ix].group, self.all[ix].name));
        self.shown = order
            .chunk_by(|a, b| self.all[*a].group == self.all[*b].group)
            .map(|ixs| (self.all[ixs[0]].group, ixs.to_vec()))
            .collect();
    }
}

impl ListDelegate for Sheet {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.filter(query);
        Task::ready(())
    }

    fn sections_count(&self, _: &App) -> usize {
        self.shown.len()
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.shown.get(section).map_or(0, |(_, ixs)| ixs.len())
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        let (group, _) = self.shown.get(section)?;
        Some(
            div()
                .px(theme::space_8())
                .pt(theme::space_8())
                .text_size(theme::ui_sm())
                .text_color(cx.theme().muted_foreground)
                .child(GROUPS[*group]),
        )
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let row = &self.all[*self.shown.get(ix.section)?.1.get(ix.row)?];
        let theme = cx.theme();
        Some(
            ListItem::new(ix).child(
                h_flex()
                    .w_full()
                    .gap(theme::space_8())
                    .child(row.name)
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(theme::ui_sm())
                            .text_color(theme.muted_foreground)
                            .child(row.keys.clone()),
                    ),
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
        h_flex()
            .size_full()
            .justify_center()
            .p(theme::space_8())
            .text_color(cx.theme().muted_foreground)
            .child(format!("No shortcuts match '{}'", self.query))
    }
}

/// The sheet over everything: a backdrop that `on_dismiss`es on click, the list and a footer.
pub fn overlay(
    list: &Entity<ListState<Sheet>>,
    on_dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let footer = div()
        .flex_none()
        .px(theme::space_8())
        .py(theme::space_4())
        .border_t_1()
        .border_color(theme.border)
        .text_size(theme::ui_sm())
        .text_color(theme.muted_foreground)
        .child(format!(
            "Edit {} to change shortcuts",
            list.read(cx).delegate().path
        ));
    let card = div()
        .w(px(560.))
        .h(px(480.))
        .mt(px(80.))
        .flex()
        .flex_col()
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .shadow_lg()
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(List::new(list).search_placeholder("Search shortcuts…")),
        )
        .child(footer);
    div()
        .id("cheat-sheet")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .justify_center()
        .items_start()
        .on_mouse_down(MouseButton::Left, on_dismiss)
        .child(card)
        .into_any_element()
}

#[cfg(test)]
mod tests;
