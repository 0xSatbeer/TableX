use super::*;

#[gpui_kit::test]
fn ctrl_k_ctrl_s_opens_the_sheet_and_escape_closes_it(cx: &mut gpui_kit::TestAppContext) {
    let (window, shell) = open_with_connection_tabs(cx);
    cx.update(|cx| {
        crate::keymap::bind(cx, &Default::default());
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let open = |cx: &mut gpui_kit::VisualTestContext| {
        shell.read_with(cx, |this, _| this.cheat_sheet.is_some())
    };
    cx.simulate_keystrokes("ctrl-k ctrl-s");
    cx.run_until_parked();
    assert!(open(&mut cx), "sheet opens");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!open(&mut cx), "sheet closes");
}
