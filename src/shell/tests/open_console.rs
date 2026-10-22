use super::support::*;
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
    shell.read_with(cx, |this, cx| {
        match this.tabs.active().map(|t| &t.content) {
            Some(crate::shell::TabContent::Console(console)) => console.read(cx).editor.clone(),
            _ => panic!("a console tab is active"),
        }
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

#[gpui_kit::test]
fn each_console_runs_on_its_own_session_and_logs_to_the_panel(cx: &mut gpui_kit::TestAppContext) {
    use crate::db::Opened;
    let (fake, window, shell) = open_with_fake(cx);
    cx.update(crate::ui::console::bind_keys);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let connects = |fake: &crate::db::fake::Fake| {
        fake.pending()
            .iter()
            .filter(|l| *l == "connect app_db")
            .count()
    };
    for key in ["1/app_db/public", "1/app_db"] {
        open_console(&mut cx, &shell, key);
        let before = connects(&fake);
        cx.simulate_input("select 1");
        cx.simulate_keystrokes("ctrl-enter");
        cx.run_until_parked();
        assert_eq!(
            connects(&fake),
            before + 1,
            "{key} connects its own session"
        );
    }
    // The console opened on a schema sets its search_path before running.
    for opens in [2, 1] {
        fake.resolve("connect app_db", Ok(crate::db::fake::session("app_db")));
        cx.run_until_parked();
        for _ in 0..opens {
            fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
            cx.run_until_parked();
        }
    }
    assert_eq!(fake.opened()[0], "SET search_path TO \"public\"");
    let logged = shell.read_with(&cx, |this, cx| {
        let log = this.bottom_panel.read(cx).logged();
        log[log.len() - 2..]
            .iter()
            .map(|e| (e.sql.clone(), e.source))
            .collect::<Vec<_>>()
    });
    let entry = ("select 1".to_string(), crate::model::QuerySource::Console);
    assert_eq!(logged, [entry.clone(), entry]);
}

#[gpui_kit::test]
fn closing_a_console_tab_closes_its_session(cx: &mut gpui_kit::TestAppContext) {
    use crate::db::Opened;
    let (fake, window, shell) = open_with_fake(cx);
    cx.update(crate::ui::console::bind_keys);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    open_console(&mut cx, &shell, "1/app_db");
    cx.simulate_input("select 1");
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    fake.resolve("connect app_db", Ok(crate::db::fake::session("app_db")));
    cx.run_until_parked();
    fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
    cx.run_until_parked();
    cx.dispatch_action(crate::CloseTab);
    cx.run_until_parked();
    click(&mut cx, "tab-discard-confirm");
    assert!(fake.pending().contains(&"close app_db".to_string()));
}

#[gpui_kit::test]
fn deleting_a_connection_closes_its_consoles_sessions(cx: &mut gpui_kit::TestAppContext) {
    use crate::db::Opened;
    let (fake, window, shell) = open_with_fake(cx);
    cx.update(crate::ui::console::bind_keys);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    open_console(&mut cx, &shell, "1/app_db");
    cx.simulate_input("select 1");
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    fake.resolve("connect app_db", Ok(crate::db::fake::session("app_db")));
    cx.run_until_parked();
    fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
    cx.run_until_parked();
    let (explorer, id) = shell.read_with(&cx, |this, cx| {
        (
            this.explorer.clone(),
            this.workspace.read(cx).connections[0].id,
        )
    });
    explorer.update(&mut cx, |_, cx| {
        cx.emit(crate::ui::explorer::ConnectionDeleted { id })
    });
    cx.run_until_parked();
    assert!(fake.pending().contains(&"close app_db".to_string()));
}

#[gpui_kit::test]
fn picking_a_schema_moves_the_tab_origin_and_title_but_keeps_its_key(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let name = shell.read_with(&cx, |this, cx| {
        this.workspace.read(cx).connections[0].name.clone()
    });
    open_console(&mut cx, &shell, "1/app_db/public");
    let console = shell.read_with(&cx, |this, _| {
        match this.tabs.active().map(|t| &t.content) {
            Some(crate::shell::TabContent::Console(console)) => console.clone(),
            _ => panic!("a console tab is active"),
        }
    });
    console.update(&mut cx, |c, cx| c.pick_schema("sales".into(), cx));
    cx.run_until_parked();
    let tab = shell.read_with(&cx, |this, _| {
        let tab = this.tabs.active().expect("a tab is active");
        let path = tab.origin.as_ref().map(|o| o.path.clone());
        (tab.key.clone(), tab.title.clone(), path)
    });
    assert_eq!(
        tab,
        (
            "console:1/app_db/public".into(),
            "app_db › sales — console".into(),
            Some(format!("{name} › app_db › sales").into()),
        )
    );
}

/// A console on app_db whose text autosaves under a temp config dir, and its file.
fn saved_console(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<crate::shell::Shell>,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    shell.update(&mut cx, |this, _| this.sessions_root = Some(root));
    open_console(&mut cx, &shell, "1/app_db");
    let file = shell.read_with(&cx, |this, cx| {
        match this.tabs.active().map(|t| &t.content) {
            Some(crate::shell::TabContent::Console(c)) => c.read(cx).file().unwrap().to_path_buf(),
            _ => panic!("a console tab is active"),
        }
    });
    assert_eq!(file.parent(), Some(dir.path().join("consoles").as_path()));
    (cx, shell, dir, file)
}

fn console_count(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cx: &gpui_kit::VisualTestContext,
) -> usize {
    shell.read_with(cx, |this, _| {
        let consoles = this.tabs.tabs.iter();
        consoles
            .filter(|t| matches!(t.content, crate::shell::TabContent::Console(_)))
            .count()
    })
}

#[gpui_kit::test]
fn closing_an_empty_console_deletes_its_file_without_asking(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, shell, _dir, file) = saved_console(cx);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "").unwrap();
    let before = console_count(&shell, &cx);
    cx.dispatch_action(crate::CloseTab);
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("tab-discard-title").is_none(),
        "no question"
    );
    assert_eq!(console_count(&shell, &cx), before - 1);
    assert!(!file.exists());
}

#[gpui_kit::test]
fn closing_a_console_with_text_asks_and_discard_deletes_its_file(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, shell, _dir, file) = saved_console(cx);
    cx.simulate_input("select 1");
    cx.executor().advance_clock(crate::ui::console::SAVE_DELAY);
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "select 1");
    let before = console_count(&shell, &cx);
    cx.dispatch_action(crate::CloseTab);
    cx.run_until_parked();
    assert!(cx.debug_bounds("tab-discard-title").is_some(), "asks first");
    click(&mut cx, "tab-discard-cancel");
    assert_eq!(console_count(&shell, &cx), before, "Cancel keeps it");
    assert!(file.exists());
    cx.dispatch_action(crate::CloseTab);
    cx.run_until_parked();
    click(&mut cx, "tab-discard-confirm");
    assert_eq!(console_count(&shell, &cx), before - 1);
    assert!(!file.exists());
}

#[gpui_kit::test]
fn ctrl_shift_n_opens_another_console_on_the_same_schema_each_time(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_plain(cx);
    cx.update(|cx| crate::keymap::bind_defaults(cx, |action, _| action == "queryx::NewConsole"));
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    open_console(&mut cx, &shell, "1/app_db/public");
    let before = console_count(&shell, &cx);
    let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
    for _ in 0..2 {
        explorer.update_in(&mut cx, |e, window, cx| {
            e.reveal("1/app_db/public", window, cx)
        });
        cx.simulate_keystrokes("ctrl-shift-n");
        cx.run_until_parked();
    }
    let consoles = shell.read_with(&cx, |this, cx| {
        this.tabs
            .tabs
            .iter()
            .filter_map(|t| match &t.content {
                crate::shell::TabContent::Console(c) => {
                    let target = c.read(cx).target();
                    Some((
                        t.key.clone(),
                        target.database.clone(),
                        target.schema.clone(),
                    ))
                }
                _ => None,
            })
            .filter(|(_, db, schema)| db == "app_db" && schema.as_deref() == Some("public"))
            .collect::<Vec<_>>()
    });
    assert_eq!(console_count(&shell, &cx), before + 2, "two new consoles");
    assert_eq!(consoles.len(), 3, "all on app_db › public");
    let keys: std::collections::HashSet<_> = consoles.iter().map(|c| c.0.clone()).collect();
    assert_eq!(keys.len(), 3, "each with its own key");
}
