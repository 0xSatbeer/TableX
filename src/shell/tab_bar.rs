use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    popover::Popover,
    v_flex,
};
use gpui_kit::*;

use crate::theme;
use crate::ui::tabs;

use super::{OVERFLOW_CONTEXT, Shell, TabContent, tab::TabDrag};

impl Shell {
    /// Points the status bar at the active tab: its origin, and the cursor only for the console.
    pub(super) fn tab_changed(&mut self, cx: &mut Context<Self>) {
        let console = matches!(
            self.tabs.active().map(|t| &t.content),
            Some(TabContent::Console)
        );
        let editor = console.then_some(&self.editor);
        let grid = match self.tabs.active().map(|t| &t.content) {
            Some(TabContent::Grid(grid)) => Some(grid.clone()),
            _ => None,
        };
        let (origin, has_tab) = (
            self.tabs.active().and_then(|t| t.origin.clone()),
            self.tabs.active().is_some(),
        );
        self.status_bar.update(cx, |bar, cx| {
            bar.set_editor(editor, cx);
            bar.set_grid(grid.as_ref(), cx);
            bar.set_origin(origin, has_tab, cx);
        });
        self.focus_content = true;
        // Without a search field the list is a plain menu; don't re-mount open with a stale query.
        self.overflow_open &= tabs::show_search(self.tabs.tabs.len());
        cx.notify();
    }

    /// Our own strip, not the library `TabBar`: its `Tab` repaints background and borders over
    /// ours, which hid the connection stripe and the production tint.
    pub(super) fn tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<_> = self
            .tabs
            .tabs
            .iter()
            .enumerate()
            .map(|(ix, t)| self.tab(ix, t, cx))
            .collect();
        let theme = cx.theme();
        let shell = cx.entity().downgrade();
        let titles: Vec<SharedString> = self.tabs.tabs.iter().map(|t| t.title.clone()).collect();
        let active_ix = self.tabs.active;
        h_flex()
            .flex_none()
            .h(theme::tab())
            .bg(theme.tab_bar)
            // The hairline under the strip is drawn per piece, so the active tab can leave a gap
            // and join the content below it.
            .child(
                h_flex()
                    .id("editor-tabs")
                    .flex_1()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs)
                    .child(
                        div()
                            .id("tabs-end")
                            .flex_1()
                            .h_full()
                            .border_b_1()
                            .border_color(theme.border)
                            .drag_over::<TabDrag>(|d, _, _, cx| {
                                d.border_l_2().border_color(cx.theme().primary)
                            })
                            .on_drop(cx.listener(|this, drag: &TabDrag, _, cx| {
                                this.tabs.move_tab(drag.ix, this.tabs.tabs.len());
                                this.tab_changed(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .h_full()
                    .px(theme::space_4())
                    .border_l_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(if tabs::show_search(titles.len()) {
                        self.overflow_search_list(cx).into_any_element()
                    } else {
                        Button::new("all-tabs")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronDown)
                            .tooltip("All open tabs")
                            .dropdown_menu(move |mut menu, _, _| {
                                menu = menu.scrollable(true);
                                for (ix, title) in titles.iter().enumerate() {
                                    let shell = shell.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(title.clone())
                                            .checked(ix == active_ix)
                                            .on_click(move |_, _, cx| {
                                                _ = shell.update(cx, |this, cx| {
                                                    this.tabs.activate(ix);
                                                    this.tab_changed(cx);
                                                });
                                            }),
                                    );
                                }
                                menu
                            })
                            .anchor(Anchor::TopRight)
                            .into_any_element()
                    }),
            )
    }

    /// The overflow list with a search field, for more than `tabs::SEARCH_MIN_TABS` tabs.
    pub(super) fn overflow_search_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let search = self.overflow_search.clone();
        let query = search.read(cx).value();
        let empty = tabs::empty_state(&self.tabs.tabs, &query);
        let rows: Vec<_> = tabs::match_tabs(&self.tabs.tabs, &query)
            .into_iter()
            .map(|ix| (ix, self.tabs.tabs[ix].title.clone()))
            .collect();
        let active_ix = self.tabs.active;
        Popover::new("all-tabs-popover")
            .anchor(Anchor::TopRight)
            .open(self.overflow_open)
            .on_open_change({
                let (shell, search) = (shell.downgrade(), search.clone());
                move |open, window, cx| {
                    _ = shell.update(cx, |this, cx| {
                        this.overflow_open = *open;
                        cx.notify();
                    });
                    if *open {
                        search.update(cx, |s, cx| {
                            s.set_value("", window, cx);
                            s.focus(window, cx);
                        });
                    }
                }
            })
            .trigger(
                Button::new("all-tabs")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .tooltip("All open tabs"),
            )
            .content(move |_, _, cx| {
                let list = if let Some(label) = empty {
                    div()
                        .p(theme::space_8())
                        .text_color(cx.theme().muted_foreground)
                        .child(label)
                        .into_any_element()
                } else {
                    v_flex()
                        .id("overflow-list")
                        .max_h(theme::space_24() * 10.)
                        .overflow_y_scroll()
                        .children(rows.iter().map(|(ix, title)| {
                            let (ix, shell) = (*ix, shell.clone());
                            Button::new(("overflow-tab", ix))
                                .ghost()
                                .small()
                                .label(title.clone())
                                .selected(ix == active_ix)
                                .on_click(move |_, _, cx| {
                                    shell.update(cx, |this, cx| {
                                        this.overflow_open = false;
                                        this.tabs.activate(ix);
                                        this.tab_changed(cx);
                                    });
                                })
                        }))
                        .into_any_element()
                };
                v_flex()
                    .key_context(OVERFLOW_CONTEXT)
                    .w(theme::space_24() * 10.)
                    .gap(theme::space_4())
                    .child(Input::new(&search).small())
                    .child(list)
            })
    }
}
