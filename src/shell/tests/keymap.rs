use super::*;

#[gpui_kit::test]
fn a_rebound_close_tab_moves_to_its_new_keys(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        let overrides = [("queryx::CloseTab".to_string(), "ctrl-q".to_string())].into();
        assert!(crate::keymap::bind(cx, &overrides).is_empty());
    });
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    shell.update(&mut cx, |this, cx| {
        this.tabs.open(crate::shell::start::grid_tab(
            "t".to_string(),
            std::rc::Rc::new(crate::mock::edge_cases()),
            cx,
        ));
        // As activating a tab does: the next render focuses its grid.
        this.focus_content = true;
        cx.notify();
    });
    cx.run_until_parked();
    let tabs =
        |cx: &mut gpui_kit::VisualTestContext| shell.read_with(cx, |this, _| this.tabs.tabs.len());
    let open = tabs(&mut cx);
    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    assert_eq!(tabs(&mut cx), open, "ctrl-w no longer closes");
    cx.simulate_keystrokes("ctrl-q");
    cx.run_until_parked();
    assert_eq!(tabs(&mut cx), open - 1, "ctrl-q closes");
}
