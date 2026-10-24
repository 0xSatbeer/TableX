//! Lazy loading of tree levels.

use super::*;

#[gpui_kit::test]
fn each_level_loads_only_when_expanded_and_shows_a_loading_row_meanwhile(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
    connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
    assert_eq!(
        fake.pending(),
        Vec::<String>::new(),
        "no database loads before it is expanded"
    );

    expand(&explorer, &mut cx, "app");
    assert_eq!(fake.pending(), ["schemas app"]);
    assert_eq!(under(&explorer, &cx, "app"), ["Loading…"]);
    fake.resolve("schemas app", Ok(strings(&["public", "empty"])));
    cx.run_until_parked();
    assert_eq!(under(&explorer, &cx, "app"), ["public", "empty"]);
    assert_eq!(
        under(&explorer, &cx, "analytics"),
        Vec::<String>::new(),
        "a database not expanded stays unloaded"
    );
    assert_eq!(
        fake.pending(),
        Vec::<String>::new(),
        "no schema loads before it is expanded"
    );

    expand(&explorer, &mut cx, "public");
    assert_eq!(fake.pending(), ["objects app/public"]);
    assert_eq!(under(&explorer, &cx, "public"), ["Loading…"]);
    fake.resolve("objects app/public", Ok(vec![table("t"), view("v")]));
    cx.run_until_parked();
    assert_eq!(under(&explorer, &cx, "public"), ["Tables", "Views"]);
    expand(&explorer, &mut cx, "Tables");
    assert_eq!(under(&explorer, &cx, "Tables"), ["t"]);
    assert_eq!(
        fake.pending(),
        Vec::<String>::new(),
        "no column loads before the table is expanded"
    );

    expand(&explorer, &mut cx, "t");
    assert_eq!(fake.pending(), ["columns app/public/t"]);
    assert_eq!(under(&explorer, &cx, "t"), ["Loading…"]);
    fake.resolve(
        "columns app/public/t",
        Ok(vec![column("id"), column("name")]),
    );
    cx.run_until_parked();
    expand(&explorer, &mut cx, "Columns");
    assert_eq!(under(&explorer, &cx, "t"), ["Columns", "id", "name"]);

    // (row expanded, the load it starts; it brings back nothing)
    let empty = [
        ("empty", "objects app/empty"),
        ("analytics", "schemas analytics"),
    ];
    for (row, call) in empty {
        expand(&explorer, &mut cx, row);
        if row == "analytics" {
            // Another database, so it opens a session of its own first.
            assert_eq!(fake.pending(), ["connect analytics"], "{row}");
            fake.resolve(
                "connect analytics",
                Ok(crate::db::fake::session("analytics")),
            );
            cx.run_until_parked();
        }
        assert_eq!(fake.pending(), [call], "{row}");
        assert_eq!(under(&explorer, &cx, row), ["Loading…"], "{row}");
        if row == "analytics" {
            fake.resolve::<Vec<String>>(call, Ok(vec![]));
        } else {
            fake.resolve::<Vec<crate::model::DbObject>>(call, Ok(vec![]));
        }
        cx.run_until_parked();
        // An empty schema says so; an empty database just has nothing under it.
        let (children, expandable): (&[&str], bool) = match row {
            "empty" => (&["No objects"], true),
            _ => (&[], false),
        };
        assert_eq!(under(&explorer, &cx, row), children, "{row}: children");
        let chevron = explorer.read_with(&cx, |e, _| {
            e.nodes
                .iter()
                .find(|n| n.label.as_ref() == row)
                .unwrap()
                .expandable
        });
        assert_eq!(chevron, expandable, "{row}: chevron once loaded empty");
    }
}

#[gpui_kit::test]
fn a_failed_load_shows_could_not_load_with_retry_under_that_node_only(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (row whose load fails, that load, how Retry is pressed, its rows once Retry succeeds)
    let cases = [
        ("public", "objects app/public", "enter", vec!["Tables"]),
        ("public", "objects app/public", "click", vec!["Tables"]),
        ("app", "schemas app", "enter", vec!["public", "sibling"]),
        ("t", "columns app/public/t", "click", vec!["Columns"]),
    ];
    for (row, call, how, after) in cases {
        let label = format!("{row} {how}");
        let reason = format!("permission denied for {row}");
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["app"]);
        for step in ["app", "public", "Tables", "t"] {
            expand(&explorer, &mut cx, step);
            for pending in fake.pending() {
                answer(
                    &fake,
                    &pending,
                    if pending == call {
                        Err(&reason)
                    } else {
                        Ok(())
                    },
                );
            }
            cx.run_until_parked();
            if step == row {
                break;
            }
        }
        let error = format!("Could not load: {reason}");
        assert_eq!(
            under(&explorer, &cx, row),
            std::slice::from_ref(&error),
            "{label}: error row under {row}"
        );
        let rows = labels(&explorer, &cx);
        assert_eq!(
            rows.iter().filter(|r| r.starts_with("Could not")).count(),
            1,
            "{label}: under that node only: {rows:?}"
        );
        assert!(
            status_style(&status_of(&workspace, &cx)).0,
            "{label}: the connection dot stays solid"
        );

        let ix = rows.iter().position(|r| *r == error).unwrap();
        match how {
            "enter" => {
                let key = explorer.read_with(&cx, |e, _| e.nodes[ix].key.clone());
                explorer.update_in(&mut cx, |e, window, cx| e.reveal(&key, window, cx));
                cx.simulate_keystrokes("enter");
            }
            _ => {
                let bounds = cx
                    .debug_bounds(format!("retry-{ix}").leak())
                    .unwrap_or_else(|| panic!("{label}: Retry drawn"));
                cx.simulate_click(bounds.center(), Default::default());
            }
        }
        cx.run_until_parked();
        assert_eq!(
            fake.pending(),
            [call],
            "{label}: Retry runs that node's load again"
        );
        answer(&fake, call, Ok(()));
        cx.run_until_parked();
        assert_eq!(
            under(&explorer, &cx, row),
            after,
            "{label}: the error row gives way to the children"
        );
        assert!(
            labels(&explorer, &cx)
                .iter()
                .all(|r| !r.starts_with("Could not")),
            "{label}: no error row left"
        );
    }
}

#[gpui_kit::test]
fn collapsing_during_a_load_still_stores_its_result_and_the_node_stays_collapsed(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (row collapsed while loading, its load, its rows once expanded again with no new load)
    let cases = [
        ("public", "objects app/public", vec!["Tables"]),
        ("app", "schemas app", vec!["public", "sibling"]),
        ("t", "columns app/public/t", vec!["Columns"]),
    ];
    for (row, call, after) in cases {
        let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["app"]);
        for step in ["app", "public", "Tables", "t"] {
            expand(&explorer, &mut cx, step);
            if step == row {
                break;
            }
            for pending in fake.pending() {
                answer(&fake, &pending, Ok(()));
            }
            cx.run_until_parked();
        }
        assert_eq!(fake.pending(), [call], "{row}");
        collapse(&explorer, &mut cx, row);
        answer(&fake, call, Ok(()));
        cx.run_until_parked();
        let expanded = explorer.read_with(&cx, |e, _| {
            e.nodes
                .iter()
                .find(|n| n.label.as_ref() == row)
                .unwrap()
                .expanded
        });
        assert!(!expanded, "{row}: stays collapsed");
        assert_eq!(
            under(&explorer, &cx, row),
            Vec::<String>::new(),
            "{row}: no rows under it"
        );
        expand(&explorer, &mut cx, row);
        assert_eq!(
            fake.pending(),
            Vec::<String>::new(),
            "{row}: the stored result shows without a new load"
        );
        assert_eq!(under(&explorer, &cx, row), after, "{row}");
    }
}

#[gpui_kit::test]
fn expanding_a_database_again_while_it_loads_opens_no_second_session(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (database expanded, collapsed and expanded again while loading, the calls pending)
    let cases = [
        ("analytics", vec!["connect analytics"]),
        ("app", vec!["schemas app"]),
    ];
    for (db, want) in cases {
        let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
        expand(&explorer, &mut cx, db);
        collapse(&explorer, &mut cx, db);
        expand(&explorer, &mut cx, db);
        assert_eq!(fake.pending(), want, "{db}");
    }
}
