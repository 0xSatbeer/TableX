use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, menu::ContextMenuExt as _,
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::tabs;
use crate::{
    CloseAllTabs, CloseOtherTabs, CloseTab, CloseTabsToRight, CopyTabName, RevealTab, TogglePinTab,
    assets, theme,
};

use super::{Shell, TabContent};

impl Shell {
    pub(super) fn tab(
        &self,
        ix: usize,
        t: &tabs::EditorTab<TabContent>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let icon = match t.content {
            TabContent::Grid(_) => Icon::empty().path(assets::object_icon("table", false)),
            TabContent::Console(_) => Icon::new(IconName::SquareTerminal),
        };
        let active = ix == self.tabs.active;
        let group: SharedString = format!("tab-{ix}").into();
        let pinned = t.pinned;
        let production = t.origin.as_ref().is_some_and(|o| o.production);
        h_flex()
            .id(("tab", ix))
            .group(group.clone())
            .relative()
            .flex_none()
            .h_full()
            .pl(theme::space_12())
            .pr(if pinned {
                theme::space_12()
            } else {
                theme::space_6()
            })
            .gap(theme::space_6())
            .border_r_1()
            .when(!active, |d| d.border_b_1())
            .border_color(theme.border)
            .text_size(theme::ui_sm())
            .map(|d| {
                if active {
                    d.bg(theme.tab_active)
                        .text_color(theme.tab_active_foreground)
                } else {
                    d.text_color(theme.tab_foreground)
                        .hover(|d| d.bg(theme.secondary_hover))
                }
            })
            // Tint and stripe are overlays so they sit on top of the active/hover background.
            .when(production, |d| {
                d.child(div().absolute().inset_0().bg(theme::production_tint(theme)))
            })
            .when_some(t.origin.as_ref(), |d, o| {
                // A bright top edge reads as "selected", so only the active tab gets it at full strength.
                let stripe = theme::connection_color(o.color, theme);
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(theme::stripe())
                        .bg(if active { stripe } else { stripe.opacity(0.35) }),
                )
            })
            // The top edge belongs to the connection color, so selection gets its own mark at the bottom.
            .when(active, |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(theme::stripe())
                        .bg(theme.primary),
                )
            })
            .tooltip({
                let text = SharedString::from(tabs::tooltip_text(&t.title, t.origin.as_ref()));
                move |window, cx| Tooltip::new(text.clone()).build(window, cx)
            })
            .on_drag(
                TabDrag {
                    ix,
                    title: t.title.clone(),
                },
                |drag, _, _, cx| cx.new(|_| drag.clone()),
            )
            .drag_over::<TabDrag>(|d, _, _, cx| d.border_l_2().border_color(cx.theme().primary))
            .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                this.tabs.move_tab(drag.ix, ix);
                this.tab_changed(cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.tabs.activate(ix);
                this.tab_changed(cx);
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close_tabs(vec![ix], window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    this.tabs.activate(ix);
                    this.tab_changed(cx);
                }),
            )
            .context_menu(move |menu, _, _| {
                menu.menu("Close", Box::new(CloseTab))
                    .menu("Close others", Box::new(CloseOtherTabs))
                    .menu("Close to the right", Box::new(CloseTabsToRight))
                    .menu("Close all", Box::new(CloseAllTabs))
                    .separator()
                    .menu(if pinned { "Unpin" } else { "Pin" }, Box::new(TogglePinTab))
                    .separator()
                    .menu("Copy name", Box::new(CopyTabName))
                    .menu("Reveal in explorer", Box::new(RevealTab))
            })
            .child(icon.small())
            .when(!pinned, |d| {
                d.child(div().when(t.preview, |d| d.italic()).child(t.title.clone()))
                    // Text, not only the red wash, so production reads without color vision.
                    .when(production, |d| d.child(theme::prod_badge(theme)))
                    .child(
                        div()
                            .id(("close-tab", ix))
                            .rounded(theme::radius_sm())
                            .p(theme::space_2())
                            .text_color(if active {
                                theme.muted_foreground
                            } else {
                                transparent_black()
                            })
                            .group_hover(group, |d| d.text_color(theme.muted_foreground))
                            .hover(|d| d.bg(theme.secondary_hover).text_color(theme.foreground))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tabs(vec![ix], window, cx);
                            }))
                            .child(Icon::new(IconName::Close).xsmall()),
                    )
            })
            .into_any_element()
    }
}

/// Drag payload of a tab; also its drag ghost. Dropping on tab `j` means "before `j`".
#[derive(Clone)]
pub(super) struct TabDrag {
    pub(super) ix: usize,
    title: SharedString,
}

impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(theme::space_12())
            .py(theme::space_6())
            .bg(theme.tab_active)
            .border_1()
            .border_color(theme.border)
            .text_size(theme::ui_sm())
            .child(self.title.clone())
    }
}
