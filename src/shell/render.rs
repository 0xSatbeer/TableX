use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme;
use crate::ui::{gallery, structure};

use super::{PANEL_MAXIMIZED_CONTEXT, Shell, TabContent, layout::Edge};

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Effects flush after the frame is presented, so the deferred stamp follows it.
        if crate::bench::frame_drawn() {
            cx.defer(|_| crate::bench::first_frame_presented());
        }
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
        let panel_focused = focused(Some(self.bottom_panel.focus_handle(cx)), cx);
        if let Some(TabContent::Console(console)) = self.tabs.active().map(|t| &t.content) {
            let text_size = self.console_text_size();
            console.update(cx, |console, _| console.text_size = text_size);
        }
        let theme = cx.theme();
        let viewer_bg = theme.table;
        let has_connections = !self.workspace.read(cx).connections.is_empty();
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
            None if !has_connections => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(theme::space_12())
                .child(
                    div()
                        .size(px(48.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::radius_lg())
                        .bg(theme.muted)
                        .text_color(theme::text_secondary(theme))
                        .child(Icon::new(Lucide::Database).size(px(24.))),
                )
                .child(
                    div()
                        .text_size(theme::ui_xl())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child("Connect a database to get started"),
                )
                .child(
                    div()
                        .max_w(px(360.))
                        .text_center()
                        .text_size(theme::ui())
                        .text_color(theme::text_secondary(theme))
                        .child("Add a PostgreSQL or SQLite connection to browse tables and run SQL."),
                )
                .mt(theme::space_8())
                .child(
                    div()
                        .debug_selector(|| "workspace-empty-add-connection".into())
                        .child(
                            Button::new("workspace-empty-add-connection")
                                .primary()
                                .icon(Lucide::Plus)
                                .label("Add connection")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.explorer.update(cx, |explorer, cx| {
                                        explorer.add_connection(window, cx)
                                    });
                                })),
                        ),
                )
                .into_any_element(),
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(theme::space_8())
                .text_size(theme::ui_sm())
                .text_color(theme::text_secondary(theme))
                .child(div().text_size(theme::ui()).text_color(theme.foreground).child("Nothing open."))
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
            .child(super::titlebar::Titlebar::new(
                self.window_title(cx),
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(theme::space_4())
                    .child(self.workspace_menu(cx))
                    .child(self.settings_button(cx)),
            ))
            .child(div().flex_1().min_h_0().map(|main| {
                if self.gallery_visible {
                    main.child(gallery::gallery(cx))
                } else {
                    let viewport = window.viewport_size();
                    let explorer_w = self.layout.explorer.shown(viewport.width);
                    let bottom_h = self.layout.bottom.shown(viewport.height);
                    let explorer_visible = has_connections && self.layout.explorer_visible;
                    let bottom_visible = has_connections && self.layout.bottom_visible;
                    let maximized = has_connections && self.layout.panel_maximized;
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
                            .when(explorer_visible, |d| {
                                // The last 1px column is the splitter line's own.
                                let pane = div()
                                    .flex_none()
                                    .h_full()
                                    .w(explorer_w - px(1.))
                                    .mr(px(1.))
                                    .overflow_hidden()
                                    .child(self.explorer.clone());
                                d.child(pane)
                            })
                            .child(
                                v_flex()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .when(!maximized, |d| {
                                        d.child(
                                            div().flex_1().min_h_0().overflow_hidden().child(
                                                v_flex().size_full().child(tabs).child(
                                                    div()
                                                        .flex_1()
                                                        .min_h_0()
                                                        .bg(viewer_bg)
                                                        .child(content),
                                                ),
                                            ),
                                        )
                                    })
                                    .when(bottom_visible, |d| {
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
                            .when(explorer_visible, |d| {
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
            .when_some(self.cheat_sheet.as_ref(), |d, open| {
                let dismiss = cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.close_cheat_sheet(window, cx)
                });
                d.child(crate::ui::cheat_sheet::overlay(&open.list, dismiss, cx))
            })
            .when_some(self.quick_open.as_ref(), |d, open| {
                let dismiss = cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.close_quick_open(None, window, cx)
                });
                d.child(crate::ui::quick_open::overlay(&open.list, dismiss, cx))
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
            .when_some(self.welcome.as_ref(), |d, welcome| {
                // Modal: focus stays inside so its keys work and nothing underneath takes them.
                if !welcome.focus.contains_focused(window, cx) {
                    window.focus(&welcome.focus, cx);
                }
                d.child(self.welcome_overlay(cx))
            })
    }
}

impl Shell {
    /// What you are looking at, qualified, in one line.
    fn window_title(&self, cx: &App) -> AnyElement {
        let tab = self.tabs.active();
        let text = crate::ui::tabs::window_title(
            &self.workspace.read(cx).name,
            tab.map(|t| (t.title.as_ref(), t.origin.as_ref())),
            tab.is_some_and(|t| matches!(t.content, TabContent::Grid(..))),
        );
        div()
            .max_w(px(640.))
            .truncate()
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(theme::ui())
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .into_any_element()
    }
}
