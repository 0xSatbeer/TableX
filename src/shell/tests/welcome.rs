use super::*;

/// A fresh install opens on the tour; Continue steps through it, a theme card applies at once,
/// and closing it saves the tour as done, so the next start goes straight to the app.
#[gpui_kit::test]
fn the_tour_runs_once_on_a_fresh_install(cx: &mut gpui_kit::TestAppContext) {
    use crate::theme::ThemeChoice;
    let dir = crate::theme::tests::test_dir("welcome");
    let path = dir.join("config.toml");
    cx.update(gpui_kit::init);
    let open = |cx: &mut gpui_kit::TestAppContext| {
        let (window, shell) = cx
            .update(|cx| {
                crate::shell::open_shell(
                    crate::shell::Mode::Normal,
                    Default::default(),
                    None,
                    Some(dir.clone()),
                    cx,
                )
            })
            .expect("open test window");
        let vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.run_until_parked();
        (vcx, shell)
    };
    let (mut vcx, shell) = open(cx);
    let step = |vcx: &mut gpui_kit::VisualTestContext| {
        shell.read_with(vcx, |this, _| this.welcome.as_ref().map(|w| w.step))
    };
    assert_eq!(step(&mut vcx), Some(0), "no config.toml: the tour opens");

    support::click(&mut vcx, "welcome-next");
    assert_eq!(step(&mut vcx), Some(1));
    support::click(&mut vcx, "welcome-theme-1");
    assert_eq!(
        shell.read_with(&vcx, |this, _| this.theme.clone()),
        ThemeChoice::Light
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("onboarded = false"),
        "a theme picked mid-tour doesn't end it"
    );

    support::click(&mut vcx, "welcome-skip");
    assert_eq!(step(&mut vcx), None);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("onboarded = true")
    );

    let (mut vcx, shell) = open(cx);
    assert!(
        shell.read_with(&vcx, |this, _| this.welcome.is_none()),
        "the second start skips the tour"
    );
    vcx.dispatch_action(crate::Welcome);
    assert!(
        shell.read_with(&vcx, |this, _| this.welcome.is_some()),
        "the palette's Welcome tour reopens it"
    );
}

/// With no connections the last step is the Add connection form; adding one ends the tour.
#[gpui_kit::test]
fn the_last_step_adds_the_first_connection(cx: &mut gpui_kit::TestAppContext) {
    let dir = crate::theme::tests::test_dir("welcome-connect");
    cx.update(gpui_kit::init);
    let (window, shell) = cx
        .update(|cx| {
            crate::shell::open_shell(
                crate::shell::Mode::Normal,
                Default::default(),
                None,
                Some(dir.clone()),
                cx,
            )
        })
        .expect("open test window");
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    for _ in 0..3 {
        support::click(&mut vcx, "welcome-next");
    }
    let form = shell.read_with(&vcx, |this, _| this.welcome_form().cloned());
    let form = form.expect("the form on the last step");

    support::click(&mut vcx, "welcome-next");
    assert!(
        shell.read_with(&vcx, |this, _| this.welcome.is_some()),
        "an empty form doesn't end the tour"
    );

    support::fill(
        &mut vcx,
        &form,
        &[
            ("name", "local"),
            ("kind-sqlite", ""),
            ("path", "/tmp/app.db"),
        ],
    );
    support::click(&mut vcx, "welcome-next");
    let (open, names) = shell.read_with(&vcx, |this, cx| {
        let ws = this.workspace.read(cx);
        let names: Vec<_> = ws.connections.iter().map(|c| c.name.clone()).collect();
        (this.welcome.is_some(), names)
    });
    assert!(!open, "adding the connection ends the tour");
    assert_eq!(names, ["local"]);
}
