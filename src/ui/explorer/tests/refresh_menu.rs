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
fn only_connection_database_schema_and_table_or_view_rows_offer_refresh_only_the_connection_edit_and_delete_and_only_a_postgres_database_schema_or_table_modify_and_new_and_only_a_database_or_schema_open_console(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut t = table("t");
    t.columns = Some(vec![column("id")]);
    let f = crate::model::DbObject {
        kind: ObjectKind::Function,
        ..table("f()")
    };
    // (Postgres connection, row right-clicked, Edit… shown, Delete shown, Refresh shown, Modify… shown, + New shown)
    let cases: [(bool, &str, bool, bool, bool, bool, bool); 13] = [
        (true, "pg", true, true, true, false, false),
        (true, "app", false, false, true, true, true),
        (true, "public", false, false, true, true, true),
        (true, "Tables", false, false, false, false, false),
        (true, "t", false, false, true, true, true),
        (true, "Columns", false, false, false, false, false),
        (true, "id", false, false, false, false, false),
        (true, "v", false, false, true, false, false),
        (true, "f()", false, false, true, false, false),
        (false, "pg", true, true, true, false, false),
        (false, "app", false, false, true, false, false),
        (false, "public", false, false, true, false, false),
        (false, "t", false, false, true, false, false),
    ];
    for (postgres, row, edit, delete, refresh, modify, new) in cases {
        let base = if postgres {
            pg(1, "pg", "app")
        } else {
            sqlite(1, "pg", "/x/app.db")
        };
        let conn = loaded(base, "app", "public", vec![t.clone(), view("v"), f.clone()]);
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
        let shown = ["menu-edit", "menu-delete", "menu-refresh", "menu-modify"]
            .map(|item| cx.debug_bounds(item).is_some());
        assert_eq!(
            shown,
            [edit, delete, refresh, modify],
            "{row} (postgres {postgres}): Edit…, Delete, Refresh, Modify…"
        );
        assert_eq!(
            cx.debug_bounds("menu-open-console").is_some(),
            matches!(row, "app" | "public"),
            "{row} (postgres {postgres}): Open console"
        );
        let object = matches!(row, "t" | "v" | "f()");
        let opens = matches!(row, "t" | "v");
        let shown = [
            "menu-new-console",
            "menu-open",
            "menu-copy-name",
            "menu-copy-qualified",
            "menu-show-ddl",
            "menu-drop",
        ]
        .map(|item| cx.debug_bounds(item).is_some());
        assert_eq!(
            shown,
            [
                matches!(row, "app" | "public") || object,
                opens,
                object,
                object,
                opens,
                object
            ],
            "{row} (postgres {postgres}): New console, Open, Copy name, Copy qualified name, Show DDL, Drop…"
        );
        // + New sits at the top: selecting it opens its submenu.
        cx.simulate_keystrokes("down");
        let items = [
            "menu-new-table",
            "menu-new-database",
            "menu-new-role",
            "menu-new-user",
        ]
        .map(|item| cx.debug_bounds(item));
        assert_eq!(
            items.map(|b| b.is_some()),
            [new; 4],
            "{row} (postgres {postgres}): + New ▸ Table, Database, Role, User"
        );
        if new {
            let tops = items.map(|b| b.expect("drawn").origin.y);
            assert!(
                tops.is_sorted(),
                "{row}: Table, Database, Role, User in order"
            );
        }
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

#[gpui_kit::test]
fn copy_qualified_name_puts_the_schema_and_name_quoted_on_the_clipboard_and_sqlite_only_the_name(
    cx: &mut gpui_kit::TestAppContext,
) {
    for (postgres, want) in [(true, r#""public"."users""#), (false, r#""users""#)] {
        let base = if postgres {
            pg(1, "pg", "app")
        } else {
            sqlite(1, "pg", "/x/app.db")
        };
        let conn = loaded(base, "app", "public", vec![table("users")]);
        let (mut cx, explorer, _, _) = open_explorer(cx, vec![conn]);
        for step in ["pg", "app", "public", "Tables"] {
            expand(&explorer, &mut cx, step);
        }
        right_click(&explorer, &mut cx, "users");
        click(&mut cx, "menu-copy-qualified");
        let copied = cx.read_from_clipboard().and_then(|c| c.text());
        assert_eq!(copied.as_deref(), Some(want), "postgres {postgres}");
        right_click(&explorer, &mut cx, "users");
        click(&mut cx, "menu-copy-name");
        let copied = cx.read_from_clipboard().and_then(|c| c.text());
        assert_eq!(copied.as_deref(), Some("users"), "postgres {postgres}");
    }
}

#[gpui_kit::test]
fn disconnect_on_any_row_of_a_connected_connection_disconnects_it_and_selects_its_row(
    cx: &mut gpui_kit::TestAppContext,
) {
    for row in ["pg", "app", "public", "t"] {
        let (mut cx, explorer, workspace, _) = tables_open(cx);
        right_click(&explorer, &mut cx, row);
        click(&mut cx, "menu-disconnect");
        let status = workspace.read_with(&cx, |ws, _| ws.connections[0].status.clone());
        assert_eq!(status, ConnectionStatus::Disconnected, "{row}");
        assert_eq!(
            labels(&explorer, &cx),
            strings(&["pg"]),
            "{row}: children hidden"
        );
        let selected = explorer.read_with(&cx, |e, _| e.selected_label());
        assert_eq!(selected.as_deref(), Some("pg"), "{row}");
        right_click(&explorer, &mut cx, "pg");
        assert!(
            cx.debug_bounds("menu-disconnect").is_none(),
            "{row}: no Disconnect once disconnected"
        );
    }
}
