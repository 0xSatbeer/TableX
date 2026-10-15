use super::support::*;

/// The shell with a manual console on prod-pg, made non-production so runs don't ask, whose transaction is open, its tab active.
fn open_transaction(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    std::sync::Arc<crate::db::fake::Fake>,
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<crate::shell::Shell>,
) {
    let (fake, window, shell) = open_with_fake(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.update(|_, cx| crate::ui::console::bind_keys(cx));
    let driver: std::sync::Arc<dyn crate::db::Driver> = fake.clone();
    let editor = shell.update_in(&mut cx, |this, window, cx| {
        this.workspace
            .update(cx, |ws, _| ws.connections[0].is_production = false);
        let conn = this.workspace.read(cx).connections[0].clone();
        let target = crate::ui::console::Target {
            workspace: this.workspace.clone(),
            driver,
            connection: Some(conn.id),
            database: conn.params.database(),
        };
        let tab = crate::shell::start::console_tab(
            "console-tx",
            "console-tx",
            Some(crate::shell::tabs::Origin::of(&conn)),
            target,
            "update t set a = 1",
            window,
            cx,
        );
        let crate::shell::TabContent::Console(console) = &tab.content else {
            unreachable!()
        };
        let editor = console.update(cx, |c, cx| {
            c.set_manual(true, window, cx);
            c.editor.clone()
        });
        this.tabs.open(tab);
        this.tab_changed(cx);
        editor
    });
    cx.run_until_parked();
    cx.update(|window, cx| editor.update(cx, |e, cx| e.focus(window, cx)));
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    let connect = fake.pending().pop().expect("the console connects");
    let db = connect.trim_start_matches("connect ").to_string();
    fake.resolve(&connect, Ok(crate::db::fake::session(&db)));
    cx.run_until_parked();
    for _ in 0..2 {
        fake.resolve_query(&format!("open {db}"), Ok(crate::db::Opened::Affected(1)));
        cx.run_until_parked();
    }
    assert_eq!(fake.opened(), ["BEGIN", "update t set a = 1"]);
    (fake, cx, shell)
}

fn tab_open(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cx: &gpui_kit::VisualTestContext,
) -> bool {
    shell.read_with(cx, |this, _| {
        this.tabs.tabs.iter().any(|t| t.key == "console-tx")
    })
}

#[gpui_kit::test]
fn closing_a_console_with_an_open_transaction_asks_and_confirming_rolls_back(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, shell) = open_transaction(cx);
    cx.dispatch_action(crate::CloseTab);
    cx.run_until_parked();
    assert!(cx.debug_bounds("tab-close-title").is_some(), "asks first");
    assert!(tab_open(&shell, &cx), "still open while asking");
    click(&mut cx, "tab-close-confirm");
    assert!(!tab_open(&shell, &cx));
    assert_eq!(fake.opened().last().map(String::as_str), Some("ROLLBACK"));
}

#[gpui_kit::test]
fn cancelling_the_close_keeps_the_console_and_its_transaction(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, shell) = open_transaction(cx);
    cx.dispatch_action(crate::CloseTab);
    cx.run_until_parked();
    click(&mut cx, "tab-close-cancel");
    assert!(tab_open(&shell, &cx));
    assert_eq!(fake.opened().len(), 2, "nothing sent");
}
