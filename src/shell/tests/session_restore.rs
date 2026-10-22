//! Restoring a workspace's tabs, layout and expanded tree from `sessions/<id>.toml`.

use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px};

use super::super::{Mode, Shell, TabContent};
use crate::db::fake::{self, Fake};
use crate::model::{ConnectionId, DbObject, ObjectKind, Secret, Workspaces};
use crate::persist::{
    Keychain as _, Keys, MemKeychain, SessionFile, SessionTab, load_session, save_session,
    workspaces_to_toml,
};

/// The Normal shell on the sample workspaces (Acme active: prod-pg 1 production, staging-pg 2,
/// local.db 3) with `session` saved for Acme, restored at start.
fn open_restored(
    cx: &mut TestAppContext,
    session: &SessionFile,
) -> (
    VisualTestContext,
    Entity<Shell>,
    tempfile::TempDir,
    std::sync::Arc<Fake>,
) {
    open_restored_in(cx, session, tempfile::tempdir().unwrap())
}

/// `open_restored` with config dir `dir`.
fn open_restored_in(
    cx: &mut TestAppContext,
    session: &SessionFile,
    dir: tempfile::TempDir,
) -> (
    VisualTestContext,
    Entity<Shell>,
    tempfile::TempDir,
    std::sync::Arc<Fake>,
) {
    let all = Workspaces {
        active: 1,
        list: crate::mock::workspaces(),
    };
    std::fs::write(dir.path().join("workspaces.toml"), workspaces_to_toml(&all)).unwrap();
    save_session(dir.path(), 1, session).unwrap();
    let keys = std::sync::Arc::new(MemKeychain::default());
    keys.set(ConnectionId(1), &Secret::new("one")).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        cx.set_global(Keys::new(keys));
    });
    let fake = std::sync::Arc::new(Fake::default());
    let (root, driver) = (dir.path().to_path_buf(), fake.clone());
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| {
                // Empty until the file loads, as the app starts.
                let empty = crate::model::Workspace {
                    id: 1,
                    name: "Workspace".into(),
                    connections: Vec::new(),
                    id_floor: 0,
                };
                let mut shell = Shell::with_driver(Mode::Normal, empty, driver, window, cx);
                shell.persist_workspaces(&root, window, cx);
                shell.persist_session(&root, window, cx);
                shell
            })
        })
        .expect("open test window")
    });
    let vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    (vcx, shell, dir, fake)
}

fn table(connection: u32, database: &str, schema: &str, object: &str) -> SessionTab {
    SessionTab::Table {
        connection,
        database: database.into(),
        schema: schema.into(),
        object: object.into(),
        where_: String::new(),
        order_by: String::new(),
        pinned: false,
    }
}

fn three_tabs() -> SessionFile {
    SessionFile {
        active_tab: Some(1),
        explorer_visible: false,
        explorer_width: 300.0,
        panel_visible: true,
        panel_height: 150.0,
        expanded: vec!["2".into(), "2/app_db".into()],
        tabs: vec![
            SessionTab::Console {
                connection: 3,
                database: "local.db".into(),
                schema: String::new(),
                console: 4,
                autocommit: true,
                pinned: true,
            },
            SessionTab::Table {
                connection: 2,
                database: "app_db".into(),
                schema: "public".into(),
                object: "users".into(),
                where_: "id > 10".into(),
                order_by: "id DESC".into(),
                pinned: false,
            },
            SessionTab::Console {
                connection: 2,
                database: "app_db".into(),
                schema: "public".into(),
                console: 7,
                autocommit: false,
                pinned: false,
            },
        ],
        ..SessionFile::default()
    }
}

#[gpui_kit::test]
fn saving_then_restoring_gives_the_same_tabs_and_layout(cx: &mut TestAppContext) {
    let saved = three_tabs();
    let (mut cx, shell, dir, _fake) = open_restored(cx, &saved);

    shell.read_with(&cx, |this, cx| {
        let titles: Vec<_> = this.tabs.tabs.iter().map(|t| t.title.to_string()).collect();
        assert_eq!(titles, ["local.db — console", "users", "app_db — console"]);
        let pins: Vec<_> = this.tabs.tabs.iter().map(|t| t.pinned).collect();
        assert_eq!(pins, [true, false, false]);
        assert_eq!(this.tabs.active, 1);
        assert!(!this.layout.explorer_visible && this.layout.bottom_visible);
        assert_eq!(this.layout.explorer.size, px(300.));
        assert_eq!(this.layout.bottom.size, px(150.));
        let TabContent::Grid(grid, _) = &this.tabs.tabs[1].content else {
            panic!("a table tab")
        };
        let (_, where_, order_by) = grid.read(cx).source().expect("a live table");
        assert_eq!((where_, order_by), ("id > 10", "id DESC"));
        let TabContent::Console(console) = &this.tabs.tabs[2].content else {
            panic!("a console tab")
        };
        assert!(!console.read(cx).autocommit());
    });

    // Saving what was restored writes the same file back.
    shell.update(&mut cx, |this, cx| this.save_session(cx));
    assert_eq!(load_session(dir.path(), 1).0, saved);

    // A layout change is saved on its own, 500 ms later.
    shell.update(&mut cx, |this, cx| {
        this.layout.toggle_explorer();
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!load_session(dir.path(), 1).0.explorer_visible);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    assert!(load_session(dir.path(), 1).0.explorer_visible);
}

#[gpui_kit::test]
fn tabs_of_deleted_connections_are_dropped_and_a_bad_file_means_no_tabs(cx: &mut TestAppContext) {
    let session = SessionFile {
        active_tab: Some(0),
        tabs: vec![
            table(99, "x", "public", "gone"),
            table(3, "main", "main", "t"),
        ],
        ..SessionFile::default()
    };
    let (cx, shell, _dir, _fake) = open_restored(cx, &session);
    let titles: Vec<_> = shell.read_with(&cx, |this, _| {
        this.tabs.tabs.iter().map(|t| t.title.to_string()).collect()
    });
    assert_eq!(titles, ["t"]);

    let dir = tempfile::tempdir().unwrap();
    let path = crate::persist::session_path(dir.path(), 1);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "version = [").unwrap();
    let (session, notice) = load_session(dir.path(), 1);
    assert!(session.tabs.is_empty() && notice.is_some());
}

#[gpui_kit::test]
fn production_tabs_wait_for_a_click_while_others_connect_on_launch(cx: &mut TestAppContext) {
    let session = SessionFile {
        active_tab: Some(0),
        tabs: vec![
            table(1, "app_db", "public", "users"),
            table(3, "main", "main", "notes"),
        ],
        ..SessionFile::default()
    };
    let (mut cx, _shell, _dir, fake) = open_restored(cx, &session);

    let pending = fake.pending();
    assert!(
        pending.iter().any(|l| l == "connect local.db"),
        "{pending:?}"
    );
    assert!(!pending.iter().any(|l| l.contains("app_db")), "{pending:?}");
    assert!(cx.debug_bounds("table-connect").is_some());

    super::support::click(&mut cx, "table-connect");
    assert!(
        fake.pending().iter().any(|l| l == "connect app_db"),
        "{:?}",
        fake.pending()
    );
}

fn object(name: &str) -> DbObject {
    DbObject {
        kind: ObjectKind::Table,
        name: name.into(),
        columns: None,
        keys: Vec::new(),
        indexes: Vec::new(),
        row_estimate: None,
    }
}

#[gpui_kit::test]
fn expanded_nodes_restore_by_name_when_the_order_changes(cx: &mut TestAppContext) {
    let session = SessionFile {
        active_tab: Some(0),
        expanded: vec![
            "3".into(),
            "3/main".into(),
            "3/main/main".into(),
            "3/main/main/Tables".into(),
        ],
        tabs: vec![table(3, "main", "main", "notes")],
        ..SessionFile::default()
    };
    let (mut cx, shell, dir, fake) = open_restored(cx, &session);

    // The grid's session first, then the tree's.
    fake.resolve("connect local.db", Ok(fake::session("local.db")));
    fake.resolve("connect local.db", Ok(fake::session("local.db")));
    cx.run_until_parked();
    fake.resolve(
        "databases local.db",
        Ok(vec!["aux".to_string(), "main".to_string()]),
    );
    cx.run_until_parked();
    // Database "main" gets a session of its own.
    fake.resolve("connect local.db", Ok(fake::session("local.db")));
    cx.run_until_parked();
    fake.resolve(
        "schemas local.db",
        Ok(vec!["temp".to_string(), "main".to_string()]),
    );
    cx.run_until_parked();
    fake.resolve(
        "objects local.db/main",
        Ok(vec![object("zeta"), object("notes"), object("alpha")]),
    );
    cx.run_until_parked();

    let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
    let labels: Vec<String> = explorer.read_with(&cx, |e, _| {
        e.row_labels().iter().map(|l| l.to_string()).collect()
    });
    let at = labels.iter().position(|l| l == "local.db").unwrap();
    assert_eq!(
        labels[at..],
        [
            "local.db", "aux", "main", "temp", "main", "Tables", "zeta", "notes", "alpha"
        ]
    );

    shell.update(&mut cx, |this, cx| this.save_session(cx));
    assert_eq!(load_session(dir.path(), 1).0.expanded, session.expanded);
}

fn titles(shell: &Entity<Shell>, cx: &VisualTestContext) -> Vec<String> {
    shell.read_with(cx, |this, _| {
        this.tabs.tabs.iter().map(|t| t.title.to_string()).collect()
    })
}

#[gpui_kit::test]
fn switching_workspace_saves_one_session_and_restores_the_other(cx: &mut TestAppContext) {
    let acme = three_tabs();
    let (mut cx, shell, dir, _fake) = open_restored(cx, &acme);
    let side = SessionFile {
        active_tab: Some(0),
        explorer_width: 222.0,
        expanded: vec!["5".into(), "5/main".into()],
        tabs: vec![table(5, "main", "main", "notes")],
        ..SessionFile::default()
    };
    save_session(dir.path(), 2, &side).unwrap();
    // A change still waiting on its debounce is saved by the switch itself.
    shell.update(&mut cx, |this, cx| {
        this.layout.toggle_bottom();
        cx.notify();
    });
    cx.run_until_parked();

    shell.update_in(&mut cx, |this, window, cx| {
        this.switch_workspace(2, window, cx)
    });
    cx.run_until_parked();

    let acme_saved = load_session(dir.path(), 1).0;
    assert_eq!(acme_saved.tabs, acme.tabs);
    assert!(!acme_saved.panel_visible);
    assert_eq!(titles(&shell, &cx), ["notes"]);
    shell.read_with(&cx, |this, cx| {
        assert_eq!(this.layout.explorer.size, px(222.));
        assert!(this.layout.bottom_visible);
        assert_eq!(this.explorer.read(cx).expanded_paths(), side.expanded);
    });
    // The switch didn't write Acme's tabs over workspace 2's file.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(load_session(dir.path(), 2).0.tabs, side.tabs);

    shell.update_in(&mut cx, |this, window, cx| {
        this.switch_workspace(1, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        titles(&shell, &cx),
        ["local.db — console", "users", "app_db — console"]
    );
    assert_eq!(load_session(dir.path(), 2).0.tabs, side.tabs);

    shell.update_in(&mut cx, |this, window, cx| {
        assert!(this.delete_workspace(window, cx));
    });
    cx.run_until_parked();
    assert!(!crate::persist::session_path(dir.path(), 1).exists());
    assert_eq!(titles(&shell, &cx), ["notes"]);
}

#[gpui_kit::test]
fn two_consoles_on_the_same_schema_restore_as_two_tabs_each_with_its_own_text(
    cx: &mut TestAppContext,
) {
    let console = |id| SessionTab::Console {
        connection: 2,
        database: "app_db".into(),
        schema: "public".into(),
        console: id,
        autocommit: true,
        pinned: false,
    };
    let saved = SessionFile {
        active_tab: Some(0),
        tabs: vec![console(7), console(8)],
        ..SessionFile::default()
    };
    // The consoles' files are written before the shell starts, as a previous run left them.
    let dir = tempfile::tempdir().unwrap();
    let consoles = dir.path().join("consoles");
    std::fs::create_dir_all(&consoles).unwrap();
    std::fs::write(consoles.join("7.sql"), "select 7").unwrap();
    std::fs::write(consoles.join("8.sql"), "select 8").unwrap();
    let (mut cx, shell, root, _fake) = open_restored_in(cx, &saved, dir);

    let tabs = shell.read_with(&cx, |this, cx| {
        this.tabs
            .tabs
            .iter()
            .map(|t| match &t.content {
                TabContent::Console(c) => (t.key.to_string(), c.read(cx).text(cx)),
                _ => panic!("a console tab"),
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(tabs.len(), 2);
    assert_ne!(tabs[0].0, tabs[1].0, "each keeps its own key");
    assert_eq!([&*tabs[0].1, &*tabs[1].1], ["select 7", "select 8"]);
    shell.update(&mut cx, |this, cx| this.save_session(cx));
    assert_eq!(
        load_session(root.path(), 1).0,
        saved,
        "each keeps its own file"
    );
}
