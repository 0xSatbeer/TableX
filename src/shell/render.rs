use gpui_kit::component::{ActiveTheme as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme;
use crate::ui::{gallery, structure};

use super::{PANEL_MAXIMIZED_CONTEXT, Shell, TabContent, layout::Edge};

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A vertical wheel scrolls the tab strip sideways, like VS Code.
        let tabs = div()
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                let delta = ev.delta.pixel_delta(window.line_height());
                let max = this.tab_scroll.max_offset().x.max(Pixels::ZERO);
                let mut offset = this.tab_scroll.offset();
                offset.x = (offset.x + delta.y).clamp(-max, Pixels::ZERO);
                this.tab_scroll.set_offset(offset);
                cx.notify();
            }))
            .child(self.tab_bar(cx));
        // Every way out of maximize refocuses the content: it is unmounted while maximized, so stay pending.
        if self.layout.panel_maximized {
            self.focus_content = true;
        } else if std::mem::take(&mut self.focus_content)
            // With no tab open the tree takes focus, so the window's keys still have a target.
            && let Some(handle) = self
                .content_focus(cx)
                .or_else(|| Some(self.explorer.focus_handle(cx)))
        {
            window.focus(&handle, cx);
        }
        let focused = |handle: Option<FocusHandle>, cx: &App| {
            handle.is_some_and(|h| h.contains_focused(window, cx))
        };
        let content_focused = focused(self.content_focus(cx), cx);
        // The console editor shares the content pane; only the grid itself counts for the aggregate.
        let grid_focused = content_focused
            && match self.tabs.active().map(|t| &t.content) {
                Some(TabContent::Grid(_, s)) => s
                    .as_ref()
                    .is_none_or(|s| s.read(cx).sub == structure::Sub::Data),
                _ => false,
            };
        self.status_bar
            .update(cx, |bar, cx| bar.set_grid_focused(grid_focused, cx));
        let explorer_focused = focused(Some(self.explorer.focus_handle(cx)), cx);
        let panel_focused = focused(Some(self.bottom_panel.focus_handle(cx)), cx);
        if let Some(TabContent::Console(console)) = self.tabs.active().map(|t| &t.content) {
            let text_size = self.console_text_size();
            console.update(cx, |console, _| console.text_size = text_size);
        }
        let theme = cx.theme();
        let content = match self.tabs.active().map(|t| &t.content) {
            Some(TabContent::Grid(_, Some(s))) => s.clone().into_any_element(),
            Some(TabContent::Grid(grid, None)) => grid.clone().into_any_element(),
            Some(TabContent::Console(console)) => {
                let production = self.tabs.active().and_then(|t| t.origin.as_ref()).is_some_and(|o| o.production);
                // ponytail: a rule, not the full background tint QX-012 brings with the console toolbar
                h_flex()
                    .size_full()
                    .when(production, |d| d.child(div().flex_none().w(theme::stripe()).h_full().bg(theme.danger)))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .on_action(cx.listener(|this, _: &theme::ZoomIn, window, cx| this.zoom_console(theme::Zoom::In, window, cx)))
                            .on_action(cx.listener(|this, _: &theme::ZoomOut, window, cx| this.zoom_console(theme::Zoom::Out, window, cx)))
                            .on_action(cx.listener(|this, _: &theme::ZoomReset, window, cx| this.zoom_console(theme::Zoom::Reset, window, cx)))
                            .child(console.clone()),
                    )
                    .into_any_element()
            }
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(theme::space_8())
                .text_size(theme::ui_sm())
                .text_color(theme.muted_foreground)
                .child(div().text_size(theme::ui()).text_color(theme::text_secondary(theme)).child("Nothing open."))
                .child("Pick a table in the explorer (Ctrl+Alt+F) · Ctrl+Shift+T brings the last tab back")
                .into_any_element(),
        };

        v_flex()
            .relative()
            .size_full()
            // Only while maximized: a tracked root focuses itself on every click on plain chrome.
            .when(self.layout.panel_maximized, |d| {
                d.track_focus(&self.focus_handle)
                    .key_context(PANEL_MAXIMIZED_CONTEXT)
            })
            .bg(theme.background)
            .text_size(theme::ui())
            .text_color(theme.foreground)
            .map(|root| super::actions::on_actions(root, cx))
            .child(
                TitleBar::new().h(theme::titlebar()).child(
                    h_flex()
                        .w_full()
                        .pr(theme::space_8())
                        .justify_between()
                        // Wordmark: a console prompt with its cursor parked. Static, nothing moves when idle.
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(1.))
                                .font_family(theme.mono_font_family.clone())
                                .font_weight(FontWeight::BOLD)
                                .text_size(theme::ui())
                                .child("queryx")
                                .child(
                                    div()
                                        .w(theme::space_6() + px(1.))
                                        .h(theme::ui())
                                        .bg(theme.primary),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap(theme::space_4())
                                .child(self.workspace_menu(cx))
                                .child(self.settings_menu(cx)),
                        ),
                ),
            )
            .child(div().flex_1().min_h_0().map(|main| {
                if self.gallery_visible {
                    main.child(gallery::gallery(cx))
                } else {
                    let viewport = window.viewport_size();
                    let explorer_w = self.layout.explorer.shown(viewport.width);
                    let bottom_h = self.layout.bottom.shown(viewport.height);
                    let maximized = self.layout.panel_maximized;
                    let console = match self.tabs.active().map(|t| &t.content) {
                        Some(TabContent::Console(console)) => Some(console.clone()),
                        _ => None,
                    };
                    self.bottom_panel.update(cx, |panel, _| {
                        (panel.maximized, panel.console) = (maximized, console)
                    });
                    main.child(
                        h_flex()
                            .relative()
                            .size_full()
                            .when(self.layout.explorer_visible, |d| {
                                // The last 1px column is the splitter line's own, so it never paints over the ring.
                                let pane = div()
                                    .flex_none()
                                    .h_full()
                                    .w(explorer_w - px(1.))
                                    .mr(px(1.))
                                    .overflow_hidden()
                                    .child(self.explorer.clone());
                                d.child(theme::focus_ring(
                                    "pane-explorer",
                                    pane,
                                    explorer_focused,
                                    cx.theme(),
                                ))
                            })
                            .child(
                                v_flex()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .when(!maximized, |d| {
                                        d.child(div().flex_1().min_h_0().overflow_hidden().child(
                                            v_flex().size_full().child(tabs).child(
                                                theme::focus_ring(
                                                    "pane-content",
                                                    div().flex_1().min_h_0().child(content),
                                                    content_focused,
                                                    cx.theme(),
                                                ),
                                            ),
                                        ))
                                    })
                                    .when(self.layout.bottom_visible, |d| {
                                        let panel = div()
                                            .w_full()
                                            .overflow_hidden()
                                            .child(self.bottom_panel.clone());
                                        let panel = theme::focus_ring(
                                            "pane-panel",
                                            panel,
                                            panel_focused,
                                            cx.theme(),
                                        );
                                        if maximized {
                                            d.child(panel.flex_1().min_h_0())
                                        } else {
                                            // The top 1px row is the splitter line's own, as with the explorer.
                                            d.child(
                                                panel.flex_none().h(bottom_h - px(1.)).mt(px(1.)),
                                            )
                                            .child(self.splitter(Edge::Bottom, bottom_h, cx))
                                        }
                                    }),
                            )
                            .when(self.layout.explorer_visible, |d| {
                                d.child(self.splitter(Edge::Explorer, explorer_w, cx))
                            }),
                    )
                }
            }))
            .child(self.status_bar.clone())
            .when_some(self.palette.as_ref(), |d, open| {
                let dismiss = cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.close_palette(None, window, cx)
                });
                d.child(crate::ui::palette::overlay(&open.list, dismiss, cx))
            })
            // While a splitter is dragged, one layer over everything takes the pointer, so the
            // panels underneath neither react nor swap the cursor.
            .when_some(self.layout.dragging.as_ref().map(|d| d.edge), |d, edge| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .map(|d| {
                            if edge == Edge::Explorer {
                                d.cursor_col_resize()
                            } else {
                                d.cursor_row_resize()
                            }
                        })
                        .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                            this.layout.drag_to(ev.position, window.viewport_size());
                            cx.notify();
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseUpEvent, window, cx| {
                                this.layout.release(ev.position, window.viewport_size());
                                cx.notify();
                            }),
                        ),
                )
            })
    }
}
