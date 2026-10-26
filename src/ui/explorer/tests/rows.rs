//! Row clicks, status dots and tooltips.

use super::*;

#[test]
fn status_dot_and_tooltip() {
    let cases = [
        (ConnectionStatus::Connected, true, "Connected"),
        (ConnectionStatus::Disconnected, false, "Disconnected"),
        (
            ConnectionStatus::Error("timeout".into()),
            false,
            "Could not connect: timeout",
        ),
        (
            ConnectionStatus::Error("".into()),
            false,
            "Could not connect: ",
        ),
        (ConnectionStatus::Connecting, false, "Connecting…"),
    ];
    for (status, solid, tip) in cases {
        assert_eq!(
            status_style(&status),
            (solid, tip.to_string()),
            "{status:?}"
        );
    }
}

#[test]
fn group_click_selects_chevron_or_double_click_toggles() {
    let containers = [
        NodeKind::Group(ObjectKind::Table),
        NodeKind::Group(ObjectKind::View),
        NodeKind::Connection(DbKind::Postgres),
        NodeKind::Database,
        NodeKind::Schema,
    ];
    for kind in containers {
        let cases = [
            (Hit::Row { clicks: 1 }, Click::Select),
            (Hit::Chevron, Click::Toggle),
            (Hit::Row { clicks: 2 }, Click::Toggle),
            (Hit::Row { clicks: 3 }, Click::Select),
        ];
        for (hit, want) in cases {
            assert_eq!(click_action(kind, hit), want, "{kind:?} {hit:?}");
        }
    }
    for kind in [NodeKind::Loading, NodeKind::ConnError] {
        assert_eq!(
            click_action(kind, Hit::Row { clicks: 1 }),
            Click::Select,
            "{kind:?}"
        );
    }
}

#[test]
fn table_click_opens_chevron_shows_structure() {
    for kind in [ObjectKind::Table, ObjectKind::View] {
        let kind = NodeKind::Object(kind);
        let cases = [
            (Hit::Row { clicks: 1 }, Click::Open { preview: true }),
            (Hit::Row { clicks: 2 }, Click::Open { preview: false }),
            (Hit::Chevron, Click::Toggle),
        ];
        for (hit, want) in cases {
            assert_eq!(click_action(kind, hit), want, "{kind:?} {hit:?}");
        }
    }
    for kind in [NodeKind::Part(Part::Columns), NodeKind::Item(Part::Columns)] {
        assert_eq!(
            click_action(kind, Hit::Row { clicks: 1 }),
            Click::Select,
            "{kind:?}"
        );
    }

    let ws = &mock::workspaces()[0];
    let mut open = default_expanded(ws);
    let nodes = flatten(ws, &open, &Default::default(), false);
    let table = nodes
        .iter()
        .position(|n| n.kind == NodeKind::Object(ObjectKind::Table))
        .unwrap();
    assert!(
        nodes[table].expandable && !nodes[table].expanded,
        "a table has a chevron, closed by default"
    );
    assert_eq!(
        nodes[table + 1].kind,
        NodeKind::Object(ObjectKind::Table),
        "children not expanded"
    );

    open.insert(nodes[table].key.clone());
    let nodes = flatten(ws, &open, &Default::default(), false);
    let parts: Vec<_> = nodes[table + 1..]
        .iter()
        .take_while(|n| n.depth > nodes[table].depth)
        .map(|n| (n.kind, n.label.to_string()))
        .collect();
    assert_eq!(
        parts,
        [
            (NodeKind::Part(Part::Columns), "Columns".to_string()),
            (NodeKind::Part(Part::Keys), "Keys".to_string()),
            (NodeKind::Part(Part::Indexes), "Indexes".to_string()),
        ]
    );
}

#[gpui_kit::test]
fn truncated_tree_names_show_their_full_text_in_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{AppContext as _, Bounds, Entity, WindowBounds, WindowOptions, point, px, size};
    cx.update(gpui_kit::init);
    let long = "shipment_line_item_carrier_tracking_events_archive_partitioned_by_month_2026_q3x";
    assert_eq!(long.len(), 80);
    // (explorer width, table name, tooltip shown)
    let cases = [
        (260., long, true),
        (260., "orders", false),
        (1200., long, false),
    ];
    for (width, name, tip) in cases {
        let mut ws = mock::workspaces().remove(0);
        let table = ws.connections[0].databases.as_mut().unwrap()[0]
            .schemas
            .as_mut()
            .unwrap()[0]
            .objects
            .iter_mut()
            .flatten()
            .find(|o| o.name == "orders")
            .expect("orders");
        table.name = name.to_string();
        let key = flatten(&ws, &default_expanded(&ws), &Default::default(), true)
            .into_iter()
            .find(|n| n.label.as_ref() == name)
            .expect("table row")
            .key;
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(width), px(600.)),
            ))),
            ..Default::default()
        };
        let (window, explorer): (_, Entity<super::Explorer>) = cx.update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                let ws = cx.new(|_| ws);
                let (handle, driver) =
                    (window.window_handle(), Arc::new(crate::db::Live::default()));
                let sessions = cx.new(|cx| Sessions::new(ws.clone(), driver, handle, cx));
                cx.new(|cx| super::Explorer::new(ws, sessions, window, cx))
            })
            .expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        explorer.update_in(&mut cx, |e, window, cx| e.reveal(&key, window, cx));
        cx.run_until_parked();
        let ix = explorer.read_with(&cx, |e, _| e.selected);
        let shown =
            crate::theme::tests::hover_shows_truncation_tip(&mut cx, format!("tree-label-{ix}"));
        assert_eq!(
            shown,
            tip,
            "{} chars at {width}px: tooltip shown",
            name.len()
        );
    }
}

#[test]
fn row_menus_offer_modify_and_new_only_on_postgres_and_disconnect_only_when_connected() {
    let (pg, lite) = (DbKind::Postgres, DbKind::Sqlite);
    let table = NodeKind::Object(ObjectKind::Table);
    let actions = |menu, new, console, object| RowActions {
        menu,
        new,
        console,
        object,
        connected: true,
    };
    let cases = [
        (
            NodeKind::Connection(pg),
            pg,
            Some(actions(RowMenu::Connection, false, false, None)),
        ),
        (
            NodeKind::Database,
            pg,
            Some(actions(
                RowMenu::Modify("Modify database"),
                true,
                true,
                None,
            )),
        ),
        (
            NodeKind::Database,
            lite,
            Some(actions(RowMenu::Refresh, false, true, None)),
        ),
        (
            NodeKind::Schema,
            pg,
            Some(actions(RowMenu::Modify("Modify schema"), true, true, None)),
        ),
        (
            table,
            pg,
            Some(actions(
                RowMenu::Modify("Modify table"),
                true,
                false,
                Some((true, true)),
            )),
        ),
        (
            table,
            lite,
            Some(actions(RowMenu::Refresh, false, false, Some((true, true)))),
        ),
        (NodeKind::Group(ObjectKind::Table), pg, None),
        (NodeKind::Loading, pg, None),
    ];
    for (kind, db, want) in cases {
        assert_eq!(
            row_actions(kind, db, true, "users"),
            want,
            "{kind:?} {db:?}"
        );
    }
    let offline = row_actions(NodeKind::Database, pg, false, "app_db").unwrap();
    assert!(!offline.connected);
}
