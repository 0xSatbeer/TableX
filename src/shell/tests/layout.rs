use crate::shell::layout::{Edge, LayoutState};
use crate::ui::split;
use gpui_kit::{point, px, size};

fn layout() -> LayoutState {
    LayoutState {
        explorer_visible: true,
        bottom_visible: true,
        maximized_from: None,
        panel_maximized: false,
        explorer: split::EXPLORER,
        bottom: split::BOTTOM,
        dragging: None,
    }
}

#[test]
fn explorer_released_below_min_hides_and_ctrl_b_shows_it_at_the_pre_drag_width() {
    let window = size(px(1200.), px(800.));
    let mut l = layout();
    l.press(Edge::Explorer, point(px(260.), px(300.)), window);
    l.drag_to(point(px(179.), px(300.)), window);
    l.release(point(px(179.), px(300.)), window);
    assert!(!l.explorer_visible, "explorer hidden after release at 179");
    assert_eq!(
        l.explorer.size,
        px(260.),
        "stored explorer width after release at 179"
    );
    l.toggle_explorer();
    assert!(l.explorer_visible, "explorer visible after Ctrl+B");
    assert_eq!(
        l.explorer.shown(window.width),
        px(260.),
        "explorer width after Ctrl+B"
    );
}

#[test]
fn bottom_click_without_moving_keeps_its_stored_height() {
    let small = size(px(1200.), px(600.));
    let mut l = layout();
    l.bottom.size = px(500.);
    let at = point(px(600.), px(180.));
    l.press(Edge::Bottom, at, small);
    l.release(at, small);
    assert_eq!(
        l.bottom.size,
        px(500.),
        "stored bottom height after a click at window height 600"
    );
    assert_eq!(
        l.bottom.shown(px(800.)),
        px(500.),
        "bottom height after the window grows to 800"
    );
}

#[test]
fn ctrl_alt_f_shows_a_hidden_explorer_and_never_hides_it() {
    let mut l = layout();
    l.explorer_visible = false;
    l.show_explorer();
    assert!(l.explorer_visible, "hidden -> shown");
    l.show_explorer();
    assert!(l.explorer_visible, "visible stays visible");
    l.toggle_maximize();
    assert!(!l.explorer_visible, "maximized hides it");
    l.show_explorer();
    assert!(l.explorer_visible, "maximized -> shown");
    assert!(
        l.maximized_from.is_none(),
        "maximize state dropped so the next toggle starts fresh"
    );
}

#[test]
fn panel_maximize_toggles_and_keeps_sizes() {
    let window = size(px(1200.), px(800.));
    let mut l = layout();
    l.explorer.size = px(300.);
    l.bottom.size = px(333.);
    l.toggle_panel_maximize();
    assert!(l.panel_maximized, "maximized after the first press");
    assert!(l.bottom_visible, "panel stays visible when maximized");
    l.toggle_panel_maximize();
    assert!(!l.panel_maximized, "restored after the second press");
    l.toggle_panel_maximize();
    l.restore_panel();
    assert!(!l.panel_maximized, "Esc restores");
    l.restore_panel();
    assert!(!l.panel_maximized, "Esc when not maximized does nothing");
    assert_eq!(
        (
            l.explorer.shown(window.width),
            l.bottom.shown(window.height)
        ),
        (px(300.), px(333.)),
        "previous sizes restored"
    );
    assert!(l.explorer_visible, "explorer visibility untouched");
}

#[test]
fn panel_maximize_needs_a_visible_panel_and_ends_when_it_hides() {
    let mut l = layout();
    l.bottom_visible = false;
    l.toggle_panel_maximize();
    assert!(!l.panel_maximized, "hidden panel can't be maximized");
    l.bottom_visible = true;
    l.toggle_panel_maximize();
    assert!(l.panel_maximized, "visible panel maximizes");
    l.toggle_bottom();
    assert!(
        !l.bottom_visible && !l.panel_maximized,
        "Ctrl+J while maximized hides the panel and ends maximize"
    );
    l.toggle_bottom();
    assert!(
        l.bottom_visible && !l.panel_maximized,
        "panel comes back at its normal size"
    );
    l.toggle_panel_maximize();
    assert!(l.panel_maximized, "maximized again");
    l.toggle_maximize();
    assert!(
        !l.bottom_visible && !l.panel_maximized,
        "Ctrl+Shift+F while maximized hides everything and ends maximize"
    );
}
