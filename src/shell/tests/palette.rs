use super::*;

#[gpui_kit::test]
fn typing_cls_tab_then_enter_closes_the_tab(cx: &mut gpui_kit::TestAppContext) {
    let (window, shell) = open_with_connection_tabs(cx);
    cx.update(|cx| {
        crate::keymap::bind(cx, &Default::default());
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let tabs =
        |cx: &mut gpui_kit::VisualTestContext| shell.read_with(cx, |this, _| this.tabs.tabs.len());
    let open = tabs(&mut cx);
    cx.simulate_keystrokes("ctrl-shift-p");
    cx.run_until_parked();
    assert!(
        shell.read_with(&cx, |this, _| this.palette.is_some()),
        "palette opens"
    );
    cx.simulate_input("cls tab");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        shell.read_with(&cx, |this, _| this.palette.is_none()),
        "palette closes"
    );
    assert_eq!(tabs(&mut cx), open - 1, "Close tab ran");
    let recent = cx.update(|_, cx| crate::ui::palette::Commands::new(cx).action(0));
    assert_eq!(
        recent,
        Some("queryx::CloseTab"),
        "most recent is listed first"
    );
}
