use super::support::*;
use super::*;

#[gpui_kit::test]
fn delete_in_the_row_menu_opens_the_confirmation_with_the_spec_title_and_buttons(
    cx: &mut gpui_kit::TestAppContext,
) {
    for row in ["prod-pg", "staging-pg", "local.db"] {
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        right_click_row(&mut cx, &shell, row);
        assert!(
            cx.debug_bounds("menu-delete").is_some(),
            "{row}: Delete in the row menu"
        );
        click(&mut cx, "menu-delete");
        assert!(
            cx.debug_bounds("dialog-layer").is_some(),
            "{row}: confirmation shown"
        );
        let title: &'static str = format!("delete-title Delete connection {row}?").leak();
        assert!(cx.debug_bounds(title).is_some(), "{row}: titled {title:?}");
        for button in ["delete-cancel", "delete-confirm"] {
            assert!(cx.debug_bounds(button).is_some(), "{row}: {button} drawn");
        }
    }
}

#[gpui_kit::test]
fn delete_connection_is_enabled_at_once_for_non_production_and_only_on_the_exact_name_for_production(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (row, typed into the name field, then press, enabled)
    let cases: [(&str, &str, &str, bool); 13] = [
        ("local.db", "", "click delete-confirm", true),
        ("staging-pg", "", "click delete-confirm", true),
        ("local.db", "", "enter", true),
        ("prod-pg", "", "click delete-confirm", false),
        ("prod-pg", "prod-p", "click delete-confirm", false),
        ("prod-pg", "Prod-pg", "click delete-confirm", false),
        ("prod-pg", " prod-pg", "click delete-confirm", false),
        ("prod-pg", "prod-pg ", "click delete-confirm", false),
        ("prod-pg", "prod-pg", "click delete-confirm", true),
        ("prod-pg", "", "enter", false),
        ("prod-pg", "prod-p", "enter", false),
        ("prod-pg", "Prod-pg", "enter", false),
        ("prod-pg", "prod-pg", "enter", true),
    ];
    for (row, typed, press, enabled) in cases {
        let label = format!("{row} {typed:?} {press}");
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        open_delete(&mut cx, &shell, row);
        assert_eq!(
            cx.debug_bounds("delete-name").is_some(),
            row == "prod-pg",
            "{label}: name field only for Production"
        );
        if !typed.is_empty() {
            cx.simulate_input(typed);
        }
        match press.strip_prefix("click ") {
            Some(selector) => click(&mut cx, selector),
            None => cx.simulate_keystrokes(press),
        }
        cx.run_until_parked();
        let kept = workspace.read_with(&cx, |ws, _| ws.connections.iter().any(|c| c.name == row));
        assert_eq!(!kept, enabled, "{label}: deleted");
        assert_eq!(
            cx.debug_bounds("dialog-layer").is_none(),
            enabled,
            "{label}: confirmation closed"
        );
    }
}

#[gpui_kit::test]
fn cancel_or_esc_in_the_delete_confirmation_leaves_the_workspace_and_tabs_unchanged(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (row, typed first, press)
    let cases: [(&str, &str, &str); 5] = [
        ("local.db", "", "escape"),
        ("local.db", "", "click delete-cancel"),
        ("prod-pg", "", "escape"),
        ("prod-pg", "prod-pg", "escape"),
        ("prod-pg", "prod-pg", "click delete-cancel"),
    ];
    for (row, typed, press) in cases {
        let label = format!("{row} {typed:?} {press}");
        let (window, shell) = open_with_connection_tabs(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        let before = (
            workspace.read_with(&cx, |ws, _| ws.clone()),
            tab_origins(&shell, &cx),
        );
        open_delete(&mut cx, &shell, row);
        if !typed.is_empty() {
            cx.simulate_input(typed);
        }
        match press.strip_prefix("click ") {
            Some(selector) => click(&mut cx, selector),
            None => cx.simulate_keystrokes(press),
        }
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("dialog-layer").is_none(),
            "{label}: confirmation closed"
        );
        assert_eq!(
            (
                workspace.read_with(&cx, |ws, _| ws.clone()),
                tab_origins(&shell, &cx)
            ),
            before,
            "{label}: workspace and tabs"
        );
    }
}

#[gpui_kit::test]
fn confirming_the_delete_of_b_removes_it_from_the_tree_and_the_panel_and_closes_only_its_tabs(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionId;
    let (window, shell) = open_with_connection_tabs(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let (workspace, explorer, panel) = shell.read_with(&cx, |this, _| {
        (
            this.workspace.clone(),
            this.explorer.clone(),
            this.bottom_panel.clone(),
        )
    });
    // Two tabs from staging-pg, one pinned, and one of them closed into the reopen history.
    shell.update_in(&mut cx, |this, window, cx| {
        let staging = this.workspace.read(cx).connections[1].clone();
        for (key, pinned) in [
            ("staging-1", false),
            ("staging-2", true),
            ("staging-3", false),
        ] {
            this.tabs
                .open(console_on(&staging, key, pinned, window, cx));
        }
        let closed = this
            .tabs
            .tabs
            .iter()
            .position(|t| t.key == "staging-3")
            .unwrap();
        this.tabs.close(closed);
        this.tab_changed(cx);
    });
    cx.run_until_parked();
    let staging = ConnectionId(2);
    let from = |id: ConnectionId, cx: &gpui_kit::VisualTestContext| {
        shell.read_with(cx, |this, _| {
            this.tabs
                .tabs
                .iter()
                .filter(|t| t.origin.as_ref().is_some_and(|o| o.connection == id))
                .map(|t| t.key.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(from(staging, &cx).len(), 2, "two tabs from staging-pg open");
    assert!(
        panel.read_with(&cx, |p, _| p.logged_connections().contains(&staging)),
        "the log has staging-pg rows"
    );
    let others: Vec<_> = shell.read_with(&cx, |this, _| {
        this.tabs
            .tabs
            .iter()
            .map(|t| t.key.clone())
            .filter(|k| !k.starts_with("staging"))
            .collect()
    });
    let prod_tabs = from(ConnectionId(1), &cx);

    open_delete(&mut cx, &shell, "staging-pg");
    click(&mut cx, "delete-confirm");

    assert!(
        cx.debug_bounds("dialog-layer").is_none(),
        "confirmation closed"
    );
    let names: Vec<String> = workspace.read_with(&cx, |ws, _| {
        ws.connections.iter().map(|c| c.name.clone()).collect()
    });
    assert_eq!(names, ["prod-pg", "local.db"], "workspace [a, c]");
    assert!(
        !explorer
            .read_with(&cx, |e, _| e.row_labels())
            .iter()
            .any(|l| l == "staging-pg"),
        "no tree row"
    );
    assert_eq!(
        panel.read_with(&cx, |p, cx| p.connection(staging, cx)),
        Default::default(),
        "the panel no longer knows it"
    );
    assert!(
        !panel.read_with(&cx, |p, _| p.logged_connections().contains(&staging)),
        "its log rows are gone"
    );
    assert!(
        from(staging, &cx).is_empty(),
        "both its tabs closed, the pinned one too"
    );
    let keys: Vec<_> = shell.read_with(&cx, |this, _| {
        this.tabs.tabs.iter().map(|t| t.key.clone()).collect()
    });
    assert_eq!(keys, others, "every other tab stays open, in order");
    assert_eq!(from(ConnectionId(1), &cx), prod_tabs, "prod-pg's tabs stay");
    shell.update(&mut cx, |this, _| this.tabs.reopen());
    assert!(
        from(staging, &cx).is_empty(),
        "reopen never brings back a deleted connection's tab"
    );
}
