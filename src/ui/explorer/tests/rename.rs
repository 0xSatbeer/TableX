//! Inline rename of connections.

use super::*;

#[test]
fn validate_rename_cases() {
    let others = ["local.db", "staging-pg"];
    let cases = [
        ("  prod-main ", Ok("prod-main")),
        ("", Err("Name can't be empty")),
        ("   ", Err("Name can't be empty")),
        (
            "local.db",
            Err("Another connection is already named local.db"),
        ),
        (
            " local.db ",
            Err("Another connection is already named local.db"),
        ),
        ("Prod-PG", Ok("Prod-PG")),
        ("Local.db", Ok("Local.db")),
    ];
    for (input, want) in cases {
        let want = want.map(String::from).map_err(String::from);
        assert_eq!(validate_rename(input, &others), want, "input {input:?}");
    }
}

#[test]
fn rename_keys() {
    let others = ["local.db"];
    let mut rn = Rename {
        conn: 0,
        text: "prod-pg".into(),
        error: None,
    };
    // Typing edits the buffer and the input stays open.
    assert_eq!(rn.key("x", Some("x"), &others), RenameKey::Open);
    assert_eq!(rn.text, "prod-pgx");
    assert_eq!(rn.key("backspace", None, &others), RenameKey::Open);
    assert_eq!(rn.text, "prod-pg");
    // Enter commits the trimmed name.
    rn.text = "  prod-main ".into();
    assert_eq!(
        rn.key("enter", None, &others),
        RenameKey::Commit("prod-main".into())
    );
    // A rejected name keeps the input open with the message; typing clears it.
    rn.text = "local.db".into();
    assert_eq!(rn.key("enter", None, &others), RenameKey::Open);
    assert_eq!(
        rn.error.as_deref(),
        Some("Another connection is already named local.db")
    );
    rn.text.clear();
    assert_eq!(rn.key("enter", None, &others), RenameKey::Open);
    assert_eq!(rn.error.as_deref(), Some("Name can't be empty"));
    assert_eq!(rn.key("a", Some("a"), &others), RenameKey::Open);
    assert_eq!(rn.error, None);
    // Esc cancels.
    assert_eq!(rn.key("escape", None, &others), RenameKey::Cancel);
}

#[test]
fn rename_to_own_name_is_allowed() {
    let ws = &mock::workspaces()[0];
    let own = ws.connections[0].name.clone();
    let others = super::other_names(ws, 0);
    assert!(!others.contains(&own.as_str()));
    assert_eq!(others.len(), ws.connections.len() - 1);
    assert_eq!(validate_rename(&own, &others), Ok(own.clone()));
}

#[test]
fn renamed_connection_shows_under_filter() {
    let mut ws = mock::workspaces()[0].clone();
    let filter = "prod";
    ws.connections[0].name = "prod-main".into();
    let nodes = tree_nodes(&ws, &Default::default(), &Default::default(), filter);
    let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_ref()).collect();
    assert!(
        labels.contains(&ws.connections[0].name.as_str()),
        "{labels:?}"
    );
    assert!(
        !labels.contains(&ws.connections[2].name.as_str()),
        "filter still applied: {labels:?}"
    );
}

#[test]
fn rename_lets_chords_through() {
    use gpui_kit::Modifiers;
    assert!(rename_swallows(Modifiers::none()));
    assert!(rename_swallows(Modifiers::shift()));
    assert!(!rename_swallows(Modifiers::control()));
    assert!(!rename_swallows(Modifiers::alt()));
    assert!(!rename_swallows(Modifiers::command()));
}

#[test]
fn f2_renames_connections_only() {
    let cases = [
        (NodeKind::Connection(DbKind::Postgres), true),
        (NodeKind::Connection(DbKind::Sqlite), true),
        (NodeKind::Database, false),
        (NodeKind::Schema, false),
        (NodeKind::Group(ObjectKind::Table), false),
        (NodeKind::Object(ObjectKind::Table), false),
        (NodeKind::Object(ObjectKind::View), false),
        (NodeKind::Loading, false),
        (NodeKind::ConnError, false),
    ];
    for (kind, want) in cases {
        assert_eq!(can_rename(kind), want, "{kind:?}");
    }
}
