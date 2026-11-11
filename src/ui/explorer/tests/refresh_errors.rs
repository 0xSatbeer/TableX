//! Refresh before connecting and refreshes that fail.

use super::*;

#[gpui_kit::test]
fn refresh_on_a_connection_that_is_not_connected_connects_the_same_as_expand(
    cx: &mut gpui_kit::TestAppContext,
) {
    let cases = [
        ConnectionStatus::Disconnected,
        ConnectionStatus::Error("connection refused".into()),
    ];
    for status in cases {
        let conn = crate::model::Connection {
            status: status.clone(),
            ..pg(1, "pg", "app")
        };
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![conn]);
        refresh(&explorer, &mut cx, "pg");
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Connecting,
            "{status:?}"
        );
        assert_eq!(fake.pending(), ["connect app"], "{status:?}");
        fake.resolve("connect app", Ok(crate::db::fake::session("app")));
        cx.run_until_parked();
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Connected,
            "{status:?}"
        );
        answer(&fake, "databases app", Ok(()));
        cx.run_until_parked();
        assert_eq!(
            labels(&explorer, &cx),
            ["pg", "app"],
            "{status:?}: open on its databases"
        );
    }
}

#[gpui_kit::test]
fn a_failed_refresh_of_a_database_schema_or_table_shows_an_error_row_and_removes_the_previous_children(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (row refreshed, its load)
    let cases = [
        ("app", "schemas app"),
        ("public", "objects app/public"),
        ("t", "columns app/public/t"),
    ];
    for (row, call) in cases {
        let (mut cx, explorer, workspace, fake) = tables_open(cx);
        expand(&explorer, &mut cx, "t");
        answer(&fake, "columns app/public/t", Ok(()));
        cx.run_until_parked();
        refresh(&explorer, &mut cx, row);
        answer(&fake, call, Err("permission denied"));
        cx.run_until_parked();
        assert_eq!(
            under(&explorer, &cx, row),
            ["Could not load: permission denied"],
            "{row}: error row under it"
        );
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Connected,
            "{row}"
        );
        // The old children are gone, so Retry loads from nothing.
        let ix = labels(&explorer, &cx)
            .iter()
            .position(|r| r.starts_with("Could not"))
            .unwrap();
        explorer.update_in(&mut cx, |e, _, cx| e.retry(ix, cx));
        cx.run_until_parked();
        assert_eq!(fake.pending(), [call], "{row}");
        assert_eq!(
            under(&explorer, &cx, row),
            ["Loading…"],
            "{row}: previous children removed"
        );
    }
}

#[gpui_kit::test]
fn a_failed_refresh_of_the_connection_sets_status_error_and_shows_the_connection_error_row(
    cx: &mut gpui_kit::TestAppContext,
) {
    let reason = "server closed the connection unexpectedly";
    // (a refresh of database app is also in flight when the connection's fails)
    for app_in_flight in [false, true] {
        let (mut cx, explorer, workspace, fake) = tables_open(cx);
        if app_in_flight {
            refresh(&explorer, &mut cx, "app");
        }
        refresh(&explorer, &mut cx, "pg");
        answer(&fake, "databases app", Err(reason));
        cx.run_until_parked();
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Error(reason.into()),
            "app in flight {app_in_flight}"
        );
        assert_eq!(
            labels(&explorer, &cx),
            ["pg", &format!("Could not connect: {reason}")],
            "app in flight {app_in_flight}"
        );

        // Retry reconnects; app's old load is dropped and its children read again.
        explorer.update_in(&mut cx, |e, _, cx| e.retry(1, cx));
        cx.run_until_parked();
        answer(&fake, "connect app", Ok(()));
        cx.run_until_parked();
        answer(&fake, "databases app", Ok(()));
        cx.run_until_parked();
        let mut stale = fake.pending();
        stale.retain(|c| c == "schemas app");
        assert_eq!(
            stale.len(),
            1 + app_in_flight as usize,
            "app in flight {app_in_flight}: app is read again: {:?}",
            fake.pending()
        );
        for _ in stale {
            answer(&fake, "schemas app", Ok(()));
        }
        cx.run_until_parked();
        answer(&fake, "objects app/public", Ok(()));
        cx.run_until_parked();
        assert_eq!(
            labels(&explorer, &cx),
            ["pg", "app", "public", "tables", "t", "u", "sibling"],
            "app in flight {app_in_flight}"
        );
    }
}
