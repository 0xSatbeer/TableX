use super::*;

#[gpui_kit::test]
fn toggle_gallery_shows_the_gallery_only_in_demo_mode(cx: &mut gpui_kit::TestAppContext) {
    let cases = [
        (crate::shell::Mode::Normal, false),
        (crate::shell::Mode::Demo, true),
    ];
    for (mode, visible) in cases {
        let (window, shell) = open_started(cx, mode);
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.dispatch_action(crate::ToggleGallery);
        vcx.run_until_parked();
        assert_eq!(
            shell.read_with(&vcx, |this, _| this.gallery_visible),
            visible,
            "{mode:?}"
        );
    }
}
