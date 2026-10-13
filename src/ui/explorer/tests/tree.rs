//! Flattening the tree and its keys.

use super::*;

#[test]
fn flatten_follows_expanded_set() {
    let ws = &mock::workspaces()[0];
    let collapsed = flatten(ws, &Default::default(), &Default::default(), false);
    let failed = ws
        .connections
        .iter()
        .filter(|c| matches!(c.status, ConnectionStatus::Error(_)))
        .count();
    assert!(failed > 0, "mock has a failed connection");
    assert_eq!(
        collapsed.len(),
        ws.connections.len() + failed,
        "connections, plus the message under each failed one"
    );
    assert_eq!(
        collapsed
            .iter()
            .filter(|n| n.kind == NodeKind::ConnError)
            .count(),
        failed
    );

    let mut open = default_expanded(ws);
    let nodes = flatten(ws, &open, &Default::default(), false);
    let tables = nodes
        .iter()
        .position(|n| n.kind == NodeKind::Group(ObjectKind::Table))
        .expect("tables group");
    let count = nodes[tables].count.unwrap();
    assert!(
        nodes[tables + 1..=tables + count]
            .iter()
            .all(|n| n.kind == NodeKind::Object(ObjectKind::Table))
    );
    assert_eq!(parent_of(&nodes, tables + 1), Some(tables));

    let schema = &nodes[parent_of(&nodes, tables).unwrap()].key;
    let loading = flatten(
        ws,
        &open,
        &[(schema.clone(), super::Load::Loading)].into(),
        false,
    );
    assert_eq!(
        loading
            .iter()
            .filter(|n| n.kind == NodeKind::Loading)
            .count(),
        1,
        "loading schema shows one placeholder"
    );
    assert!(loading.iter().all(
        |n| n.kind != NodeKind::Group(ObjectKind::Table) || !n.key.starts_with(schema.as_str())
    ));

    open.remove(&nodes[tables].key);
    assert_eq!(
        flatten(ws, &open, &Default::default(), false).len(),
        nodes.len() - count,
        "collapsing hides exactly the children"
    );
}

#[test]
fn only_connected_connections_show_children() {
    let mut ws = mock::workspaces()[0].clone();
    let all =
        |ws: &crate::model::Workspace| flatten(ws, &Default::default(), &Default::default(), true);
    let ci = ws
        .connections
        .iter()
        .position(|c| c.status == ConnectionStatus::Disconnected)
        .unwrap();
    let prefix = format!("{}/", ws.connections[ci].id.0);
    let under = |nodes: &[super::FlatNode]| {
        nodes
            .iter()
            .filter(|n| n.key.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert!(
        under(&all(&ws)).is_empty(),
        "disconnected: no children even when open"
    );

    ws.connections[ci].status = ConnectionStatus::Connecting;
    let connecting = under(&all(&ws));
    assert_eq!(connecting.len(), 1);
    assert_eq!(connecting[0].kind, NodeKind::Loading);
    assert!(connecting[0].label.starts_with("Connecting to "));

    ws.connections[ci].status = ConnectionStatus::Connected;
    assert!(
        under(&all(&ws))
            .iter()
            .any(|n| n.kind == NodeKind::Database)
    );

    ws.connections[ci].status = ConnectionStatus::Error("boom".into());
    let failed = under(&all(&ws));
    assert_eq!(
        (failed.len(), failed[0].kind, failed[0].label.as_ref()),
        (1, NodeKind::ConnError, "Could not connect: boom")
    );
}

#[gpui_kit::test]
fn keys_are_name_paths_with_slash_and_backslash_escaped(cx: &mut gpui_kit::TestAppContext) {
    let mut users = table("users");
    users.columns = Some(vec![column("id"), column("a/b")]);
    let objects = vec![
        users,
        table("a/b"),
        table("a\\b"),
        table("\\/"),
        table("x\\"),
        table(""),
    ];
    let conn = loaded(pg(7, "pg", "app"), "my/db", "s\\1", objects);
    let (mut cx, explorer, _, _) = open_explorer(cx, vec![conn]);
    explorer.update_in(&mut cx, |e, _, cx| {
        e.expand_all_under(0, cx);
        // `*` leaves tables collapsed.
        for label in ["users", "Columns"] {
            let ix = e
                .nodes
                .iter()
                .position(|n| n.label.as_ref() == label)
                .unwrap_or_else(|| panic!("{label} row"));
            e.set_expanded(ix, true, cx);
        }
    });
    // (row, key): segments are connection id, database, schema, kind, object, then the structure tail
    let cases = [
        ("pg", r"7"),
        ("my/db", r"7/my\/db"),
        ("s\\1", r"7/my\/db/s\\1"),
        ("Tables", r"7/my\/db/s\\1/table"),
        ("users", r"7/my\/db/s\\1/table/users"),
        ("Columns", r"7/my\/db/s\\1/table/users/Columns"),
        ("id", r"7/my\/db/s\\1/table/users/Columns/0"),
        ("a/b", r"7/my\/db/s\\1/table/users/Columns/1"),
        ("a/b", r"7/my\/db/s\\1/table/a\/b"),
        ("a\\b", r"7/my\/db/s\\1/table/a\\b"),
        ("\\/", r"7/my\/db/s\\1/table/\\\/"),
        ("x\\", r"7/my\/db/s\\1/table/x\\"),
        ("", r"7/my\/db/s\\1/table/"),
    ];
    let rows: Vec<(String, String)> = explorer.read_with(&cx, |e, _| {
        e.nodes
            .iter()
            .map(|n| (n.label.to_string(), n.key.clone()))
            .collect()
    });
    let want: Vec<(String, String)> = cases
        .iter()
        .map(|(label, key)| (label.to_string(), key.to_string()))
        .collect();
    assert_eq!(rows, want);
    // Each key reveals its own row from a collapsed tree: the escapes keep names whole.
    for (label, key) in cases {
        explorer.update_in(&mut cx, |e, window, cx| {
            e.expanded.clear();
            e.refresh(cx);
            e.reveal(key, window, cx);
        });
        let selected = explorer.read_with(&cx, |e, _| {
            e.nodes
                .get(e.selected)
                .map(|n| (n.label.to_string(), n.key.clone()))
        });
        assert_eq!(
            selected,
            Some((label.to_string(), key.to_string())),
            "reveal {key}"
        );
    }
}

#[gpui_kit::test]
fn a_tab_on_t_keeps_opening_and_highlighting_t_after_a_table_is_inserted_before_it(
    cx: &mut gpui_kit::TestAppContext,
) {
    use std::{cell::RefCell, rc::Rc};
    let conn = loaded(
        pg(1, "pg", "app"),
        "app",
        "public",
        vec![table("t"), table("u")],
    );
    let (mut cx, explorer, workspace, _) = open_explorer(cx, vec![conn]);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&explorer, move |_, ev: &super::OpenObject, _| {
            sink.borrow_mut().push(ev.key.to_string())
        })
    });
    let open_t = |cx: &mut gpui_kit::VisualTestContext| {
        explorer.update_in(cx, |e, _, cx| {
            e.expand_all_under(0, cx);
            let ix = e
                .nodes
                .iter()
                .position(|n| n.label.as_ref() == "t")
                .expect("t row");
            e.open(ix, false, cx);
        });
    };
    open_t(&mut cx);
    workspace.update(&mut cx, |ws, cx| {
        let schema = &mut ws.connections[0].databases.as_mut().unwrap()[0]
            .schemas
            .as_mut()
            .unwrap()[0];
        schema.objects.as_mut().unwrap().insert(0, table("a"));
        cx.notify();
    });
    cx.run_until_parked();
    open_t(&mut cx);
    let keys = opened.borrow().clone();
    assert_eq!(keys.len(), 2);
    assert_eq!(
        keys[0], keys[1],
        "opening t again gives the same tab key, so the same tab"
    );
    explorer.update_in(&mut cx, |e, window, cx| e.reveal(&keys[0], window, cx));
    assert_eq!(
        explorer.read_with(&cx, |e, _| e.selected_label()),
        Some("t".into()),
        "the tab highlights t, not a"
    );
}
