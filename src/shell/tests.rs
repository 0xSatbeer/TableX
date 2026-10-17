mod add_form;
mod add_save;
mod close_transaction;
mod connect;
mod connection_test;
mod connection_test_outcome;
mod delete_cleanup;
mod delete_dialog;
mod delete_selection;
mod edit_form;
mod edit_sessions;
mod focus;
mod layout;
mod maximize;
mod open_console;
mod overflow;
mod races;
mod rename;
mod startup;
mod support;
mod table_data;
mod themes;
mod workspaces;

/// A driver whose calls wait until the test answers them, so no test reaches a real database.
fn fake() -> std::sync::Arc<dyn crate::db::Driver> {
    std::sync::Arc::new(crate::db::fake::Fake::default())
}

/// The Shell as `--demo` starts it, on the sample workspace with its trees already loaded,
/// reading through `driver`.
fn demo_shell(
    driver: std::sync::Arc<dyn crate::db::Driver>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<super::Shell>,
) -> super::Shell {
    super::Shell::with_driver(
        super::Mode::Demo,
        crate::mock::workspaces().remove(0),
        driver,
        window,
        cx,
    )
}

/// The Shell with a table tab from prod-pg opened from the tree, the console on prod-pg it
/// starts with, and a console on local.db.
fn open_with_connection_tabs(
    cx: &mut gpui_kit::TestAppContext,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<super::Shell>) {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    let explorer = shell.read_with(&vcx, |this, _| this.explorer.clone());
    explorer.update_in(&mut vcx, |e, window, cx| {
        e.reveal("1/app_db/public/table/users", window, cx)
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    shell.update_in(&mut vcx, |this, window, cx| {
        let local = this.workspace.read(cx).connections[2].clone();
        this.tabs.open(super::start::console_tab(
            "console-local",
            "console-local",
            Some(super::tabs::Origin::of(&local)),
            crate::ui::console::Target {
                workspace: this.workspace.clone(),
                driver: this.driver.clone(),
                connection: Some(local.id),
                database: local.params.database(),
                schema: None,
            },
            "",
            window,
            cx,
        ));
        this.tab_changed(cx);
    });
    vcx.run_until_parked();
    let prod_tabs = shell.read_with(&vcx, |this, _| {
        this.tabs
            .tabs
            .iter()
            .filter(|t| {
                t.origin
                    .as_ref()
                    .is_some_and(|o| o.connection == crate::model::ConnectionId(1))
            })
            .count()
    });
    assert_eq!(prod_tabs, 2, "a table tab and the console on prod-pg");
    (window, shell)
}

fn tab_origins(
    shell: &gpui_kit::Entity<super::Shell>,
    cx: &gpui_kit::VisualTestContext,
) -> Vec<(gpui_kit::SharedString, super::tabs::Origin)> {
    shell.read_with(cx, |this, _| {
        this.tabs
            .tabs
            .iter()
            .filter_map(|t| Some((t.key.clone(), t.origin.clone()?)))
            .collect()
    })
}

/// The Shell with the mock workspace, parked. Motion is reduced so a dialog lands in place
/// at once; mid-slide, a click's mouse-up would hit a different spot than its mouse-down.
fn open_plain(
    cx: &mut gpui_kit::TestAppContext,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<super::Shell>) {
    open_plain_with(cx, fake())
}

/// `open_plain` with the explorer reading through `driver`.
fn open_plain_with(
    cx: &mut gpui_kit::TestAppContext,
    driver: std::sync::Arc<dyn crate::db::Driver>,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<super::Shell>) {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(driver, window, cx))
        })
        .expect("open test window")
    });
    gpui_kit::VisualTestContext::from_window(window, cx).run_until_parked();
    (window, shell)
}

/// The Shell as the app starts it in `mode`, parked.
fn open_started(
    cx: &mut gpui_kit::TestAppContext,
    mode: super::Mode,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<super::Shell>) {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| super::Shell::new(mode, window, cx))
        })
        .expect("open test window")
    });
    gpui_kit::VisualTestContext::from_window(window, cx).run_until_parked();
    (window, shell)
}

/// The editor of the `console-1` tab the Shell starts with.
fn console_1(
    this: &super::Shell,
    cx: &gpui_kit::App,
) -> gpui_kit::Entity<gpui_kit::component::input::EditorState> {
    match this
        .tabs
        .tabs
        .iter()
        .find(|t| t.key == "console-1")
        .map(|t| &t.content)
    {
        Some(super::TabContent::Console(console)) => console.read(cx).editor.clone(),
        _ => panic!("the console-1 tab"),
    }
}
