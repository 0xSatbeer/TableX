use super::*;

#[test]
fn delete_and_edit_cases() {
    let ws = crate::mock::workspaces().remove(0);
    let ids: Vec<ConnectionId> = ws.connections.iter().map(|c| c.id).collect();
    let names = |ws: &Workspace| {
        ws.connections
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
    };
    // (id deleted, position returned, names left)
    let cases = [
        (ids[1], Some(1), vec!["prod-pg", "local.db"]),
        (ids[0], Some(0), vec!["staging-pg", "local.db"]),
        (ids[2], Some(2), vec!["prod-pg", "staging-pg"]),
        (
            ConnectionId(99),
            None,
            vec!["prod-pg", "staging-pg", "local.db"],
        ),
    ];
    for (id, at, left) in cases {
        let mut after = ws.clone();
        assert_eq!(after.delete(id), at, "{id:?}");
        assert_eq!(names(&after), left, "{id:?}");
    }
    let mut last = Workspace {
        connections: ws.connections[..1].to_vec(),
        ..ws.clone()
    };
    assert_eq!(last.delete(ids[0]), Some(0));
    assert!(last.connections.is_empty(), "deleting the last leaves []");
    let mut gone = ws.clone();
    gone.edit(
        ConnectionId(99),
        "x".into(),
        1,
        true,
        ConnectionParams::Sqlite {
            path: PathBuf::from("x"),
        },
    );
    assert_eq!(gone, ws, "an edit of a deleted connection does nothing");
}

#[test]
fn secret_debug_is_masked_and_a_password_only_edit_disconnects() {
    assert_eq!(format!("{:?}", Secret::new("hunter2")), "***");
    let pg = |password: Option<&str>| ConnectionParams::Postgres {
        host: "h".into(),
        port: 5432,
        user: "u".into(),
        password: password.map(Secret::new),
        database: "d".into(),
        ssl: SslMode::Prefer,
    };
    assert!(
        !format!("{:?}", pg(Some("hunter2"))).contains("hunter2"),
        "params Debug hides the password"
    );
    // (old password, new password, disconnected by the edit)
    let cases = [
        (None, Some("a"), true),
        (Some("a"), Some("b"), true),
        (Some("a"), None, true),
        (Some("a"), Some(""), true),
        (Some("a"), Some("a"), false),
        (None, None, false),
    ];
    for (old, new, disconnects) in cases {
        let mut ws = Workspace {
            id: 1,
            name: "w".into(),
            connections: Vec::new(),
        };
        let id = ws.add("c".into(), 0, false, pg(old));
        ws.connections[0].status = ConnectionStatus::Connected;
        ws.connections[0].databases = Some(vec![Database {
            name: "d".into(),
            schemas: None,
        }]);
        ws.edit(id, "c".into(), 0, false, pg(new));
        let conn = &ws.connections[0];
        assert_eq!(
            conn.status == ConnectionStatus::Disconnected,
            disconnects,
            "{old:?} -> {new:?}"
        );
        assert_eq!(conn.databases.is_none(), disconnects, "{old:?} -> {new:?}");
        assert_eq!(conn.params, pg(new), "{old:?} -> {new:?}");
    }
}

#[test]
fn value_stays_compact() {
    assert_eq!(size_of::<Value>(), 24);
}

#[test]
fn values_display_like_postgres() {
    let ts = 1_767_350_521_000_000; // 2026-01-02 10:42:01 UTC
    let cases = [
        (Value::Null, "NULL"),
        (Value::Int(-42), "-42"),
        (
            Value::Bytes(Arc::from(&[0xde, 0xad, 0x01][..])),
            "\\xdead01",
        ),
        (Value::Timestamp(ts), "2026-01-02 10:42:01"),
        (Value::Timestamp(ts + 120_000), "2026-01-02 10:42:01.12"),
        (Value::Timestamp(-1), "1969-12-31 23:59:59.999999"),
        (
            Value::TimestampTz {
                micros: ts,
                offset_secs: 19_800,
            },
            "2026-01-02 16:12:01+05:30",
        ),
        (
            Value::TimestampTz {
                micros: ts,
                offset_secs: -8 * 3600,
            },
            "2026-01-02 02:42:01-08",
        ),
        (Value::Timestamp(951_782_400_000_000), "2000-02-29 00:00:00"),
    ];
    for (value, want) in cases {
        assert_eq!(value.to_string(), want);
    }
}
