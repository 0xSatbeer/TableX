//! `*` expanding everything under a node.

use super::*;

#[gpui_kit::test]
fn star_on_a_connected_connection_with_two_unloaded_databases_expands_their_schemas_and_objects_but_never_tables(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
    connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
    explorer.update_in(&mut cx, |e, _, cx| e.expand_all_under(0, cx));
    cx.run_until_parked();
    let calls = answer_all(&fake, &mut cx, "");
    assert!(
        calls.iter().all(|c| !c.starts_with("columns")),
        "no column fetch: {calls:?}"
    );
    for call in [
        "schemas analytics",
        "schemas app",
        "objects analytics/public",
        "objects analytics/sibling",
        "objects app/public",
        "objects app/sibling",
    ] {
        assert!(calls.iter().any(|c| c == call), "{call} fetched: {calls:?}");
    }
    let want: Vec<&str> = ["pg", "analytics"]
        .iter()
        .chain(&EXPANDED_DB)
        .chain(&["app"])
        .chain(&EXPANDED_DB)
        .copied()
        .collect();
    assert_eq!(labels(&explorer, &cx), want);
    assert!(!is_expanded(&explorer, &cx, "t"), "tables stay collapsed");
}

#[gpui_kit::test]
fn star_on_a_connection_that_is_not_connected_connects_first_then_expands_everything(
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
        explorer.update_in(&mut cx, |e, _, cx| e.expand_all_under(0, cx));
        cx.run_until_parked();
        assert_eq!(
            status_of(&workspace, &cx),
            ConnectionStatus::Connecting,
            "{status:?}"
        );
        assert_eq!(fake.pending(), ["connect app"], "{status:?}");
        fake.resolve("connect app", Ok(crate::db::fake::session("app")));
        cx.run_until_parked();
        fake.resolve("databases app", Ok(strings(&["analytics", "app"])));
        cx.run_until_parked();
        let calls = answer_all(&fake, &mut cx, "");
        assert!(
            calls.iter().all(|c| !c.starts_with("columns")),
            "{status:?}: no column fetch: {calls:?}"
        );
        let want: Vec<&str> = ["pg", "analytics"]
            .iter()
            .chain(&EXPANDED_DB)
            .chain(&["app"])
            .chain(&EXPANDED_DB)
            .copied()
            .collect();
        assert_eq!(labels(&explorer, &cx), want, "{status:?}");
    }
}

#[gpui_kit::test]
fn star_where_one_load_fails_shows_an_error_row_under_that_node_and_the_others_still_expand(
    cx: &mut gpui_kit::TestAppContext,
) {
    let error = "Could not load: permission denied";
    // (load that fails, rows of analytics then app)
    let cases: [(&str, Vec<&str>, Vec<&str>); 3] = [
        (
            "objects app/sibling",
            EXPANDED_DB.to_vec(),
            vec!["public", "tables", "t", "u", "sibling", error],
        ),
        (
            "objects app/public",
            EXPANDED_DB.to_vec(),
            vec!["public", error, "sibling", "tables", "t", "u"],
        ),
        ("schemas analytics", vec![error], EXPANDED_DB.to_vec()),
    ];
    for (fail, analytics, app) in cases {
        let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
        explorer.update_in(&mut cx, |e, _, cx| e.expand_all_under(0, cx));
        cx.run_until_parked();
        answer_all(&fake, &mut cx, fail);
        let want: Vec<&str> = ["pg", "analytics"]
            .into_iter()
            .chain(analytics)
            .chain(["app"])
            .chain(app)
            .collect();
        assert_eq!(labels(&explorer, &cx), want, "{fail}");
    }
}
