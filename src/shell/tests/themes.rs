use super::*;

/// Edits to the active user theme's file apply without a restart; the theme dir is only read.
#[gpui_kit::test]
fn editing_the_active_user_theme_applies_without_restart(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{AppContext as _, component::ActiveTheme as _};
    let theme_json = |bg: &str| {
        format!(
            r##"{{"name": "Nord", "themes": [{{"name": "Nord", "mode": "dark", "colors": {{"background": "{bg}"}}}}]}}"##
        )
    };
    let dir = crate::theme::tests::test_dir("watch");
    let missing = crate::theme::tests::test_dir("watch-missing").join("themes");
    std::fs::write(dir.join("nord.json"), theme_json("#2E3440")).unwrap();
    cx.update(gpui_kit::init);
    let open = |dir: std::path::PathBuf, cx: &mut gpui_kit::TestAppContext| {
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut shell = demo_shell(fake(), window, cx);
                    shell.watch_user_themes(dir, window, cx);
                    shell
                })
            })
            .expect("open test window")
        });
        cx.run_until_parked();
        (window, shell)
    };
    let (window, shell) = open(dir.clone(), cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| {
        shell.update(cx, |this, cx| {
            this.theme = crate::theme::ThemeChoice::User("Nord".into());
            this.theme.apply(&this.user_themes, window, cx);
        })
    });
    let background =
        |vcx: &mut gpui_kit::VisualTestContext| vcx.update(|_, cx| cx.theme().background);
    assert_eq!(
        background(&mut vcx),
        gpui_kit::rgb(0x2E3440).into(),
        "Nord applied"
    );

    std::fs::write(dir.join("nord.json"), format!("{} ", theme_json("#3B4252"))).unwrap(); // longer too, so the change doesn't hinge on mtime resolution
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    vcx.run_until_parked();
    assert_eq!(
        background(&mut vcx),
        gpui_kit::rgb(0x3B4252).into(),
        "edited Nord applied without restart"
    );
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(files, ["nord.json"], "the theme dir is only read");

    open(missing.clone(), cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    cx.run_until_parked();
    assert!(!missing.exists(), "a missing theme dir is not created");
}

/// An unrelated edit doesn't repeat a known problem; deleting the active theme's file falls back to Dark.
#[gpui_kit::test]
fn reloading_user_themes_repeats_no_notification_and_falls_back_when_the_file_is_gone(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{AppContext as _, component::WindowExt as _};
    let nord = |bg: &str| {
        format!(
            r##"{{"name": "Nord", "themes": [{{"name": "Nord", "mode": "dark", "colors": {{"background": "{bg}"}}}}]}}"##
        )
    };
    let dir = crate::theme::tests::test_dir("reload");
    std::fs::write(dir.join("bad.json"), "not json").unwrap();
    std::fs::write(dir.join("nord.json"), nord("#2E3440")).unwrap();
    cx.update(gpui_kit::init);
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    // After the window exists: notifications need its root.
    vcx.update(|window, cx| {
        shell.update(cx, |this, cx| {
            this.watch_user_themes(dir.clone(), window, cx)
        })
    });
    vcx.run_until_parked();
    let count = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.update(|window, cx| window.notifications(cx).len())
    };
    assert_eq!(count(&mut vcx), 1, "bad.json reported once on load");

    // (a) an unrelated edit reloads, but bad.json is not reported again.
    vcx.update(|window, cx| window.clear_notifications(cx));
    std::fs::write(dir.join("nord.json"), format!("{} ", nord("#3B4252"))).unwrap();
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    vcx.run_until_parked();
    assert_eq!(count(&mut vcx), 0, "no repeat notification for bad.json");

    // (b) the active user theme's file is deleted: back to Dark.
    shell.update(&mut vcx, |this, _| {
        this.theme = crate::theme::ThemeChoice::User("Nord".into())
    });
    std::fs::remove_file(dir.join("nord.json")).unwrap();
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    vcx.run_until_parked();
    assert_eq!(
        shell.read_with(&vcx, |this, _| this.theme.clone()),
        crate::theme::ThemeChoice::Dark
    );
}

/// A broken theme file present at launch is reported once, not a crash.
#[gpui_kit::test]
fn broken_theme_file_at_startup_is_reported_not_a_crash(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    let dir = crate::theme::tests::test_dir("startup-bad");
    std::fs::write(dir.join("bad.json"), "not json").unwrap();
    cx.update(gpui_kit::init);
    let (window, shell) = cx
        .update(|cx| {
            crate::shell::open_shell(
                crate::shell::Mode::Normal,
                Default::default(),
                Some(dir),
                None,
                cx,
            )
        })
        .expect("open test window");
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|window, cx| window.notifications(cx).len()),
        1,
        "one notification"
    );
    let errors = shell.read_with(&vcx, |this, _| this.user_theme_errors.clone());
    assert!(
        matches!(&errors[..], [e] if e.starts_with("Couldn't load theme bad.json: ")),
        "{errors:?}"
    );
}

/// config.toml applies at start, and Ctrl+K Ctrl+T writes the new theme to it.
#[gpui_kit::test]
fn the_saved_settings_apply_at_start_and_a_theme_change_writes_config_toml(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::theme::ThemeChoice;
    let dir = crate::theme::tests::test_dir("config-toml");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "version = 1\ntheme = \"light\"\nui_scale = 110\nconsole_font_px = 15.0\n",
    )
    .unwrap();
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
    let (theme, mono) = shell.read_with(&vcx, |this, _| (this.theme.clone(), this.console_mono));
    assert_eq!(
        (theme, mono, crate::theme::ui_scale()),
        (ThemeChoice::Light, gpui_kit::px(15.), 110)
    );

    vcx.dispatch_action(crate::CycleTheme);
    vcx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "version = 1\ntheme = \"system\"\nui_scale = 110\nconsole_font_px = 15.0\n"
    );
    crate::theme::set_ui_scale(crate::theme::DEFAULT_UI_SCALE, &mut vcx.app.borrow_mut());
}

/// Ctrl+, opens Settings, and each setter its rows call applies and saves.
#[gpui_kit::test]
fn the_settings_dialog_opens_and_its_setters_save(cx: &mut gpui_kit::TestAppContext) {
    let dir = crate::theme::tests::test_dir("settings-dialog");
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
    vcx.dispatch_action(crate::OpenSettings);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("settings").is_some(), "dialog drawn");

    shell.update_in(&mut vcx, |this, window, cx| {
        this.set_theme(crate::theme::ThemeChoice::Light, window, cx);
        this.set_ui_scale(120, window, cx);
        this.set_console_font(gpui_kit::px(15.), window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(dir.join("config.toml")).unwrap(),
        "version = 1\ntheme = \"light\"\nui_scale = 120\nconsole_font_px = 15.0\n"
    );
    crate::theme::set_ui_scale(crate::theme::DEFAULT_UI_SCALE, &mut vcx.app.borrow_mut());
}
