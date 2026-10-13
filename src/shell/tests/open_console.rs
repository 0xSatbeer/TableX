use super::*;

/// Reveals the tree row keyed `key` and picks Open console on it.
fn open_console(
    cx: &mut gpui_kit::VisualTestContext,
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    key: &str,
) {
    let explorer = shell.read_with(cx, |this, _| this.explorer.clone());
    explorer.update_in(cx, |e, window, cx| e.reveal(key, window, cx));
    cx.run_until_parked();
    cx.dispatch_action(crate::OpenConsole);
    cx.run_until_parked();
}

fn active_console(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cx: &gpui_kit::VisualTestContext,
) -> gpui_kit::Entity<gpui_kit::component::input::EditorState> {
    shell.read_with(cx, |this, _| match this.tabs.active().map(|t| &t.content) {
        Some(crate::shell::TabContent::Console(editor)) => editor.clone(),
        _ => panic!("a console tab is active"),
    })
}

#[gpui_kit::test]
fn open_console_on_a_schema_opens_its_tab_once_and_then_activates_it(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let conn = shell.read_with(&cx, |this, cx| {
        this.workspace.read(cx).connections[0].clone()
    });
    open_console(&mut cx, &shell, "1/app_db/public");
    let (count, tab) = shell.read_with(&cx, |this, _| {
        let tab = this.tabs.active().expect("a tab is active");
        (
            this.tabs.tabs.len(),
            (tab.key.clone(), tab.title.clone(), tab.origin.clone()),
        )
    });
    assert_eq!(
        tab,
        (
            "console:1/app_db/public".into(),
            "app_db — console".into(),
            Some(crate::shell::tabs::Origin {
                connection: conn.id,
                color: conn.color,
                production: conn.is_production,
                path: format!("{} › app_db › public", conn.name).into(),
                tail: "app_db › public".into(),
            }),
        )
    );
    shell.update(&mut cx, |this, cx| {
        this.tabs.activate(0);
        this.tab_changed(cx);
    });
    open_console(&mut cx, &shell, "1/app_db/public");
    let again = shell.read_with(&cx, |this, _| {
        (
            this.tabs.tabs.len(),
            this.tabs.active().map(|t| t.key.clone()),
        )
    });
    assert_eq!(
        again,
        (count, Some("console:1/app_db/public".into())),
        "the same tab is activated, no second one opened"
    );
}

#[gpui_kit::test]
fn two_consoles_keep_separate_editor_text(cx: &mut gpui_kit::TestAppContext) {
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    open_console(&mut cx, &shell, "1/app_db/public");
    let schema = active_console(&shell, &cx);
    cx.simulate_input("select 1");
    open_console(&mut cx, &shell, "1/app_db");
    let database = active_console(&shell, &cx);
    cx.simulate_input("select 2");
    let texts = cx.update(|_, cx| [schema, database].map(|e| e.read(cx).value().to_string()));
    assert_eq!(texts, ["select 1", "select 2"]);
}
