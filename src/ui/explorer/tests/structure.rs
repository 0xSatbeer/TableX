//! Row estimates and table structure rows.

use super::*;

#[test]
fn row_estimate_labels() {
    let cases = [
        (None, None),
        (Some(0), Some("0")),
        (Some(1), Some("1")),
        (Some(999), Some("999")),
        (Some(1000), Some("1k")),
        (Some(1234), Some("1.2k")),
        (Some(1950), Some("2k")),
        (Some(9_999), Some("10k")),
        (Some(10_000), Some("10k")),
        (Some(12_345), Some("12k")),
        (Some(99_999), Some("100k")),
        (Some(999_499), Some("999k")),
        (Some(999_500), Some("1M")),
        (Some(999_999), Some("1M")),
        (Some(1_000_000), Some("1M")),
        (Some(1_234_567), Some("1.2M")),
        (Some(12_345_678), Some("12M")),
        (Some(1_500_000_000), Some("1.5B")),
    ];
    for (rows, want) in cases {
        assert_eq!(row_estimate_label(rows).as_deref(), want, "{rows:?}");
    }

    let ws = &mock::workspaces()[0];
    let nodes = flatten(ws, &Default::default(), &Default::default(), true);
    let first = |kind| {
        nodes
            .iter()
            .find(|n| n.kind == NodeKind::Object(kind))
            .unwrap()
    };
    let table = first(ObjectKind::Table);
    let conn = ws
        .connections
        .iter()
        .find(|c| c.status == ConnectionStatus::Connected)
        .unwrap();
    let obj = conn.databases.as_ref().unwrap()[0]
        .schemas
        .as_ref()
        .unwrap()[0]
        .objects
        .iter()
        .flatten()
        .find(|o| o.kind == ObjectKind::Table)
        .unwrap();
    assert_eq!(obj.name, table.label.as_ref());
    let estimate = obj.row_estimate;
    assert_eq!(
        table.suffix.as_deref(),
        row_estimate_label(estimate).as_deref(),
        "table row shows its estimate"
    );
    assert!(table.suffix.is_some());
    assert_eq!(
        first(ObjectKind::View).suffix,
        None,
        "views have no estimate"
    );
}

#[test]
fn structure_omits_empty_parts() {
    let mut ws = mock::workspaces()[0].clone();
    let conn = ws
        .connections
        .iter_mut()
        .find(|c| c.status == ConnectionStatus::Connected)
        .unwrap();
    let obj = conn.databases.as_mut().unwrap()[0]
        .schemas
        .as_mut()
        .unwrap()[0]
        .objects
        .iter_mut()
        .flatten()
        .find(|o| o.kind == ObjectKind::Table)
        .unwrap();
    obj.columns.as_mut().unwrap().truncate(3);
    obj.keys.truncate(1);
    obj.indexes.clear();
    let name = obj.name.clone();

    let all = flatten(&ws, &Default::default(), &Default::default(), true);
    let table = all.iter().find(|n| n.label.as_ref() == name).unwrap();
    let nodes = flatten(&ws, &[table.key.clone()].into(), &Default::default(), true);
    let at = nodes.iter().position(|n| n.key == table.key).unwrap();
    let children: Vec<_> = nodes[at + 1..]
        .iter()
        .take_while(|n| n.depth > table.depth)
        .map(|n| (n.label.to_string(), n.count))
        .collect();
    assert_eq!(
        children,
        [
            ("Columns".to_string(), Some(3)),
            ("Keys".to_string(), Some(1))
        ]
    );

    let conn = ws
        .connections
        .iter_mut()
        .find(|c| c.status == ConnectionStatus::Connected)
        .unwrap();
    let obj = conn.databases.as_mut().unwrap()[0]
        .schemas
        .as_mut()
        .unwrap()[0]
        .objects
        .iter_mut()
        .flatten()
        .find(|o| o.name == name)
        .unwrap();
    obj.columns.as_mut().unwrap().clear();
    obj.keys.clear();
    let nodes = flatten(&ws, &[table.key.clone()].into(), &Default::default(), true);
    let empty = nodes.iter().find(|n| n.key == table.key).unwrap();
    assert!(!empty.expandable, "nothing to show, no chevron");
}

#[test]
fn column_rows_show_type_and_pk() {
    let ws = &mock::workspaces()[0];
    let conn = ws
        .connections
        .iter()
        .find(|c| c.status == ConnectionStatus::Connected)
        .unwrap();
    let obj = conn.databases.as_ref().unwrap()[0]
        .schemas
        .as_ref()
        .unwrap()[0]
        .objects
        .iter()
        .flatten()
        .find(|o| o.kind == ObjectKind::Table)
        .unwrap();
    let table = flatten(ws, &Default::default(), &Default::default(), true)
        .into_iter()
        .find(|n| n.label.as_ref() == obj.name)
        .unwrap();
    let open = ["", "/Columns", "/Keys", "/Indexes"]
        .map(|part| format!("{}{part}", table.key))
        .into();
    let nodes = flatten(ws, &open, &Default::default(), true);
    let items = |part| {
        nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Item(part) && n.key.starts_with(&table.key))
            .collect::<Vec<_>>()
    };

    let columns = items(Part::Columns);
    let got: Vec<_> = columns
        .iter()
        .map(|n| {
            (
                n.label.to_string(),
                n.suffix.as_ref().map(|s| s.to_string()),
                n.pk,
            )
        })
        .collect();
    let want: Vec<_> = obj
        .columns
        .iter()
        .flatten()
        .map(|c| (c.name.clone(), Some(c.data_type.clone()), c.is_pk))
        .collect();
    assert_eq!(got, want);
    assert_eq!(
        got[0],
        ("id".to_string(), Some("bigint".to_string()), true),
        "id bigint PK"
    );
    assert!(got[1..].iter().all(|(_, _, pk)| !pk), "only id is a PK");

    let labels = |part| {
        items(part)
            .iter()
            .map(|n| n.label.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(Part::Keys),
        obj.keys
            .iter()
            .map(|k| match k {
                crate::model::Key::Primary { name, .. }
                | crate::model::Key::Foreign { name, .. }
                | crate::model::Key::Unique { name, .. } => name.clone(),
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        labels(Part::Indexes),
        obj.indexes
            .iter()
            .map(|i| i.name.clone())
            .collect::<Vec<_>>()
    );
}
