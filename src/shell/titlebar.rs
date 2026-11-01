//! The window titlebar: square min/max/close on the side the desktop's button layout puts close
//! (gpui-kit's TitleBar always puts them on the right).

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme;

#[derive(IntoElement)]
pub(super) struct Titlebar {
    title: AnyElement,
    content: AnyElement,
}

impl Titlebar {
    /// `title` is centered on the whole bar; `content` sits between the window buttons.
    pub(super) fn new(title: impl IntoElement, content: impl IntoElement) -> Self {
        Self {
            title: title.into_any_element(),
            content: content.into_any_element(),
        }
    }
}

/// Set on mouse down, consumed by the first move: a click or double click never starts a drag.
struct Drag(bool);

impl RenderOnce for Titlebar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let drag = window.use_state(cx, |_, _| Drag(false));
        let (left, right) = buttons(window, cx);
        h_flex()
            .id("titlebar")
            .relative()
            .flex_shrink_0()
            .h(theme::titlebar())
            // Square buttons sit flush with the window edge; only a bare side gets padding.
            .when(left.is_empty(), |d| d.pl(theme::space_8()))
            .when(right.is_empty(), |d| d.pr(theme::space_8()))
            .gap(theme::space_8())
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .bg({
                // A step darker than the theme's bar, so it separates from the tab strip below.
                let c = cx.theme().title_bar;
                hsla(c.h, c.s, (c.l - 0.04).max(0.), c.a)
            })
            // macOS draws its own traffic lights over the left edge.
            .when(cfg!(target_os = "macos"), |d| d.pl(px(80.)))
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&drag, |d, _, _, _| d.0 = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&drag, |d, _, _, _| d.0 = false),
            )
            .on_mouse_down_out(window.listener_for(&drag, |d, _, _, _| d.0 = false))
            .on_mouse_move(window.listener_for(&drag, |d, _, window, _| {
                if std::mem::take(&mut d.0) {
                    window.start_window_move();
                }
            }))
            .on_mouse_down(MouseButton::Right, |ev, window, _| {
                window.show_window_menu(ev.position)
            })
            .on_click(|ev, window, _| {
                if ev.click_count() == 2 {
                    window.zoom_window();
                }
            })
            // First child, so it paints under the controls if a long title reaches them.
            .child(
                h_flex()
                    .absolute()
                    .inset_0()
                    .justify_center()
                    .child(self.title),
            )
            .children(controls(left, window, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(self.content),
            )
            .children(controls(right, window, cx))
    }
}

/// The desktop's (left, right) buttons; none when the compositor decorates the window itself.
fn buttons(window: &Window, cx: &App) -> (Vec<WindowButton>, Vec<WindowButton>) {
    if cfg!(target_os = "macos")
        || !matches!(window.window_decorations(), Decorations::Client { .. })
    {
        return Default::default();
    }
    let supported = window.window_controls();
    // Always minimize, maximize, close; the desktop layout only picks the side (close on the left → left).
    let all: Vec<WindowButton> = [
        WindowButton::Minimize,
        WindowButton::Maximize,
        WindowButton::Close,
    ]
    .into_iter()
    .filter(|b| match b {
        WindowButton::Minimize => supported.minimize,
        WindowButton::Maximize => supported.maximize,
        WindowButton::Close => true,
    })
    .collect();
    let close_left = cx
        .button_layout()
        .is_some_and(|l| l.left.contains(&Some(WindowButton::Close)));
    if close_left {
        (all.into_iter().rev().collect(), vec![])
    } else {
        (vec![], all)
    }
}

fn controls(buttons: Vec<WindowButton>, window: &Window, cx: &App) -> Option<impl IntoElement> {
    if buttons.is_empty() {
        return None;
    }
    let maximized = window.is_maximized();
    let t = cx.theme();
    let (fg, hover, active) = (t.foreground, t.secondary_hover, t.secondary_active);
    let (danger, danger_active, danger_fg) = (t.danger, t.danger_active, t.danger_foreground);
    Some(
        h_flex()
            .h_full()
            .children(buttons.into_iter().map(move |b| {
                let (icon, label, area) = match b {
                    WindowButton::Minimize => {
                        (IconName::WindowMinimize, "Minimize", WindowControlArea::Min)
                    }
                    WindowButton::Maximize if maximized => {
                        (IconName::WindowRestore, "Restore", WindowControlArea::Max)
                    }
                    WindowButton::Maximize => {
                        (IconName::WindowMaximize, "Maximize", WindowControlArea::Max)
                    }
                    WindowButton::Close => {
                        (IconName::WindowClose, "Close", WindowControlArea::Close)
                    }
                };
                div()
                    .id(b.id())
                    .debug_selector(move || format!("window-{}", b.id()))
                    .h_full()
                    .w(theme::titlebar())
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(fg)
                    .map(|d| match b {
                        WindowButton::Close => d
                            .hover(|s| s.bg(danger).text_color(danger_fg))
                            .active(|s| s.bg(danger_active).text_color(danger_fg)),
                        _ => d.hover(|s| s.bg(hover)).active(|s| s.bg(active)),
                    })
                    // Windows hit-tests these areas itself; Linux needs the click handler.
                    .window_control_area(area)
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        match b {
                            WindowButton::Minimize => window.minimize_window(),
                            WindowButton::Maximize => window.zoom_window(),
                            WindowButton::Close => window.remove_window(),
                        }
                    })
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx)
                    })
                    .child(Icon::new(icon).small())
            })),
    )
}
