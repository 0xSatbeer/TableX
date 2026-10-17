use super::*;

/// A Shell with the panel maximized, on the Logs tab with its filter field focused if `logs_focused`.
fn open_maximized(
    cx: &mut gpui_kit::TestAppContext,
    logs_focused: bool,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<crate::shell::Shell>,
) {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::shell::bind_panel_keys(cx);
    });
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    if logs_focused {
        shell.update(&mut vcx, |this, cx| {
            this.bottom_panel.update(cx, |p, _| p.show_logs())
        });
    }
    vcx.run_until_parked();
    // As the header's maximize button does.
    vcx.dispatch_action(crate::MaximizePanel);
    vcx.run_until_parked();
    if logs_focused {
        let search = shell.read_with(&vcx, |this, cx| this.bottom_panel.read(cx).search_field());
        vcx.update(|window, cx| search.update(cx, |s, cx| s.focus(window, cx)));
        vcx.run_until_parked();
    }
    (window, shell)
}

#[gpui_kit::test]
fn esc_while_maximized_restores_the_panel(cx: &mut gpui_kit::TestAppContext) {
    for logs_focused in [false, true] {
        let (window, shell) = open_maximized(cx, logs_focused);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let sizes = |shell: &gpui_kit::Entity<crate::shell::Shell>,
                     cx: &gpui_kit::VisualTestContext| {
            shell.read_with(cx, |this, _| {
                (this.layout.explorer.size, this.layout.bottom.size)
            })
        };
        let before = sizes(&shell, &cx);
        assert!(
            shell.read_with(&cx, |this, _| this.layout.panel_maximized),
            "maximized (logs_focused={logs_focused})"
        );
        cx.simulate_keystrokes("escape");
        assert!(
            !shell.read_with(&cx, |this, _| this.layout.panel_maximized),
            "Esc restores (logs_focused={logs_focused})"
        );
        assert_eq!(
            sizes(&shell, &cx),
            before,
            "stored sizes unchanged (logs_focused={logs_focused})"
        );
        let root = shell.read_with(&cx, |this, _| this.focus_handle.clone());
        assert!(
            !cx.update(|window, _| root.is_focused(window)),
            "content refocused after restore (logs_focused={logs_focused})"
        );
    }
}

#[gpui_kit::test]
fn tab_change_while_maximized_keeps_esc_restoring_the_panel(cx: &mut gpui_kit::TestAppContext) {
    for logs_focused in [false, true] {
        let (window, shell) = open_maximized(cx, logs_focused);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        cx.dispatch_action(crate::NextTab);
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        assert!(
            !shell.read_with(&cx, |this, _| this.layout.panel_maximized),
            "Esc restores after a tab change (logs_focused={logs_focused})"
        );
    }
}

#[gpui_kit::test]
fn every_way_out_of_panel_maximize_refocuses_the_content(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::Focusable as _;
    // The bottom splitter isn't drawn while maximized, so a drag can't hide the panel from there.
    let exits: [(&str, fn(&mut gpui_kit::VisualTestContext)); 4] = [
        ("Esc", |cx| cx.simulate_keystrokes("escape")),
        ("maximize button again", |cx| {
            cx.dispatch_action(crate::MaximizePanel)
        }),
        ("Ctrl+J / hide panel button", |cx| {
            cx.dispatch_action(crate::ToggleBottomPanel)
        }),
        ("Ctrl+Shift+F", |cx| {
            cx.dispatch_action(crate::MaximizeEditor)
        }),
    ];
    for (exit, run) in exits {
        for logs_focused in [false, true] {
            let (window, shell) = open_maximized(cx, logs_focused);
            let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
            run(&mut cx);
            cx.run_until_parked();
            let (maximized, root, content) = shell.read_with(&cx, |this, cx| {
                let content = match this.tabs.active().map(|t| &t.content) {
                    Some(crate::shell::TabContent::Grid(grid, _)) => grid.focus_handle(cx),
                    Some(crate::shell::TabContent::Console(_)) => {
                        super::console_1(this, cx).focus_handle(cx)
                    }
                    None => panic!("launch opens a tab"),
                };
                (
                    this.layout.panel_maximized,
                    this.focus_handle.clone(),
                    content,
                )
            });
            assert!(
                !maximized,
                "{exit} ends maximize (logs_focused={logs_focused})"
            );
            assert!(
                !cx.update(|window, _| root.is_focused(window)),
                "{exit} leaves the root (logs_focused={logs_focused})"
            );
            assert!(
                cx.update(|window, _| content.is_focused(window)),
                "{exit} focuses the content (logs_focused={logs_focused})"
            );
        }
    }
}

#[gpui_kit::test]
fn clicking_chrome_keeps_focus_when_not_maximized(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{AppContext as _, Focusable as _};
    cx.update(gpui_kit::init);
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    let editor = shell.read_with(&cx, |this, cx| super::console_1(this, cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&editor, cx));
    let bottom_middle = cx.update(|window, _| {
        let v = window.viewport_size();
        gpui_kit::point(v.width / 2., v.height - gpui_kit::px(4.))
    });
    cx.simulate_click(bottom_middle, Default::default());
    assert!(
        cx.update(|window, _| editor.is_focused(window)),
        "a click on non-focusable chrome must not steal focus"
    );
}
