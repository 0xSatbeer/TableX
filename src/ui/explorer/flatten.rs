//! Flattening the workspace tree into the visible rows.

use super::*;

/// The rows under a structure part: label, muted suffix, primary key.
pub(super) fn part_rows(obj: &DbObject, part: Part) -> Vec<(&str, Option<&str>, bool)> {
    match part {
        Part::Columns => obj
            .columns
            .iter()
            .flatten()
            .map(|c| (c.name.as_str(), Some(c.data_type.as_str()), c.is_pk))
            .collect(),
        Part::Keys => obj
            .keys
            .iter()
            .map(|k| match k {
                Key::Primary { name, .. }
                | Key::Foreign { name, .. }
                | Key::Unique { name, .. } => (name.as_str(), None, false),
            })
            .collect(),
        Part::Indexes => obj
            .indexes
            .iter()
            .map(|i| (i.name.as_str(), None, false))
            .collect(),
    }
}

pub(super) fn part_label(part: Part) -> &'static str {
    match part {
        Part::Columns => "Columns",
        Part::Keys => "Keys",
        Part::Indexes => "Indexes",
    }
}

pub(super) fn group_label(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Table => "tables",
        ObjectKind::View => "views",
        ObjectKind::MaterializedView => "materialized views",
        ObjectKind::Sequence => "sequences",
        ObjectKind::Function => "functions",
        ObjectKind::Procedure => "procedures",
        ObjectKind::Type => "types",
        ObjectKind::Index => "indexes",
        ObjectKind::Trigger => "triggers",
    }
}

/// Test fixture: production open down to its first schema's tables. The app itself starts
/// collapsed, so every connection is visible and production is never the default context.
#[cfg(test)]
pub(super) fn default_expanded(ws: &Workspace) -> HashSet<String> {
    let mut set = HashSet::new();
    for conn in ws.connections.iter().filter(|c| c.is_production) {
        let key = conn.id.0.to_string();
        set.insert(key.clone());
        let db = conn.databases.iter().flatten().next();
        if let Some((db, schema)) =
            db.and_then(|db| Some((db, db.schemas.iter().flatten().next()?)))
        {
            let db = format!("{key}/{}", seg(&db.name));
            let schema_key = format!("{db}/{}", seg(&schema.name));
            if schema
                .objects
                .iter()
                .flatten()
                .any(|o| o.kind == ObjectKind::Table)
            {
                set.insert(format!("{schema_key}/{}", ObjectKind::Table.as_str()));
            }
            set.extend([db, schema_key]);
        }
    }
    set
}

/// Where a connection points, in the user's terms.
pub(super) fn target(conn: &Connection) -> String {
    match &conn.params {
        ConnectionParams::Postgres { host, port, .. } => format!("{host}:{port}"),
        ConnectionParams::Sqlite { path } => path.display().to_string(),
    }
}

/// Visible nodes in preorder. `all` opens every node (used while filtering). Expanded schemas
/// in `loads` show a single loading or error row instead of their children.
pub(super) fn flatten(
    ws: &Workspace,
    expanded: &HashSet<String>,
    loads: &HashMap<String, Load>,
    all: bool,
) -> Vec<FlatNode> {
    let mut out = Vec::new();
    let mut push = |mut node: FlatNode| {
        // Filtering opens everything but structure, so a query doesn't list every table's columns.
        let structure = matches!(node.kind, NodeKind::Object(_) | NodeKind::Part(_));
        node.expanded = node.expandable && (expanded.contains(&node.key) || all && !structure);
        let open = node.expanded;
        out.push(node);
        open
    };
    for (ci, conn) in ws.connections.iter().enumerate() {
        let node = |key: String, kind, label: &str, depth| FlatNode {
            key,
            kind,
            label: label.to_string().into(),
            depth,
            conn: ci,
            count: None,
            expandable: false,
            expanded: false,
            suffix: None,
            pk: false,
            unloaded: false,
        };
        let key = conn.id.0.to_string();
        // Expanding a connection that isn't connected connects it, even with no databases yet.
        let expandable = conn.databases.as_ref().is_none_or(|d| !d.is_empty())
            || conn.status != ConnectionStatus::Connected;
        let unloaded = conn.databases.is_none();
        let open = push(FlatNode {
            expandable,
            unloaded,
            ..node(
                key.clone(),
                NodeKind::Connection(conn.kind()),
                &conn.name,
                0,
            )
        });
        match &conn.status {
            ConnectionStatus::Connected => {}
            ConnectionStatus::Error(msg) => {
                push(node(
                    format!("{key}/{ERROR}"),
                    NodeKind::ConnError,
                    &format!("Could not connect: {msg}"),
                    1,
                ));
                continue;
            }
            ConnectionStatus::Connecting if open => {
                push(node(
                    format!("{key}/{LOADING}"),
                    NodeKind::Loading,
                    &format!("Connecting to {}…", target(conn)),
                    1,
                ));
                continue;
            }
            ConnectionStatus::Connecting | ConnectionStatus::Disconnected => continue,
        }
        if !open {
            continue;
        }
        // While a level loads, or once its load failed, one row says so in place of its children.
        if let Some((key, kind, label)) = load_row(loads, &key) {
            push(node(key, kind, &label, 1));
            continue;
        }
        for db in conn.databases.iter().flatten() {
            let key = format!("{key}/{}", seg(&db.name));
            let (expandable, unloaded) = (
                db.schemas.as_ref().is_none_or(|s| !s.is_empty()),
                db.schemas.is_none(),
            );
            if !push(FlatNode {
                expandable,
                unloaded,
                ..node(key.clone(), NodeKind::Database, &db.name, 1)
            }) {
                continue;
            }
            if let Some((key, kind, label)) = load_row(loads, &key) {
                push(node(key, kind, &label, 2));
                continue;
            }
            for schema in db.schemas.iter().flatten() {
                let key = format!("{key}/{}", seg(&schema.name));
                if !push(FlatNode {
                    expandable: true,
                    unloaded: schema.objects.is_none(),
                    ..node(key.clone(), NodeKind::Schema, &schema.name, 2)
                }) {
                    continue;
                }
                if let Some((key, kind, label)) = load_row(loads, &key) {
                    push(node(key, kind, &label, 3));
                    continue;
                }
                if schema.objects.as_ref().is_some_and(Vec::is_empty) {
                    push(node(
                        format!("{key}/{EMPTY}"),
                        NodeKind::Empty,
                        "No objects",
                        3,
                    ));
                    continue;
                }
                let mut groups: BTreeMap<ObjectKind, Vec<&DbObject>> = BTreeMap::new();
                for obj in schema.objects.iter().flatten() {
                    groups.entry(obj.kind).or_default().push(obj);
                }
                for (kind, objects) in groups {
                    let key = format!("{key}/{}", kind.as_str());
                    let group = node(key.clone(), NodeKind::Group(kind), group_label(kind), 3);
                    if !push(FlatNode {
                        count: Some(objects.len()),
                        expandable: true,
                        ..group
                    }) {
                        continue;
                    }
                    for obj in objects {
                        let key = format!("{key}/{}", seg(&obj.name));
                        let kind = NodeKind::Object(kind);
                        let parts = [
                            (Part::Columns, obj.columns.as_ref().map_or(0, Vec::len)),
                            (Part::Keys, obj.keys.len()),
                            (Part::Indexes, obj.indexes.len()),
                        ];
                        let parts: Vec<_> = parts.into_iter().filter(|(_, n)| *n > 0).collect();
                        let unloaded = opens_tab(kind) && obj.columns.is_none();
                        let expandable = opens_tab(kind) && !parts.is_empty() || unloaded;
                        if !push(FlatNode {
                            expandable,
                            unloaded,
                            suffix: None,
                            ..node(key.clone(), kind, &obj.name, 4)
                        }) {
                            continue;
                        }
                        if let Some((key, kind, label)) = load_row(loads, &key) {
                            push(node(key, kind, &label, 5));
                            continue;
                        }
                        for (part, n) in parts {
                            let label = part_label(part);
                            let key = format!("{key}/{label}");
                            if !push(FlatNode {
                                count: Some(n),
                                expandable: true,
                                ..node(key.clone(), NodeKind::Part(part), label, 5)
                            }) {
                                continue;
                            }
                            for (i, (label, suffix, pk)) in
                                part_rows(obj, part).into_iter().enumerate()
                            {
                                push(FlatNode {
                                    suffix: suffix.map(Into::into),
                                    pk,
                                    ..node(format!("{key}/{i}"), NodeKind::Item(part), label, 6)
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    out
}
