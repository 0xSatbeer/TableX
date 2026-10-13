use super::support::*;
use super::*;

#[gpui_kit::test]
fn a_connect_in_flight_is_discarded_when_an_edit_changes_the_params(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::{ConnectionParams, ConnectionStatus};
    use ConnectionStatus::{Connected, Connecting, Disconnected};
    use std::path::PathBuf;
    // (new name, paths saved in turn, connect again after the edits, status once the first
    // connect finishes, status once the second one does)
    let cases: [(&str, &[&str], bool, ConnectionStatus, ConnectionStatus); 7] = [
        (
            "local.db",
            &["~/acme/other.db"],
            false,
            Disconnected,
            Disconnected,
        ),
        (
            "renamed.db",
            &["~/acme/other.db"],
            false,
            Disconnected,
            Disconnected,
        ),
        (
            "renamed.db",
            &["~/acme/local.db"],
            false,
            Connected,
            Connected,
        ),
        (
            "local.db",
            &["~/acme/local.db"],
            false,
            Connected,
            Connected,
        ),
        // Edited away and back: the params match again, but the edit still discarded it.
        (
            "local.db",
            &["~/acme/other.db", "~/acme/local.db"],
            false,
            Disconnected,
            Disconnected,
        ),
        // Only the latest connect lands, on the old params or the new.
        (
            "local.db",
            &["~/acme/other.db"],
            true,
            Connecting,
            Connected,
        ),
        (
            "local.db",
            &["~/acme/other.db", "~/acme/local.db"],
            true,
            Connecting,
            Connected,
        ),
    ];
    for (name, paths, again, first, second) in cases {
        let label = format!("{name} {paths:?} again {again}");
        let driver = std::sync::Arc::new(crate::db::fake::Fake::default());
        let (window, shell) = open_plain_with(cx, driver.clone());
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let (workspace, explorer) = shell.read_with(&cx, |this, _| {
            (this.workspace.clone(), this.explorer.clone())
        });
        explorer.update_in(&mut cx, |e, window, cx| e.reveal("3", window, cx));
        cx.run_until_parked();
        // Enter on the disconnected local.db starts a connect that lands when the test answers it.
        cx.simulate_keystrokes("enter");
        let (id, status) = workspace.read_with(&cx, |ws, _| {
            (ws.connections[2].id, ws.connections[2].status.clone())
        });
        assert_eq!(status, Connecting, "{label}: connecting");
        for path in paths {
            workspace.update(&mut cx, |ws, cx| {
                ws.edit(
                    id,
                    name.into(),
                    4,
                    false,
                    ConnectionParams::Sqlite {
                        path: PathBuf::from(path),
                    },
                );
                cx.notify();
            });
        }
        cx.run_until_parked();
        if again {
            // The edit left the row open; close it and open it to connect again.
            cx.simulate_keystrokes("left enter");
            assert_eq!(
                workspace.read_with(&cx, |ws, _| ws.connections[2].status.clone()),
                Connecting,
                "{label}: connecting again"
            );
        }
        let connects: Vec<String> = driver
            .pending()
            .into_iter()
            .filter(|l| l.starts_with("connect "))
            .collect();
        driver.resolve(&connects[0], Ok(crate::db::fake::session("local.db")));
        cx.run_until_parked();
        let conn = workspace.read_with(&cx, |ws, _| ws.connections[2].clone());
        assert_eq!(
            (conn.name.as_str(), conn.status),
            (name, first),
            "{label}: after the first connect finished"
        );
        if let Some(next) = connects.get(1) {
            driver.resolve(next, Ok(crate::db::fake::session("other.db")));
            cx.run_until_parked();
        }
        assert_eq!(
            workspace.read_with(&cx, |ws, _| ws.connections[2].status.clone()),
            second,
            "{label}: after the second connect finished"
        );
        assert_eq!(
            explorer.read_with(&cx, |e, _| e
                .row_labels()
                .iter()
                .filter(|l| l.as_ref() == name)
                .count()),
            1,
            "{label}: one tree row"
        );
    }
}

#[gpui_kit::test]
fn a_connect_in_flight_finds_its_connection_deleted_and_is_discarded(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionStatus::{self, *};
    use std::time::Duration;
    // (row connecting, key to press Enter on, then add a connection and connect it, the new one's status once the old connect finished)
    let cases: [(&str, &str, Option<bool>, Option<ConnectionStatus>); 4] = [
        ("local.db", "3", None, None),
        ("staging-pg", r"2/\error", None, None),
        // local.db has the highest id, so the new connection may get it again.
        ("local.db", "3", Some(false), Some(Disconnected)),
        ("local.db", "3", Some(true), Some(Connecting)),
    ];
    let latency = Duration::from_millis(1000);
    crate::mock::set_latency(latency.as_millis() as u32);
    for (row, key, add, want) in cases {
        let label = format!("{row} add {add:?}");
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let (workspace, explorer) = shell.read_with(&cx, |this, _| {
            (this.workspace.clone(), this.explorer.clone())
        });
        explorer.update_in(&mut cx, |e, window, cx| e.reveal(key, window, cx));
        cx.simulate_keystrokes("enter");
        let status = workspace.read_with(&cx, |ws, _| {
            ws.connections
                .iter()
                .find(|c| c.name == row)
                .unwrap()
                .status
                .clone()
        });
        assert_eq!(status, Connecting, "{label}: connecting");
        delete_row(&mut cx, &shell, row);
        // Half way through the old connect, so a new one lands later than it.
        cx.executor().advance_clock(latency / 2);
        if let Some(connect) = add {
            let dialog = open_add_dialog(&mut cx);
            finish(
                &mut cx,
                &dialog,
                &[
                    ("kind-sqlite", ""),
                    ("name", "new.db"),
                    ("path", "/tmp/new.db"),
                ],
                "enter",
            );
            if connect {
                let id = workspace.read_with(&cx, |ws, _| {
                    ws.connections
                        .iter()
                        .find(|c| c.name == "new.db")
                        .unwrap()
                        .id
                });
                explorer.update_in(&mut cx, |e, window, cx| {
                    e.reveal(&id.0.to_string(), window, cx)
                });
                cx.simulate_keystrokes("enter");
            }
        }
        let rows = explorer.read_with(&cx, |e, _| e.row_labels());
        let others = workspace.read_with(&cx, |ws, _| ws.clone());
        cx.executor().advance_clock(latency / 2);
        cx.run_until_parked();
        assert!(
            !workspace.read_with(&cx, |ws, _| ws.connections.iter().any(|c| c.name == row)),
            "{label}: stays deleted"
        );
        assert_eq!(
            explorer.read_with(&cx, |e, _| e.row_labels()),
            rows,
            "{label}: no row comes back, no error row"
        );
        if let Some(want) = want {
            let status = workspace.read_with(&cx, |ws, _| {
                ws.connections
                    .iter()
                    .find(|c| c.name == "new.db")
                    .unwrap()
                    .status
                    .clone()
            });
            assert_eq!(status, want, "{label}: the new connection");
        } else {
            assert_eq!(
                workspace.read_with(&cx, |ws, _| ws.clone()),
                others,
                "{label}: the others unchanged"
            );
        }
    }
    crate::mock::set_latency(0);
}

#[gpui_kit::test]
fn deleting_a_connection_during_an_f2_rename_leaves_no_rename_to_land_on_another_connection(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (row being renamed, row deleted, how). DeleteConnection acts on the selected row, the one being renamed.
    let cases: [(&str, &str, &str); 4] = [
        ("local.db", "local.db", "action"),
        ("staging-pg", "staging-pg", "action"),
        ("local.db", "prod-pg", "menu"),
        ("local.db", "staging-pg", "menu"),
    ];
    for (renamed, deleted, how) in cases {
        let label = format!("rename {renamed}, delete {deleted} by {how}");
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let (workspace, explorer) = shell.read_with(&cx, |this, _| {
            (this.workspace.clone(), this.explorer.clone())
        });
        let mut want: Vec<String> = workspace.read_with(&cx, |ws, _| {
            ws.connections.iter().map(|c| c.name.clone()).collect()
        });
        let at = want.iter().position(|n| n == renamed).unwrap();
        let id = workspace.read_with(&cx, |ws, _| ws.connections[at].id);
        explorer.update_in(&mut cx, |e, window, cx| {
            e.reveal(&id.0.to_string(), window, cx)
        });
        cx.simulate_keystrokes("f2 x");
        if how == "action" {
            cx.dispatch_action(crate::DeleteConnection);
            cx.run_until_parked();
            click(&mut cx, "delete-confirm");
        } else {
            delete_row(&mut cx, &shell, deleted);
        }
        want.retain(|n| n != deleted);
        // Back in the tree, Enter would commit a rename still open.
        let first = workspace.read_with(&cx, |ws, _| ws.connections[0].id);
        explorer.update_in(&mut cx, |e, window, cx| {
            e.reveal(&first.0.to_string(), window, cx)
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let names: Vec<String> = workspace.read_with(&cx, |ws, _| {
            ws.connections.iter().map(|c| c.name.clone()).collect()
        });
        assert_eq!(names, want, "{label}: no connection renamed");
    }
}
