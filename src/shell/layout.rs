use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::split;

use super::{Shell, TabContent};

/// Grab band of a splitter on each side of its 1px line.
const SPLITTER_PAD: Pixels = px(4.);

/// The panel edge a splitter moves.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Edge {
    Explorer,
    Bottom,
}

/// A splitter drag in progress: where the pointer went down, the panel's drawn size at that
/// moment, and its stored size, which can be larger than drawn when the window is short.
pub(super) struct Dragging {
    pub(super) edge: Edge,
    from: Point<Pixels>,
    start: Pixels,
    stored: Pixels,
}

/// Panel visibility and pixel sizes in one place, so QX-020 can persist it.
/// Hidden panels keep their size, so showing one again brings it back as it was.
pub(super) struct LayoutState {
    pub(super) explorer_visible: bool,
    pub(super) bottom_visible: bool,
    /// Visibility of (explorer, bottom) from before `MaximizeEditor`, restored by the next one.
    pub(super) maximized_from: Option<(bool, bool)>,
    /// The bottom panel covers the editor area; sizes stay as they were, so un-maximizing restores them.
    pub(super) panel_maximized: bool,
    pub(super) explorer: split::Split,
    pub(super) bottom: split::Split,
    pub(super) dragging: Option<Dragging>,
}

impl LayoutState {
    pub(super) fn split(&mut self, edge: Edge) -> &mut split::Split {
        match edge {
            Edge::Explorer => &mut self.explorer,
            Edge::Bottom => &mut self.bottom,
        }
    }

    /// The size `drag` asks for with the pointer at `to`, and the window extent along its axis.
    fn requested(drag: &Dragging, to: Point<Pixels>, window: Size<Pixels>) -> (Pixels, Pixels) {
        match drag.edge {
            Edge::Explorer => (drag.start + (to.x - drag.from.x), window.width),
            Edge::Bottom => (drag.start - (to.y - drag.from.y), window.height),
        }
    }

    /// Starts a drag of `edge` from the panel's drawn size.
    pub(super) fn press(&mut self, edge: Edge, at: Point<Pixels>, window: Size<Pixels>) {
        let extent = if edge == Edge::Explorer {
            window.width
        } else {
            window.height
        };
        let split = self.split(edge);
        let (start, stored) = (split.shown(extent), split.size);
        self.dragging = Some(Dragging {
            edge,
            from: at,
            start,
            stored,
        });
    }

    /// Follows the pointer, clamped; going below the minimum only hides on release.
    pub(super) fn drag_to(&mut self, to: Point<Pixels>, window: Size<Pixels>) {
        let Some(drag) = &self.dragging else { return };
        let (size, extent) = Self::requested(drag, to, window);
        let edge = drag.edge;
        let split = self.split(edge);
        split.size = size.clamp(split.min, (split.max)(extent));
    }

    pub(super) fn release(&mut self, at: Point<Pixels>, window: Size<Pixels>) {
        let Some(drag) = self.dragging.take() else {
            return;
        };
        let (size, extent) = Self::requested(&drag, at, window);
        let split = self.split(drag.edge);
        // A click is not a resize: keep the stored size instead of the drawn one.
        if at == drag.from {
            split.size = drag.stored;
            return;
        }
        match split::resolve_drag(size, split.min, (split.max)(extent), drag.start) {
            split::Drag::Size(size) => split.size = size,
            split::Drag::Hidden(size) => {
                split.size = size;
                match drag.edge {
                    Edge::Explorer => self.explorer_visible = false,
                    Edge::Bottom => self.bottom_visible = false,
                }
            }
        }
    }

    pub(super) fn toggle_explorer(&mut self) {
        self.maximized_from = None;
        self.explorer_visible = !self.explorer_visible;
    }

    /// Ctrl+Alt+F / reveal: shows the explorer if hidden; never hides it.
    pub(super) fn show_explorer(&mut self) {
        self.maximized_from = None;
        self.explorer_visible = true;
    }

    pub(super) fn toggle_bottom(&mut self) {
        self.maximized_from = None;
        self.panel_maximized = false;
        self.bottom_visible = !self.bottom_visible;
    }

    /// Maximize button on the panel header; does nothing while the panel is hidden.
    pub(super) fn toggle_panel_maximize(&mut self) {
        self.panel_maximized = self.bottom_visible && !self.panel_maximized;
    }

    /// Esc: leaves panel maximize; does nothing otherwise.
    pub(super) fn restore_panel(&mut self) {
        self.panel_maximized = false;
    }

    pub(super) fn toggle_maximize(&mut self) {
        self.panel_maximized = false;
        match self.maximized_from.take() {
            Some((explorer, bottom)) => {
                (self.explorer_visible, self.bottom_visible) = (explorer, bottom)
            }
            None => {
                self.maximized_from = Some((self.explorer_visible, self.bottom_visible));
                (self.explorer_visible, self.bottom_visible) = (false, false);
            }
        }
    }
}

impl Shell {
    /// Focus handle of the active tab's content: its grid or the console editor.
    pub(super) fn content_focus(&self, cx: &App) -> Option<FocusHandle> {
        match self.tabs.active().map(|t| &t.content) {
            Some(TabContent::Grid(grid)) => Some(grid.focus_handle(cx)),
            Some(TabContent::Console(editor)) => Some(editor.focus_handle(cx)),
            None => None,
        }
    }

    /// Our own handle over the panel's edge, drawn after the panels so its band sits on top.
    /// `size` is the panel's drawn size; the 1px line covers the panel's own edge pixel.
    pub(super) fn splitter(
        &self,
        edge: Edge,
        size: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let dragging = self
            .layout
            .dragging
            .as_ref()
            .is_some_and(|d| d.edge == edge);
        let offset = size - px(1.) - SPLITTER_PAD;
        let name = if edge == Edge::Explorer {
            "splitter-explorer"
        } else {
            "splitter-bottom"
        };
        let line = div()
            .debug_selector(move || name.into())
            .flex_none()
            .bg(if dragging {
                theme.primary
            } else {
                theme.border
            });
        div()
            .absolute()
            .occlude()
            .map(|d| match edge {
                Edge::Explorer => d
                    .top_0()
                    .h_full()
                    .left(offset)
                    .px(SPLITTER_PAD)
                    .cursor_col_resize()
                    .child(line.w(px(1.)).h_full()),
                Edge::Bottom => d
                    .left_0()
                    .w_full()
                    .bottom(offset)
                    .py(SPLITTER_PAD)
                    .cursor_row_resize()
                    .child(line.h(px(1.)).w_full()),
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if ev.click_count >= 2 {
                        this.layout.dragging = None;
                        this.layout.split(edge).reset();
                    } else {
                        this.layout.press(edge, ev.position, window.viewport_size());
                    }
                    cx.notify();
                }),
            )
    }
}
