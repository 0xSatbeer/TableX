//! Refreshing nodes.

use super::*;

#[gpui_kit::test]
fn refreshing_a_schema_after_a_table_was_created_elsewhere_shows_it_sorted(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, explorer, _, fake) = tables_open(cx);
    assert_eq!(under(&explorer, &cx, "tables"), ["t", "u"]);
    refresh(&explorer, &mut cx, "public");
    assert_eq!(fake.pending(), ["objects app/public"]);
    fake.resolve(
        "objects app/public",
        Ok(vec![table("t"), table("t2"), table("u")]),
    );
    cx.run_until_parked();
    assert_eq!(under(&explorer, &cx, "tables"), ["t", "t2", "u"]);
}

#[gpui_kit::test]
fn refreshing_a_schema_after_a_table_was_dropped_removes_it_and_forgets_its_expansion(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (t expanded before it was dropped)
    for t_open in [true, false] {
        let (mut cx, explorer, _, fake) = tables_open(cx);
        if t_open {
            expand(&explorer, &mut cx, "t");
            answer(&fake, "columns app/public/t", Ok(()));
            cx.run_until_parked();
        }
        refresh(&explorer, &mut cx, "public");
        fake.resolve("objects app/public", Ok(vec![table("u")]));
        cx.run_until_parked();
        assert_eq!(
            under(&explorer, &cx, "tables"),
            ["u"],
            "t open {t_open}: t gone"
        );

        // t created again: it comes back collapsed, with no column load.
        refresh(&explorer, &mut cx, "public");
        fake.resolve("objects app/public", Ok(vec![table("t"), table("u")]));
        cx.run_until_parked();
        assert_eq!(
            under(&explorer, &cx, "tables"),
            ["t", "u"],
            "t open {t_open}"
        );
        assert!(
            !is_expanded(&explorer, &cx, "t"),
            "t open {t_open}: its expansion is forgotten"
        );
        assert_eq!(fake.pending(), Vec::<String>::new(), "t open {t_open}");
    }
}

#[gpui_kit::test]
fn refreshing_a_schema_with_table_t_expanded_keeps_t_expanded_with_its_columns_re_read(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (row refreshed, the loads it runs before t's columns, answered with the sample)
    let cases = [
        ("public", vec!["objects app/public"]),
        ("app", vec!["schemas app", "objects app/public"]),
    ];
    for (row, loads) in cases {
        let (mut cx, explorer, _, fake) = tables_open(cx);
        expand(&explorer, &mut cx, "t");
        fake.resolve("columns app/public/t", Ok(vec![column("id")]));
        cx.run_until_parked();
        expand(&explorer, &mut cx, "Columns");
        refresh(&explorer, &mut cx, row);
        for call in loads {
            assert_eq!(fake.pending(), [call], "{row}");
            answer(&fake, call, Ok(()));
            cx.run_until_parked();
        }
        assert_eq!(
            fake.pending(),
            ["columns app/public/t"],
            "{row}: t's columns are re-read"
        );
        assert!(is_expanded(&explorer, &cx, "t"), "{row}: t stays expanded");
        fake.resolve(
            "columns app/public/t",
            Ok(vec![column("id"), column("name")]),
        );
        cx.run_until_parked();
        assert_eq!(
            under(&explorer, &cx, "t"),
            ["Columns", "id", "name"],
            "{row}"
        );
    }
}

#[gpui_kit::test]
fn refreshing_a_connection_re_lists_its_databases_and_re_reads_its_expanded_databases_and_schemas(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (databases the refresh lists, rows after)
    let cases = [
        (
            vec!["analytics", "app", "new"],
            vec![
                "pg",
                "analytics",
                "app",
                "public",
                "tables",
                "sibling",
                "new",
            ],
        ),
        (
            vec!["app"],
            vec!["pg", "app", "public", "tables", "sibling"],
        ),
    ];
    for (databases, rows) in cases {
        let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
        for step in ["app", "public"] {
            expand(&explorer, &mut cx, step);
            for pending in fake.pending() {
                answer(&fake, &pending, Ok(()));
            }
            cx.run_until_parked();
        }
        refresh(&explorer, &mut cx, "pg");
        assert_eq!(
            fake.pending(),
            ["databases app"],
            "{databases:?}: no new connect, the databases are re-listed"
        );
        fake.resolve("databases app", Ok(strings(&databases)));
        cx.run_until_parked();
        // analytics is not expanded, so it is not read.
        for call in ["schemas app", "objects app/public"] {
            assert_eq!(fake.pending(), [call], "{databases:?}");
            answer(&fake, call, Ok(()));
            cx.run_until_parked();
        }
        assert_eq!(fake.pending(), Vec::<String>::new(), "{databases:?}");
        assert_eq!(labels(&explorer, &cx), rows, "{databases:?}");
    }
}

#[gpui_kit::test]
fn old_children_stay_visible_while_a_refresh_is_in_flight(cx: &mut gpui_kit::TestAppContext) {
    // (row refreshed, its load)
    let cases = [
        ("pg", "databases app"),
        ("app", "schemas app"),
        ("public", "objects app/public"),
        ("t", "columns app/public/t"),
    ];
    for (row, call) in cases {
        let (mut cx, explorer, _, fake) = tables_open(cx);
        expand(&explorer, &mut cx, "t");
        answer(&fake, "columns app/public/t", Ok(()));
        cx.run_until_parked();
        expand(&explorer, &mut cx, "Columns");
        let before = under(&explorer, &cx, row);
        refresh(&explorer, &mut cx, row);
        assert_eq!(fake.pending(), [call], "{row}");
        assert_eq!(
            under(&explorer, &cx, row),
            before,
            "{row}: the old children stay until the new ones arrive"
        );
    }
}

#[gpui_kit::test]
fn the_connection_row_shows_a_refresh_icon_while_a_refresh_under_it_is_in_flight(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (row refreshed, its load)
    let cases = [
        ("pg", "databases app"),
        ("app", "schemas app"),
        ("public", "objects app/public"),
        ("t", "columns app/public/t"),
    ];
    for (row, call) in cases {
        let (mut cx, explorer, _, fake) = tables_open(cx);
        expand(&explorer, &mut cx, "t");
        answer(&fake, "columns app/public/t", Ok(()));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("conn-refreshing-0").is_none(),
            "{row}: no icon before the refresh"
        );
        refresh(&explorer, &mut cx, row);
        assert!(
            cx.debug_bounds("conn-refreshing-0").is_some(),
            "{row}: icon while in flight"
        );
        answer(&fake, call, Ok(()));
        cx.run_until_parked();
        while !fake.pending().is_empty() {
            for pending in fake.pending() {
                answer(&fake, &pending, Ok(()));
            }
            cx.run_until_parked();
        }
        assert!(
            cx.debug_bounds("conn-refreshing-0").is_none(),
            "{row}: icon gone once the refresh lands"
        );
    }
}
