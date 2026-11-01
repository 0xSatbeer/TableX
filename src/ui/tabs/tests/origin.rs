use super::*;

#[test]
fn a_console_origin_uses_the_dialog_database_or_the_sqlite_file_name() {
    use crate::model::{
        Connection, ConnectionId, ConnectionParams, ConnectionStatus, Database, Schema, SslMode,
    };
    let pg = |database: &str| ConnectionParams::Postgres {
        host: "h".into(),
        port: 5432,
        user: "u".into(),
        password: None,
        database: database.into(),
        ssl: SslMode::Prefer,
    };
    let sqlite = |path: &str| ConnectionParams::Sqlite { path: path.into() };
    let listed = |names: &[&str]| {
        Some(
            names
                .iter()
                .map(|n| Database {
                    name: n.to_string(),
                    schemas: Some(vec![Schema {
                        name: "public".into(),
                        objects: None,
                    }]),
                })
                .collect(),
        )
    };
    // (params, databases listed, origin database)
    let cases = [
        (pg("app"), listed(&["analytics", "app"]), "app"),
        (pg("app"), None, "app"),
        (pg("app"), listed(&[]), "app"),
        (pg("app"), listed(&["analytics"]), "app"),
        (sqlite("/x/local.db"), listed(&["local.db"]), "local.db"),
        (sqlite("/x/local.db"), listed(&["main"]), "local.db"),
        (sqlite("/x/local.db"), None, "local.db"),
        (sqlite("notes"), None, "notes"),
    ];
    for (params, databases, want) in cases {
        let label = format!("{params:?} {databases:?}");
        let conn = Connection {
            save_password: false,
            id: ConnectionId(1),
            name: "c".into(),
            color: 0,
            is_production: false,
            status: ConnectionStatus::Connected,
            params,
            databases,
        };
        let origin = Origin::of(&conn);
        assert_eq!(
            (origin.tail.as_ref(), origin.path.as_ref()),
            (want, format!("c › {want}").as_str()),
            "{label}"
        );
    }
}

#[test]
fn tooltip_is_title_then_origin_path() {
    let origin = |path: &str| Origin {
        connection: crate::model::ConnectionId(0),
        color: 0,
        production: false,
        path: path.to_string().into(),
        tail: "".into(),
    };
    let cases = [
        (
            "users",
            Some(origin("prod-pg › app_db › public")),
            "users\nprod-pg › app_db › public",
        ),
        (
            "big_events",
            None,
            "big_events\nSample data · not from a connection",
        ),
        ("Console", Some(origin("dev")), "Console\ndev"),
        ("", None, "\nSample data · not from a connection"),
    ];
    for (title, o, want) in cases {
        assert_eq!(tooltip_text(title, o.as_ref()), want, "{title}");
    }
}

#[test]
fn refresh_origins_cases() {
    use crate::model::{Connection, Workspace};
    let ws = crate::mock::workspaces().remove(0);
    let prod = Origin::of(&ws.connections[0]);
    let local = Origin::of(&ws.connections[2]);
    let tail = prod
        .path
        .strip_prefix("prod-pg")
        .expect("path starts with the connection name")
        .to_string();
    assert!(
        tail.starts_with(" › "),
        "path has segments after the name: {:?}",
        prod.path
    );
    let edit = |f: fn(&mut Connection)| {
        let mut ws = ws.clone();
        f(&mut ws.connections[0]);
        ws
    };
    let with = |color: u8, production: bool, path: &str| Origin {
        color,
        production,
        path: path.to_string().into(),
        ..prod.clone()
    };
    let gone = Workspace {
        connections: ws.connections[1..].to_vec(),
        ..ws.clone()
    };
    // (case, workspace after the change, tab origin before, origin expected after)
    let cases: Vec<(&str, Workspace, Option<Origin>, Option<Origin>)> = vec![
        (
            "rename",
            edit(|c| c.name = "live".into()),
            Some(prod.clone()),
            Some(with(0, true, &format!("live{tail}"))),
        ),
        (
            "recolor",
            edit(|c| c.color = 3),
            Some(prod.clone()),
            Some(with(3, true, &prod.path)),
        ),
        (
            "production off",
            edit(|c| c.is_production = false),
            Some(prod.clone()),
            Some(with(0, false, &prod.path)),
        ),
        (
            "rename, recolor and production off",
            edit(|c| (c.name, c.color, c.is_production) = ("live".into(), 7, false)),
            Some(prod.clone()),
            Some(with(7, false, &format!("live{tail}"))),
        ),
        (
            "path is only the connection name",
            edit(|c| c.name = "live".into()),
            Some(Origin {
                path: "prod-pg".into(),
                tail: "".into(),
                ..prod.clone()
            }),
            Some(Origin {
                tail: "".into(),
                ..with(0, true, "live")
            }),
        ),
        (
            "old name contains the separator",
            edit(|c| c.name = "c".into()),
            Some(Origin::of(&Connection {
                name: "a › b".into(),
                ..ws.connections[0].clone()
            })),
            Some(with(0, true, &format!("c{tail}"))),
        ),
        (
            "new name contains the old one",
            edit(|c| c.name = "prod-pg-2".into()),
            Some(prod.clone()),
            Some(with(0, true, &format!("prod-pg-2{tail}"))),
        ),
        (
            "another connection's tab",
            edit(|c| (c.name, c.color) = ("live".into(), 7)),
            Some(local.clone()),
            Some(local.clone()),
        ),
        (
            "sample tab without an origin",
            edit(|c| c.name = "live".into()),
            None,
            None,
        ),
        (
            "connection no longer in the workspace",
            gone,
            Some(prod.clone()),
            Some(prod.clone()),
        ),
        (
            "nothing changed",
            ws.clone(),
            Some(prod.clone()),
            Some(prod.clone()),
        ),
    ];
    for (case, ws, before, expected) in cases {
        let mut tabs = Tabs::new(vec![
            EditorTab {
                origin: before.clone(),
                ..tab("open", false)
            },
            EditorTab {
                origin: before,
                ..tab("closed", false)
            },
        ]);
        tabs.close(1);
        tabs.refresh_origins(&ws);
        assert_eq!(tabs.tabs[0].origin, expected, "{case}: open tab");
        tabs.reopen();
        assert_eq!(
            tabs.tabs[1].origin, expected,
            "{case}: closed tab, once reopened"
        );
    }
}

#[test]
fn dropping_a_connection_keeps_a_full_reopen_history_of_other_connections() {
    use crate::model::ConnectionId;
    let on = |key: String, conn: u32| {
        let origin = Origin {
            connection: ConnectionId(conn),
            color: 0,
            production: false,
            path: "".into(),
            tail: "".into(),
        };
        EditorTab {
            key: key.clone().into(),
            title: key.into(),
            origin: Some(origin),
            tooltip: None,
            preview: false,
            pinned: false,
            content: (),
        }
    };
    let others: Vec<String> = (0..super::CLOSED_HISTORY)
        .map(|i| format!("other-{i}"))
        .collect();
    let mut t = Tabs::new(vec![
        on("gone-a".into(), 2),
        on("keep".into(), 1),
        on("gone-b".into(), 2),
    ]);
    for key in &others {
        t.open(on(key.clone(), 1));
        t.close(t.tabs.iter().position(|x| x.key == key.as_str()).unwrap());
    }
    t.drop_connection(ConnectionId(2));
    assert_eq!(keys(&t), ["keep"]);
    let mut reopened = Vec::new();
    while t.tabs.len() < 1 + super::CLOSED_HISTORY + 1 {
        let before = t.tabs.len();
        t.reopen();
        if t.tabs.len() == before {
            break;
        }
        reopened.push(t.tabs[t.active].key.to_string());
    }
    reopened.reverse();
    assert_eq!(
        reopened, others,
        "every other connection's closed tab is still reopenable"
    );
}

#[test]
fn same_title_from_different_connections_gets_the_connection_name() {
    use crate::model::ConnectionId;
    let on = |key: &'static str, title: &'static str, conn: Option<(u32, &str)>| EditorTab {
        title: title.into(),
        origin: conn.map(|(id, name)| Origin {
            connection: ConnectionId(id),
            color: 0,
            production: false,
            path: crate::ui::tabs::join_path(name, "app › public"),
            tail: "app › public".into(),
        }),
        ..tab(key, false)
    };
    let t = Tabs::new(vec![
        on("1", "users", Some((1, "prod"))),
        on("2", "users", Some((2, "local"))),
        on("3", "orders", Some((1, "prod"))),
        on("4", "orders", Some((1, "prod"))),
        on("5", "items", None),
        on("6", "items", Some((1, "prod"))),
    ]);
    let labels: Vec<_> = (0..t.tabs.len())
        .map(|ix| t.label(ix).to_string())
        .collect();
    assert_eq!(
        labels,
        [
            "users [prod]",
            "users [local]",
            "orders",
            "orders",
            "items",
            "items [prod]"
        ]
    );
}

#[test]
fn window_title_qualifies_tables_and_names_the_connection() {
    let o = Origin {
        connection: crate::model::ConnectionId(0),
        color: 0,
        production: false,
        path: "fastor dev › brainey-agents › public".into(),
        tail: "brainey-agents › public".into(),
    };
    let bare = Origin {
        tail: "".into(),
        path: "fastor dev".into(),
        ..o.clone()
    };
    let cases = [
        (
            Some(("agent", Some(&o))),
            true,
            "Work – brainey-agents.public.agent [fastor dev]",
        ),
        (
            Some(("brainey-agents — console", Some(&o))),
            false,
            "Work – brainey-agents — console [fastor dev]",
        ),
        (
            Some(("agent", Some(&bare))),
            true,
            "Work – agent [fastor dev]",
        ),
        (Some(("big_events", None)), true, "Work – big_events"),
        (None, false, "Work"),
    ];
    for (tab, object, want) in cases {
        assert_eq!(window_title("Work", tab, object), want);
    }
}
