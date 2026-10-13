use super::*;

/// A Shell with 11 tabs and the overflow list open, its search field focused.
fn open_overflow(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<crate::shell::Shell>,
) {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::shell::bind_overflow_keys(cx);
    });
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let _ = cx.update_window(window, |_, _, cx| {
        shell.update(cx, |this, cx| {
            while !crate::ui::tabs::show_search(this.tabs.tabs.len()) {
                let ix = this.tabs.tabs.len();
                this.tabs.open(crate::shell::start::grid_tab(
                    format!("t{ix}"),
                    std::rc::Rc::new(crate::mock::edge_cases()),
                    cx,
                ));
            }
            this.overflow_open = true;
            cx.notify();
        });
    });
    // Let the popover mount and take focus, then focus the field as `on_open_change` does.
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    let search = shell.read_with(&vcx, |this, _| this.overflow_search.clone());
    vcx.update(|window, cx| search.update(cx, |s, cx| s.focus(window, cx)));
    vcx.run_until_parked();
    (window, shell)
}

#[gpui_kit::test]
fn typing_a_space_in_the_overflow_search_types_it_and_keeps_the_list_open(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_overflow(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    cx.simulate_keystrokes("a space b");
    shell.read_with(&cx, |this, cx| {
        assert_eq!(
            this.overflow_search.read(cx).value().to_string(),
            "a b",
            "search text"
        );
        assert!(this.overflow_open, "list stays open");
    });
}

#[gpui_kit::test]
fn overflow_list_closes_when_closing_tabs_drops_the_search_field(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_overflow(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    shell.update(&mut cx, |this, cx| {
        while crate::ui::tabs::show_search(this.tabs.tabs.len()) {
            this.tabs.close(0);
            this.tab_changed(cx);
        }
        assert!(
            !this.overflow_open,
            "open flag cleared once the list has no search field"
        );
    });
}
