//! The titlebar's workspace switcher: create, rename, switch, delete, and moving connections.

use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};

use super::super::{Mode, Shell};
use crate::model::{ConnectionId, Secret, Workspaces};
use crate::persist::{Keychain as _, Keys, MemKeychain, workspaces_from_toml, workspaces_to_toml};

/// The Normal shell on a `workspaces.toml` holding the three sample workspaces, Acme active,
/// with prod-pg's password (connection 1) and blog.db's (connection 4, Side projects) saved.
fn open_saved(
    cx: &mut TestAppContext,
) -> (
    VisualTestContext,
    Entity<Shell>,
    tempfile::TempDir,
    std::sync::Arc<MemKeychain>,
) {
    let dir = tempfile::tempdir().unwrap();
    let all = Workspaces {
        active: 1,
        list: crate::mock::workspaces(),
    };
    std::fs::write(dir.path().join("workspaces.toml"), workspaces_to_toml(&all)).unwrap();
    let keys = std::sync::Arc::new(MemKeychain::default());
    keys.set(ConnectionId(1), &Secret::new("one")).unwrap();
    keys.set(ConnectionId(4), &Secret::new("four")).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        cx.set_global(Keys::new(keys.clone()));
    });
    let root = dir.path().to_path_buf();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| {
                let first = crate::mock::workspaces().remove(0);
                let mut shell = Shell::with_driver(Mode::Normal, first, super::fake(), window, cx);
                shell.persist_workspaces(&root, window, cx);
                shell
            })
        })
        .expect("open test window")
    });
    let vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    (vcx, shell, dir, keys)
}

fn saved(dir: &tempfile::TempDir) -> Workspaces {
    workspaces_from_toml(&std::fs::read_to_string(dir.path().join("workspaces.toml")).unwrap())
        .unwrap()
}

fn connection_labels(shell: &Entity<Shell>, cx: &VisualTestContext) -> Vec<String> {
    let explorer = shell.read_with(cx, |this, _| this.explorer.clone());
    let names: Vec<String> = shell.read_with(cx, |this, cx| {
        let ws = this.workspace.read(cx);
        ws.connections.iter().map(|c| c.name.clone()).collect()
    });
    explorer.read_with(cx, |e, _| {
        e.row_labels()
            .iter()
            .map(|l| l.to_string())
            .filter(|l| names.contains(l))
            .collect()
    })
}

#[gpui_kit::test]
fn switching_shows_the_other_workspace_and_saves_it_active(cx: &mut TestAppContext) {
    let (mut cx, shell, dir, _keys) = open_saved(cx);
    assert_eq!(
        connection_labels(&shell, &cx),
        ["prod-pg", "staging-pg", "local.db"]
    );

    shell.update_in(&mut cx, |this, window, cx| {
        this.switch_workspace(2, window, cx)
    });
    cx.run_until_parked();

    let side: Vec<String> = crate::mock::workspaces()[1]
        .connections
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(connection_labels(&shell, &cx), side);
    assert_eq!(saved(&dir).active, 2);
}

#[gpui_kit::test]
fn switching_closes_the_old_workspaces_tabs(cx: &mut TestAppContext) {
    let (mut cx, shell, _dir, _keys) = open_saved(cx);
    open_console_on(&shell, 1, &mut cx);

    shell.update_in(&mut cx, |this, window, cx| {
        this.switch_workspace(2, window, cx)
    });
    cx.run_until_parked();

    assert_eq!(tabs_on(&shell, 1, &cx), 0);
}

#[gpui_kit::test]
fn a_new_workspace_is_empty_active_and_saved(cx: &mut TestAppContext) {
    let (mut cx, shell, dir, _keys) = open_saved(cx);

    shell.update_in(&mut cx, |this, window, cx| {
        this.new_workspace("Fresh".into(), window, cx)
    });
    cx.run_until_parked();

    assert!(connection_labels(&shell, &cx).is_empty());
    let all = saved(&dir);
    let fresh = all.list.last().unwrap();
    assert_eq!((fresh.name.as_str(), fresh.id), ("Fresh", 4));
    assert_eq!(all.active, 4);
}

#[gpui_kit::test]
fn renaming_saves_the_new_name(cx: &mut TestAppContext) {
    let (mut cx, shell, dir, _keys) = open_saved(cx);

    shell.update_in(&mut cx, |this, window, cx| {
        this.rename_workspace("Acme Corp".into(), window, cx)
    });
    cx.run_until_parked();

    assert_eq!(saved(&dir).list[0].name, "Acme Corp");
    assert_eq!(
        shell.read_with(&cx, |this, cx| this.workspace.read(cx).name.clone()),
        "Acme Corp"
    );
}

#[gpui_kit::test]
fn deleting_removes_it_and_its_passwords_then_shows_another(cx: &mut TestAppContext) {
    let (mut cx, shell, dir, keys) = open_saved(cx);

    let deleted = shell.update_in(&mut cx, |this, window, cx| {
        this.delete_workspace(window, cx)
    });
    cx.run_until_parked();

    assert!(deleted);
    let all = saved(&dir);
    assert_eq!(all.list.iter().map(|w| w.id).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(all.active, 2);
    assert_eq!(keys.get(ConnectionId(1)).unwrap(), None);
    assert!(keys.get(ConnectionId(4)).unwrap().is_some());
    assert!(!connection_labels(&shell, &cx).contains(&"prod-pg".to_string()));
}

#[gpui_kit::test]
fn the_last_workspace_cannot_be_deleted(cx: &mut TestAppContext) {
    let (mut cx, shell, dir, _keys) = open_saved(cx);
    for _ in 0..2 {
        shell.update_in(&mut cx, |this, window, cx| {
            this.delete_workspace(window, cx)
        });
    }
    cx.run_until_parked();

    let deleted = shell.update_in(&mut cx, |this, window, cx| {
        this.delete_workspace(window, cx)
    });
    cx.run_until_parked();

    assert!(!deleted);
    assert_eq!(saved(&dir).list.len(), 1);
}

#[gpui_kit::test]
fn moving_a_connection_keeps_its_id_and_closes_its_tabs(cx: &mut TestAppContext) {
    let (mut cx, shell, dir, keys) = open_saved(cx);
    open_console_on(&shell, 1, &mut cx);

    shell.update_in(&mut cx, |this, window, cx| {
        this.move_connection(ConnectionId(1), 2, window, cx)
    });
    cx.run_until_parked();

    assert_eq!(tabs_on(&shell, 1, &cx), 0);
    assert!(!connection_labels(&shell, &cx).contains(&"prod-pg".to_string()));
    let all = saved(&dir);
    let side = &all.list[1];
    let moved = side
        .connections
        .iter()
        .find(|c| c.name == "prod-pg")
        .unwrap();
    assert_eq!(moved.id, ConnectionId(1));
    assert!(
        all.list[0]
            .connections
            .iter()
            .all(|c| c.id != ConnectionId(1))
    );
    assert!(keys.get(ConnectionId(1)).unwrap().is_some());
}

fn open_console_on(shell: &Entity<Shell>, id: u32, cx: &mut VisualTestContext) {
    shell.update_in(cx, |this, window, cx| {
        let conn = this
            .workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| c.id == ConnectionId(id))
            .unwrap()
            .clone();
        this.tabs.open(super::super::start::console_tab(
            "console-moved",
            "console-moved",
            Some(super::super::tabs::Origin::of(&conn)),
            crate::ui::console::Target {
                workspace: this.workspace.clone(),
                driver: this.driver.clone(),
                connection: Some(conn.id),
                database: conn.params.database(),
                schema: None,
            },
            "",
            window,
            cx,
        ));
        this.tab_changed(cx);
    });
    cx.run_until_parked();
    assert_eq!(tabs_on(shell, id, cx), 1);
}

fn tabs_on(shell: &Entity<Shell>, id: u32, cx: &VisualTestContext) -> usize {
    shell.read_with(cx, |this, _| {
        this.tabs
            .tabs
            .iter()
            .filter(|t| {
                t.origin
                    .as_ref()
                    .is_some_and(|o| o.connection == ConnectionId(id))
            })
            .count()
    })
}

#[gpui_kit::test]
fn switching_to_the_shown_workspace_keeps_its_tabs(cx: &mut TestAppContext) {
    let (mut cx, shell, _dir, _keys) = open_saved(cx);
    open_console_on(&shell, 1, &mut cx);

    shell.update_in(&mut cx, |this, window, cx| {
        this.switch_workspace(1, window, cx)
    });
    cx.run_until_parked();

    assert_eq!(tabs_on(&shell, 1, &cx), 1);
}

#[gpui_kit::test]
fn a_connection_added_after_a_move_gets_a_new_id(cx: &mut TestAppContext) {
    let (mut cx, shell, _dir, _keys) = open_saved(cx);
    let add = |shell: &Entity<Shell>, cx: &mut VisualTestContext| {
        shell.update_in(cx, |this, _, cx| {
            this.workspace.update(cx, |ws, cx| {
                cx.notify();
                ws.add(
                    "x.db".into(),
                    0,
                    false,
                    crate::model::ConnectionParams::Sqlite {
                        path: "x.db".into(),
                    },
                )
            })
        })
    };
    let added = add(&shell, &mut cx);
    shell.update_in(&mut cx, |this, window, cx| {
        this.move_connection(added, 2, window, cx)
    });
    cx.run_until_parked();

    assert_ne!(add(&shell, &mut cx), added);
}
