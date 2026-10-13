//! Connecting on expand and retry, and connects overtaken by an edit or delete.

use super::*;

#[gpui_kit::test]
fn expanding_a_disconnected_connection_connects_then_lists_its_databases(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::db::fake::session;
    // (connection, database the connect opens, databases listed)
    let cases = [
        (pg(1, "pg", "app"), "app", vec!["analytics", "app"]),
        (
            sqlite(1, "lite", "/x/local.db"),
            "local.db",
            vec!["local.db"],
        ),
    ];
    for (conn, db, databases) in cases {
        let name = conn.name.clone();
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![conn]);
        expand_connection(&explorer, &mut cx);
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Connecting,
            "{name}"
        );
        assert_eq!(fake.pending(), [format!("connect {db}")], "{name}");
        fake.resolve(&format!("connect {db}"), Ok(session(db)));
        cx.run_until_parked();
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Connected,
            "{name}"
        );
        assert_eq!(fake.pending(), [format!("databases {db}")], "{name}");
        fake.resolve(
            &format!("databases {db}"),
            Ok(databases.iter().map(|d| d.to_string()).collect::<Vec<_>>()),
        );
        cx.run_until_parked();
        let mut want = vec![name.clone()];
        want.extend(databases.iter().map(|d| d.to_string()));
        assert_eq!(
            labels(&explorer, &cx),
            want,
            "{name}: the child rows are the databases"
        );
    }
}

#[gpui_kit::test]
fn a_failed_connection_connects_again_on_expand_or_retry(cx: &mut gpui_kit::TestAppContext) {
    use crate::db::fake::session;
    use ConnectionStatus::*;
    // (how, the connect's answer, status after)
    let cases = [
        ("expand", Ok(()), Connected),
        (
            "expand",
            Err("password authentication failed for user \"u\""),
            Error("password authentication failed for user \"u\"".into()),
        ),
        ("retry", Ok(()), Connected),
        (
            "retry",
            Err("timeout expired after 10 s"),
            Error("timeout expired after 10 s".into()),
        ),
        ("retry", Err("old"), Error("old".into())),
    ];
    for (how, answer, want) in cases {
        let label = format!("{how} {answer:?}");
        let conn = crate::model::Connection {
            status: Error("old".into()),
            ..pg(1, "pg", "app")
        };
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![conn]);
        match how {
            "expand" => expand_connection(&explorer, &mut cx),
            _ => {
                explorer.update_in(&mut cx, |e, window, cx| {
                    e.reveal(&format!("1/{}", super::ERROR), window, cx)
                });
                assert_eq!(
                    explorer.read_with(&cx, |e, _| e.nodes[e.selected].kind),
                    NodeKind::ConnError,
                    "{label}"
                );
                cx.simulate_keystrokes("enter");
            }
        }
        assert_eq!(status_of(&workspace, &cx), Connecting, "{label}");
        fake.resolve(
            "connect app",
            answer.map(|()| session("app")).map_err(String::from),
        );
        cx.run_until_parked();
        assert_eq!(status_of(&workspace, &cx), want, "{label}");
    }
}

#[gpui_kit::test]
fn a_connect_that_failed_on_expand_connects_again_on_enter_or_right_on_its_open_row(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::db::fake::session;
    use ConnectionStatus::*;
    // (key on the failed, still open connection row, the new connect's answer, status after)
    let cases = [
        ("enter", Ok(()), Connected),
        ("enter", Err("new"), Error("new".into())),
        ("enter", Err("boom"), Error("boom".into())),
        ("right", Ok(()), Connected),
        ("right", Err("new"), Error("new".into())),
    ];
    for (key, answer, want) in cases {
        let label = format!("{key} {answer:?}");
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        expand_connection(&explorer, &mut cx);
        fake.resolve::<crate::db::Session>("connect app", Err("boom".into()));
        cx.run_until_parked();
        explorer.update_in(&mut cx, |e, window, cx| e.reveal("1", window, cx));
        assert!(
            explorer.read_with(&cx, |e, _| e.nodes[0].expanded),
            "{label}: the failed row is open"
        );
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        assert_eq!(status_of(&workspace, &cx), Connecting, "{label}");
        assert_eq!(fake.pending(), ["connect app"], "{label}");
        fake.resolve(
            "connect app",
            answer.map(|()| session("app")).map_err(String::from),
        );
        cx.run_until_parked();
        assert_eq!(status_of(&workspace, &cx), want, "{label}");
    }
}

#[gpui_kit::test]
fn expanding_again_while_connecting_starts_no_second_connect(cx: &mut gpui_kit::TestAppContext) {
    // (keys pressed on the connecting connection's row)
    let cases = ["right", "left right", "enter", "left enter", "space space"];
    for keys in cases {
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        expand_connection(&explorer, &mut cx);
        explorer.update_in(&mut cx, |e, window, cx| e.reveal("1", window, cx));
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        assert_eq!(fake.pending(), ["connect app"], "{keys}");
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Connecting,
            "{keys}"
        );
    }
}

#[gpui_kit::test]
fn a_connection_collapsed_and_expanded_again_while_connecting_opens_when_the_connect_lands(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::db::fake::session;
    // (expanded again before the connect lands, the connect's answer, open after, rows after)
    let cases: [(bool, Result<(), &str>, bool, &[&str]); 4] = [
        (true, Ok(()), true, &["pg", "app"]),
        (true, Err("boom"), true, &["pg", "Could not connect: boom"]),
        (false, Ok(()), false, &["pg"]),
        (
            false,
            Err("boom"),
            false,
            &["pg", "Could not connect: boom"],
        ),
    ];
    for (again, answer, open, rows) in cases {
        let label = format!("again {again} {answer:?}");
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        expand_connection(&explorer, &mut cx);
        collapse(&explorer, &mut cx, "pg");
        if again {
            expand(&explorer, &mut cx, "pg");
        }
        assert_eq!(
            fake.pending(),
            ["connect app"],
            "{label}: no second connect"
        );
        fake.resolve(
            "connect app",
            answer.map(|()| session("app")).map_err(String::from),
        );
        cx.run_until_parked();
        if answer.is_ok() {
            assert_eq!(
                status_of(&workspace, &cx),
                ConnectionStatus::Connected,
                "{label}"
            );
            fake.resolve("databases app", Ok(strings(&["app"])));
            cx.run_until_parked();
        }
        assert_eq!(
            explorer.read_with(&cx, |e, _| e.nodes[0].expanded),
            open,
            "{label}: open"
        );
        assert_eq!(labels(&explorer, &cx), rows, "{label}: rows");
    }
}

#[gpui_kit::test]
fn a_failed_connect_shows_could_not_connect_in_the_error_row_and_the_dot_tooltip(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (reason the connect fails with)
    let cases = [
        "password authentication failed for user \"u\"",
        "timeout expired after 10 s",
        "No file at /x/local.db",
    ];
    for reason in cases {
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        expand_connection(&explorer, &mut cx);
        fake.resolve::<crate::db::Session>("connect app", Err(reason.into()));
        cx.run_until_parked();
        let want = format!("Could not connect: {reason}");
        assert_eq!(
            labels(&explorer, &cx),
            ["pg".to_string(), want.clone()],
            "tree error row"
        );
        assert_eq!(
            status_style(&status_of(&workspace, &cx)),
            (false, want),
            "ring dot and its tooltip"
        );
    }
}

#[gpui_kit::test]
fn a_connect_whose_connection_is_edited_or_deleted_while_connecting_is_dropped(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::db::fake::session;
    use ConnectionStatus::*;
    // (change while connecting, the connect's answer, the connection's status once it lands; None = deleted)
    let cases: [(&str, Result<(), &str>, Option<ConnectionStatus>); 8] = [
        ("database", Ok(()), Some(Disconnected)),
        ("database", Err("boom"), Some(Disconnected)),
        ("password", Ok(()), Some(Disconnected)),
        ("password", Err("boom"), Some(Disconnected)),
        ("delete", Ok(()), None),
        ("delete", Err("boom"), None),
        // Name only: the params are the same, so the connect still lands.
        ("name", Ok(()), Some(Connected)),
        ("name", Err("boom"), Some(Error("boom".into()))),
    ];
    for (change, answer, want) in cases {
        let label = format!("{change} {answer:?}");
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        expand_connection(&explorer, &mut cx);
        let id = crate::model::ConnectionId(1);
        workspace.update(&mut cx, |ws, cx| {
            match change {
                "database" => ws.edit(id, "pg".into(), 0, false, pg_params(None, "other")),
                "password" => ws.edit(id, "pg".into(), 0, false, pg_params(Some("secret"), "app")),
                "name" => ws.edit(id, "renamed".into(), 0, false, pg_params(None, "app")),
                _ => drop(ws.delete(id)),
            }
            cx.notify();
        });
        cx.run_until_parked();
        fake.resolve(
            "connect app",
            answer.map(|()| session("app")).map_err(String::from),
        );
        cx.run_until_parked();
        let status = workspace.read_with(&cx, |ws, _| {
            ws.connections.first().map(|c| c.status.clone())
        });
        assert_eq!(status, want, "{label}");
        if change != "name" {
            assert_eq!(
                fake.pending(),
                Vec::<String>::new(),
                "{label}: nothing loads from the dropped result"
            );
            let rows = labels(&explorer, &cx);
            assert!(
                rows.iter().all(|r| !r.starts_with("Could not connect")),
                "{label}: no error row from it: {rows:?}"
            );
        }
    }
}
