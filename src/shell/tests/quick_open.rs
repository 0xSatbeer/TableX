use super::*;

#[gpui_kit::test]
fn ctrl_p_then_typing_and_enter_opens_the_object_in_a_tab(cx: &mut gpui_kit::TestAppContext) {
    let (window, shell) = open_plain(cx);
    cx.update(|cx| {
        crate::keymap::bind(cx, &Default::default());
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.run_until_parked();
    assert!(
        shell.read_with(&cx, |this, _| this.quick_open.is_some()),
        "quick open opens"
    );
    cx.simulate_input("users");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        shell.read_with(&cx, |this, _| this.quick_open.is_none()),
        "quick open closes"
    );
    let active = shell.read_with(&cx, |this, _| this.tabs.active().map(|t| t.key.to_string()));
    assert_eq!(active.as_deref(), Some("1/app_db/public/table/users"));
}
