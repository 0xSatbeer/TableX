//! Refresh deduplication and the row menu.

use super::*;

#[gpui_kit::test]
fn refresh_while_a_refresh_of_the_same_node_is_running_starts_no_second_load(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (rows refreshed in turn, the loads pending after)
    let cases: [(&[&str], &[&str]); 5] = [
        (&["pg", "pg"], &["databases app"]),
        (&["app", "app"], &["schemas app"]),
        (&["public", "public"], &["objects app/public"]),
        (&["t", "t"], &["columns app/public/t"]),
        (
            &["public", "t"],
            &["objects app/public", "columns app/public/t"],
        ),
    ];
    for (rows, want) in cases {
        let (mut cx, explorer, _, fake) = tables_open(cx);
        expand(&explorer, &mut cx, "t");
        answer(&fake, "columns app/public/t", Ok(()));
        cx.run_until_parked();
        for row in rows {
            refresh(&explorer, &mut cx, row);
        }
        assert_eq!(fake.pending(), want, "{rows:?}");
    }
}

#[gpui_kit::test]
fn only_connection_database_schema_and_table_or_view_rows_offer_refresh_and_only_the_connection_edit_and_delete(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut t = table("t");
    t.columns = Some(vec![column("id")]);
    let f = crate::model::DbObject {
        kind: ObjectKind::Function,
        ..table("f()")
    };
    // (row right-clicked, Edit… shown, Delete shown, Refresh shown)
    let cases = [
        ("pg", true, true, true),
        ("app", false, false, true),
        ("public", false, false, true),
        ("Tables", false, false, false),
        ("t", false, false, true),
        ("Columns", false, false, false),
        ("id", false, false, false),
        ("v", false, false, true),
        ("f()", false, false, false),
    ];
    for (row, edit, delete, refresh) in cases {
        let conn = loaded(
            pg(1, "pg", "app"),
            "app",
            "public",
            vec![t.clone(), view("v"), f.clone()],
        );
        let (mut cx, explorer, _, _) = open_explorer(cx, vec![conn]);
        for step in [
            "pg",
            "app",
            "public",
            "Tables",
            "t",
            "Columns",
            "Views",
            "Functions",
        ] {
            expand(&explorer, &mut cx, step);
        }
        let ix = labels(&explorer, &cx)
            .iter()
            .position(|l| l == row)
            .unwrap_or_else(|| panic!("{row} row"));
        let at = cx
            .debug_bounds(format!("tree-label-{ix}").leak())
            .unwrap_or_else(|| panic!("{row} row drawn"))
            .center();
        cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
        cx.run_until_parked();
        let shown = ["menu-edit", "menu-delete", "menu-refresh"]
            .map(|item| cx.debug_bounds(item).is_some());
        assert_eq!(
            shown,
            [edit, delete, refresh],
            "{row}: Edit…, Delete, Refresh"
        );
    }
}

#[gpui_kit::test]
fn refresh_never_fetches_table_rows(cx: &mut gpui_kit::TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    for row in ["pg", "app", "public", "t"] {
        let (mut cx, explorer, _, fake) = tables_open(cx);
        expand(&explorer, &mut cx, "t");
        answer(&fake, "columns app/public/t", Ok(()));
        cx.run_until_parked();
        let opened = Rc::new(RefCell::new(0));
        let sink = opened.clone();
        let _sub = cx.update(|_, cx| {
            cx.subscribe(&explorer, move |_, _: &super::OpenObject, _| {
                *sink.borrow_mut() += 1
            })
        });
        refresh(&explorer, &mut cx, row);
        // Answer everything the refresh reads, down the tree.
        while let Some(call) = fake.pending().first().cloned() {
            assert!(
                ["databases ", "schemas ", "objects ", "columns "]
                    .iter()
                    .any(|p| call.starts_with(p)),
                "{row}: only introspection, got {call}"
            );
            answer(&fake, &call, Ok(()));
            cx.run_until_parked();
        }
        assert_eq!(
            *opened.borrow(),
            0,
            "{row}: no table opened, so no rows fetched"
        );
    }
}
