//! Edits, deletes and stale loads and sessions.

use super::*;

#[gpui_kit::test]
fn editing_the_params_or_deleting_a_connected_connection_closes_its_sessions(
    cx: &mut gpui_kit::TestAppContext,
) {
    use ConnectionStatus::*;
    // (change, session closed, status after (None = deleted), rows after)
    let cases: [(&str, bool, Option<ConnectionStatus>, &[&str]); 5] = [
        ("password", true, Some(Disconnected), &["pg"]),
        ("database", true, Some(Disconnected), &["pg"]),
        ("delete", true, None, &[]),
        ("name", false, Some(Connected), &["renamed", "app"]),
        ("nothing", false, Some(Connected), &["pg", "app"]),
    ];
    for (change, closed, want, rows) in cases {
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        let session = connect_answered(&explorer, &fake, &mut cx, &["app"]);
        assert_eq!(
            labels(&explorer, &cx),
            ["pg", "app"],
            "{change}: connected and open"
        );
        let id = crate::model::ConnectionId(1);
        workspace.update(&mut cx, |ws, cx| {
            match change {
                "password" => ws.edit(id, "pg".into(), 0, false, pg_params(Some("secret"), "app")),
                "database" => ws.edit(id, "pg".into(), 0, false, pg_params(Some("pw"), "other")),
                "name" => ws.edit(id, "renamed".into(), 0, false, pg_params(Some("pw"), "app")),
                "nothing" => ws.edit(id, "pg".into(), 0, false, pg_params(Some("pw"), "app")),
                _ => drop(ws.delete(id)),
            }
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(
            session.upgrade().is_none(),
            closed,
            "{change}: session closed"
        );
        assert_eq!(
            workspace.read_with(&cx, |ws, _| ws
                .connections
                .first()
                .map(|c| c.status.clone())),
            want,
            "{change}: status"
        );
        assert_eq!(labels(&explorer, &cx), rows, "{change}: rows");
        let collapsed = explorer.read_with(&cx, |e, _| e.nodes.first().is_none_or(|n| !n.expanded));
        assert_eq!(collapsed, closed, "{change}: tree collapsed");
    }
}

#[gpui_kit::test]
fn a_load_from_before_an_edit_and_reconnect_is_dropped_when_it_lands(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::db::fake::session;
    // (the old load's answer, whether the new load of the same schema has started when it lands)
    let cases = [
        (Ok(()), true),
        (Ok(()), false),
        (Err("server closed the connection"), true),
        (Err("server closed the connection"), false),
    ];
    for (old, new_started) in cases {
        let label = format!("old {old:?}, new load started {new_started}");
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["app"]);
        expand(&explorer, &mut cx, "app");
        fake.resolve("schemas app", Ok(strings(&["public"])));
        cx.run_until_parked();
        expand(&explorer, &mut cx, "public");
        assert_eq!(
            fake.pending(),
            ["objects app/public"],
            "{label}: the old server's load in flight"
        );

        workspace.update(&mut cx, |ws, cx| {
            ws.edit(
                crate::model::ConnectionId(1),
                "pg".into(),
                0,
                false,
                pg_params(Some("new"), "app"),
            );
            cx.notify();
        });
        cx.run_until_parked();
        expand_connection(&explorer, &mut cx);
        fake.resolve("connect app", Ok(session("app")));
        cx.run_until_parked();
        fake.resolve("databases app", Ok(strings(&["app"])));
        cx.run_until_parked();
        expand(&explorer, &mut cx, "app");
        fake.resolve("schemas app", Ok(strings(&["public"])));
        cx.run_until_parked();
        if new_started {
            expand(&explorer, &mut cx, "public");
        }

        // FIFO: the oldest "objects app/public" is the old server's.
        match old {
            Ok(()) => fake.resolve("objects app/public", Ok(vec![table("old_t")])),
            Err(reason) => fake
                .resolve::<Vec<crate::model::DbObject>>("objects app/public", Err(reason.into())),
        }
        cx.run_until_parked();
        let want: Vec<String> = if new_started {
            strings(&["Loading…"])
        } else {
            vec![]
        };
        assert_eq!(
            under(&explorer, &cx, "public"),
            want,
            "{label}: the old result is dropped"
        );
        if !new_started {
            expand(&explorer, &mut cx, "public");
        }
        fake.resolve("objects app/public", Ok(vec![table("new_t")]));
        cx.run_until_parked();
        expand(&explorer, &mut cx, "Tables");
        assert_eq!(
            labels(&explorer, &cx),
            ["pg", "app", "public", "Tables", "new_t"],
            "{label}: only the new server's data"
        );
    }
}

#[gpui_kit::test]
fn a_session_an_old_load_opens_after_an_edit_is_never_reused(cx: &mut gpui_kit::TestAppContext) {
    use crate::db::fake::session;
    // (the old session lands before or after the reconnect)
    for after_reconnect in [false, true] {
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
        expand(&explorer, &mut cx, "analytics");
        assert_eq!(
            fake.pending(),
            ["connect analytics"],
            "another database opens its own session"
        );
        workspace.update(&mut cx, |ws, cx| {
            ws.edit(
                crate::model::ConnectionId(1),
                "pg".into(),
                0,
                false,
                pg_params(Some("new"), "app"),
            );
            cx.notify();
        });
        cx.run_until_parked();
        if !after_reconnect {
            fake.resolve("connect analytics", Ok(session("analytics")));
            cx.run_until_parked();
        }
        connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
        if after_reconnect {
            fake.resolve("connect analytics", Ok(session("analytics")));
            cx.run_until_parked();
        }
        expand(&explorer, &mut cx, "analytics");
        assert_eq!(
            fake.pending(),
            ["connect analytics"],
            "after reconnect {after_reconnect}: the old server's session is not reused"
        );
    }
}

#[gpui_kit::test]
fn a_load_that_finds_its_session_closed_loses_the_connection_else_retry_reloads(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::db::fake::{close, session};
    // (row whose load fails, that load, session closed by the server, the calls Retry makes in turn)
    let cases = [
        (
            "app",
            "schemas app",
            true,
            vec!["connect app", "schemas app"],
        ),
        ("app", "schemas app", false, vec!["schemas app"]),
        (
            "pg",
            "databases app",
            true,
            vec!["connect app", "databases app"],
        ),
        ("pg", "databases app", false, vec!["databases app"]),
    ];
    for (row, call, closed, retry) in cases {
        let label = format!("{call} closed {closed}");
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        expand_connection(&explorer, &mut cx);
        let first = session("app");
        fake.resolve("connect app", Ok(first.clone()));
        cx.run_until_parked();
        if row == "app" {
            fake.resolve("databases app", Ok(strings(&["app"])));
            cx.run_until_parked();
            expand(&explorer, &mut cx, "app");
        }
        if closed {
            close(&first);
        }
        answer(
            &fake,
            call,
            Err("server closed the connection unexpectedly"),
        );
        cx.run_until_parked();
        if closed {
            assert_eq!(
                status_of(&workspace, &cx),
                ConnectionStatus::Disconnected,
                "{label}: the connection is lost"
            );
            explorer.update(&mut cx, |e, cx| {
                e.connect_connection(crate::model::ConnectionId(1), cx)
            });
            cx.run_until_parked();
            assert_eq!(fake.pending(), retry[..1], "{label}: Connect");
            continue;
        }
        let error = "Could not load: server closed the connection unexpectedly";
        assert_eq!(under(&explorer, &cx, row), [error], "{label}: error row");
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Connected,
            "{label}"
        );

        let ix = labels(&explorer, &cx)
            .iter()
            .position(|r| r == error)
            .unwrap();
        explorer.update_in(&mut cx, |e, _, cx| e.retry(ix, cx));
        cx.run_until_parked();
        for call in retry {
            assert_eq!(fake.pending(), [call], "{label}: Retry");
            answer(&fake, call, Ok(()));
            cx.run_until_parked();
        }
        assert!(
            under(&explorer, &cx, row)
                .iter()
                .all(|r| !r.starts_with("Could not")),
            "{label}: loaded after Retry"
        );
    }
}
