use super::{Found, index, ranked};
use crate::model::{
    Connection, ConnectionId, ConnectionParams, ConnectionStatus, Database, DbObject, ObjectKind,
    Schema, Workspace,
};

fn object(kind: ObjectKind, name: &str) -> DbObject {
    DbObject {
        kind,
        name: name.into(),
        columns: None,
        keys: vec![],
        indexes: vec![],
        row_estimate: None,
    }
}

fn connection(id: u32, status: ConnectionStatus, objects: Vec<DbObject>) -> Connection {
    Connection {
        save_password: false,
        id: ConnectionId(id),
        name: format!("c{id}"),
        color: 2,
        is_production: id == 1,
        status,
        params: ConnectionParams::Sqlite { path: "x".into() },
        databases: Some(vec![Database {
            name: "app".into(),
            schemas: Some(vec![
                Schema {
                    name: "public".into(),
                    objects: Some(objects),
                },
                Schema {
                    name: "unloaded".into(),
                    objects: None,
                },
            ]),
        }]),
    }
}

fn names(all: &[Found], query: &str) -> Vec<String> {
    ranked(all, query)
        .into_iter()
        .map(|(ix, _)| all[ix].name.clone())
        .collect()
}

#[test]
fn indexes_tables_views_and_functions_of_connected_connections_only() {
    let ws = Workspace {
        id: 1,
        name: "w".into(),
        id_floor: 0,
        connections: vec![
            connection(
                1,
                ConnectionStatus::Connected,
                vec![
                    object(ObjectKind::Table, "users"),
                    object(ObjectKind::MaterializedView, "totals"),
                    object(ObjectKind::Function, "now_utc"),
                    object(ObjectKind::Sequence, "users_id_seq"),
                ],
            ),
            connection(
                2,
                ConnectionStatus::Disconnected,
                vec![object(ObjectKind::Table, "stale")],
            ),
        ],
    };
    let all = index(&ws);
    let got: Vec<_> = all.iter().map(|f| (f.kind, f.name.as_str())).collect();
    assert_eq!(
        got,
        [
            (ObjectKind::Table, "users"),
            (ObjectKind::MaterializedView, "totals"),
            (ObjectKind::Function, "now_utc"),
        ]
    );
    assert_eq!(all[0].key, "1/app/public/table/users");
    assert_eq!(all[0].place, "c1 / app / public");
    assert!(all[0].production);
}

#[test]
fn typing_usr_ranks_users_above_user_settings_archive() {
    let ws = Workspace {
        id: 1,
        name: "w".into(),
        id_floor: 0,
        connections: vec![connection(
            1,
            ConnectionStatus::Connected,
            vec![
                object(ObjectKind::Table, "user_settings_archive"),
                object(ObjectKind::Table, "orders"),
                object(ObjectKind::Table, "users"),
            ],
        )],
    };
    let all = index(&ws);
    assert_eq!(names(&all, "usr"), ["users", "user_settings_archive"]);
}

/// Benchmark, ignored because wall-clock bounds flake when the suite runs in parallel. Run it
/// with `cargo test --release scoring_5000 -- --ignored`: the budget is 8 ms (measured ~4 ms).
#[test]
#[ignore]
fn scoring_5000_candidates_for_a_4_char_query_is_fast() {
    let all: Vec<Found> = (0..5000)
        .map(|i| Found {
            key: String::new(),
            kind: ObjectKind::Table,
            name: format!("customer_order_line_{i}"),
            place: String::new(),
            color: 0,
            production: false,
        })
        .collect();
    let start = std::time::Instant::now();
    let hits = ranked(&all, "cust");
    let took = start.elapsed();
    assert_eq!(hits.len(), 5000);
    let budget = if cfg!(debug_assertions) { 200 } else { 8 };
    assert!(
        took < std::time::Duration::from_millis(budget),
        "took {took:?}"
    );
}
