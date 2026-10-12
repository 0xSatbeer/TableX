//! Deterministic fake data for the UI phase. Phase 5 deletes this file; nothing outside
//! `main.rs` wiring may depend on it.

use std::path::PathBuf;
use std::sync::Arc;
use std::cell::Cell;
use std::time::{Duration, UNIX_EPOCH};

use crate::model::*;

const SEED: u64 = 0x5155_4552_5958; // "QUERYX"
/// 2026-01-01 00:00:00 UTC. All generated times hang off this, never off the clock.
const BASE_MICROS: i64 = 1_767_225_600_000_000;

pub const BIG_EVENTS_ROWS: usize = 1_000_000;
pub const WIDE_TABLE_COLS: usize = 300;

/// SplitMix64: a stateless hash, so any (row, col) cell is O(1) and order-independent.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn cell_rand(row: usize, col: usize) -> u64 {
    mix(SEED ^ mix(row as u64) ^ (col as u64).rotate_left(32))
}

fn pick<T: Copy>(items: &[T], r: u64) -> T {
    items[(r % items.len() as u64) as usize]
}

thread_local! {
    // Per thread: only the UI thread reads it in the app, and each test sets its own without racing the others.
    static LATENCY_MS: Cell<u32> = const { Cell::new(0) };
}

/// Fake round-trip time (0–2000 ms). Callers wait this long off the UI thread before
/// delivering a mock result, to exercise loading states.
pub fn latency() -> Duration {
    Duration::from_millis(LATENCY_MS.get() as u64)
}

/// shortcut: mocked, replace with Driver::test (QX-023)
/// The caller waits `latency()` first, like `Explorer::connect`.
pub fn test_connection(host: &str) -> Result<(), String> {
    if host.to_ascii_lowercase().ends_with(".invalid") {
        Err(format!("Could not resolve host {host}"))
    } else {
        Ok(())
    }
}

pub fn set_latency(ms: u32) {
    LATENCY_MS.set(ms.min(2000));
}

// ---------------------------------------------------------------- schema tree

pub fn workspaces() -> Vec<Workspace> {
    let mut next_id = 0;
    let mut conn = |name: &str, color: u8, prod: bool, status, params, databases| {
        next_id += 1;
        Connection {
            id: ConnectionId(next_id),
            name: name.into(),
            color,
            is_production: prod,
            status,
            params,
            databases,
        }
    };
    let pg = |host: &str, db: &str| ConnectionParams::Postgres {
        host: host.into(),
        port: 5432,
        user: "app".into(),
        password: None,
        database: db.into(),
        ssl: SslMode::Prefer,
    };
    let sqlite = |path: &str| ConnectionParams::Sqlite { path: PathBuf::from(path) };
    use ConnectionStatus::*;

    vec![
        Workspace {
            id: 1,
            name: "Acme".into(),
            connections: vec![
                conn("prod-pg", 0, true, Connected, pg("db.acme.internal", "app_db"), pg_databases()),
                conn("staging-pg", 2, false, Error("timeout expired after 10 s".into()), pg("staging.acme.internal", "app_db"),
                    vec![database("app_db", vec![schema("public", 1, 40)])]),
                conn("local.db", 4, false, Disconnected, sqlite("~/acme/local.db"),
                    vec![database("main", vec![schema("main", 2, 12)])]),
            ],
        },
        Workspace {
            id: 2,
            name: "Side projects".into(),
            connections: vec![
                conn("dev-pg", 5, false, Error("connection refused (localhost:5432)".into()),
                    pg("localhost", "dev"), vec![database("dev", vec![schema("public", 3, 8)])]),
                conn("notes.db", 6, false, Connecting, sqlite("~/notes/notes.db"),
                    vec![database("main", vec![schema("main", 4, 5)])]),
            ],
        },
        Workspace {
            id: 3,
            name: "Analytics".into(),
            connections: vec![conn("warehouse-pg", 7, false, Disconnected, pg("wh.acme.internal", "warehouse"),
                vec![database("warehouse", vec![schema("marts", 5, 25)])])],
        },
    ]
}

/// prod-pg: 3 databases × 4 schemas; `app_db.public` holds the 2,000-table stress case.
fn pg_databases() -> Vec<Database> {
    let mut salt = 100;
    let mut schemas = |big: bool| {
        ["public", "billing", "audit", "staging"]
            .iter()
            .map(|name| {
                salt += 1;
                schema(name, salt, if big && *name == "public" { 2000 } else { 15 })
            })
            .collect()
    };
    vec![
        database("app_db", schemas(true)),
        database("analytics", schemas(false)),
        database("auth", schemas(false)),
    ]
}

fn database(name: &str, schemas: Vec<Schema>) -> Database {
    Database { name: name.into(), schemas }
}

const NOUNS: &[&str] = &[
    "users", "orders", "invoices", "payments", "products", "sessions", "events", "accounts",
    "teams", "projects", "tasks", "comments", "tags", "files", "plans", "coupons",
];

fn schema(name: &str, salt: u64, tables: usize) -> Schema {
    let mut objects: Vec<DbObject> = (0..tables)
        .map(|i| {
            let noun = NOUNS[i % NOUNS.len()];
            let name = if i < NOUNS.len() { noun.to_string() } else { format!("{noun}_{i}") };
            table(name, mix(salt ^ i as u64))
        })
        .collect();
    let plain = |kind, name: &str| DbObject {
        kind,
        name: name.into(),
        columns: vec![],
        keys: vec![],
        indexes: vec![],
        row_estimate: None,
    };
    for view in ["active_users", "daily_signups", "monthly_revenue", "order_summary", "top_customers"] {
        let mut v = table(view.into(), mix(salt ^ 0xFEED));
        v.kind = ObjectKind::View;
        v.keys.clear();
        v.indexes.clear();
        v.row_estimate = None;
        v.columns.iter_mut().for_each(|c| c.is_pk = false);
        objects.push(v);
    }
    objects.push(plain(ObjectKind::MaterializedView, "revenue_by_month"));
    for seq in ["invoices_id_seq", "orders_id_seq", "payments_id_seq", "users_id_seq"] {
        objects.push(plain(ObjectKind::Sequence, seq));
    }
    for func in ["calculate_tax(numeric)", "current_tenant()", "refresh_stats()", "touch_updated_at()"] {
        objects.push(plain(ObjectKind::Function, func));
    }
    objects.push(plain(ObjectKind::Procedure, "archive_old_orders(interval)"));
    objects.push(plain(ObjectKind::Type, "order_status"));
    Schema { name: name.into(), objects }
}

fn table(name: String, r: u64) -> DbObject {
    const EXTRA: &[(&str, &str)] = &[
        ("email", "text"),
        ("status", "text"),
        ("amount", "numeric(12,2)"),
        ("is_active", "boolean"),
        ("metadata", "jsonb"),
        ("owner_id", "bigint"),
        ("notes", "text"),
        ("updated_at", "timestamptz"),
    ];
    let col = |name: &str, ty: &str, nullable, default: Option<&str>, is_pk| Column {
        name: name.into(),
        data_type: ty.into(),
        nullable,
        default: default.map(Into::into),
        is_pk,
        enum_values: Vec::new(),
    };
    let mut columns = vec![
        col("id", "bigint", false, Some(&format!("nextval('{name}_id_seq')")), true),
        col("created_at", "timestamptz", false, Some("now()"), false),
    ];
    let extra = 2 + (r % 6) as usize;
    columns.extend(EXTRA.iter().take(extra).enumerate().map(|(i, (n, t))| col(n, t, (r >> i) & 1 == 1, None, false)));

    let mut keys = vec![Key::Primary { name: format!("{name}_pkey"), columns: vec!["id".into()] }];
    let mut indexes = vec![Index { name: format!("{name}_pkey"), columns: vec!["id".into()], unique: true }];
    if extra > 5 {
        keys.push(Key::Foreign {
            name: format!("{name}_owner_id_fkey"),
            columns: vec!["owner_id".into()],
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
        });
        indexes.push(Index { name: format!("{name}_owner_id_idx"), columns: vec!["owner_id".into()], unique: false });
    }
    DbObject { kind: ObjectKind::Table, name, columns, keys, indexes, row_estimate: Some(mix(r) % 3_000_000) }
}

// ---------------------------------------------------------------- result sets

fn meta(cols: &[(&str, &str)]) -> Vec<ColumnMeta> {
    cols.iter().map(|(n, t)| ColumnMeta { name: (*n).into(), data_type: (*t).into(), is_pk: *n == "id", ..Default::default() }).collect()
}

/// Labels of the `event_kind` enum that `big_events.event` is declared with.
const EVENT_KINDS: [&str; 6] = ["page_view", "signup", "login", "purchase", "logout", "error"];

pub fn big_events_columns() -> Vec<ColumnMeta> {
    let mut columns = meta(&[
        ("id", "bigint"),
        ("occurred_at", "timestamptz"),
        ("user_id", "bigint"),
        ("event", "event_kind"),
        ("amount", "double precision"),
        ("ok", "boolean"),
        ("payload", "jsonb"),
        ("country", "text"),
        ("session", "bytea"),
        ("duration_ms", "integer"),
        ("referrer", "text"),
        ("logged_at", "timestamp"),
    ]);
    columns[3].enum_values = EVENT_KINDS.map(Arc::from).into();
    columns
}

/// Row `ix` of `big_events`, computed on demand from the seed. Never stored.
pub fn big_events_row(ix: usize) -> Row {
    let r = |col| cell_rand(ix, col);
    let at = BASE_MICROS + ix as i64 * 2_500_000 + (r(1) % 1_000_000) as i64;
    let null_or = |col, v: Value| if r(col) % 13 == 0 { Value::Null } else { v };
    Box::new([
        Value::Int(ix as i64 + 1),
        Value::TimestampTz { micros: at, offset_secs: 0 },
        Value::Int((r(2) % 50_000) as i64 + 1),
        Value::Text(pick(&EVENT_KINDS, r(3)).into()),
        null_or(4, Value::Float(((r(4) % 2_000_000) as f64 - 500_000.) / 100.)),
        Value::Bool(r(5) % 10 != 0),
        Value::Json(format!(r#"{{"v":{},"path":"/p/{}"}}"#, r(6) % 4, r(6) % 977).into()),
        Value::Text(pick(&["DE", "IN", "US", "BR", "JP", "FR"], r(7)).into()),
        Value::Bytes(Arc::from(&r(8).to_be_bytes()[..])),
        Value::Int((r(9) % 5_000) as i64),
        null_or(10, Value::Text(pick(&["", "google.com", "news.ycombinator.com", "t.co"], r(10)).into())),
        Value::Timestamp(at),
    ])
}

/// `wide_table`: 300 columns for column-virtualization tests.
pub fn wide_table(rows: usize) -> ResultSet {
    let columns = (0..WIDE_TABLE_COLS)
        .map(|c| ColumnMeta { name: format!("col_{c:03}").into(), data_type: "integer".into(), is_pk: false, ..Default::default() })
        .collect();
    let rows = (0..rows)
        .map(|row| (0..WIDE_TABLE_COLS).map(|col| Value::Int((cell_rand(row, col) % 100_000) as i64)).collect())
        .collect();
    ResultSet { columns, rows }
}

/// One row per awkward cell the grid has to render well.
pub fn edge_cases() -> ResultSet {
    let text = |s: &str| Value::Text(s.into());
    let cases = [
        ("null", Value::Null),
        ("empty string", text("")),
        ("10 KB text", text(&"lorem ipsum dolor sit amet ".repeat(380))),
        ("multiline", text("first line\nsecond line\n\tindented third")),
        ("emoji", text("ship it 🚀🔥 👩‍💻")),
        ("rtl", text("مرحبا بالعالم")),
        ("cjk", text("数据库工具 データベース 데이터베이스")),
        ("json", Value::Json(r#"{"id": 7, "tags": ["a", "b"], "nested": {"ok": true}}"#.into())),
        ("bytea", Value::Bytes(Arc::from(&[0x00, 0xff, 0x10, 0x80][..]))),
        ("negative int", Value::Int(i64::MIN)),
        ("negative float", Value::Float(-0.000_123)),
        ("timestamptz +05:30", Value::TimestampTz { micros: BASE_MICROS, offset_secs: 19_800 }),
        ("timestamptz -08", Value::TimestampTz { micros: BASE_MICROS, offset_secs: -28_800 }),
        ("bool", Value::Bool(false)),
    ];
    ResultSet {
        columns: meta(&[("case", "text"), ("value", "any")]),
        rows: cases.into_iter().map(|(name, v)| Box::new([text(name), v]) as Row).collect(),
    }
}

// ---------------------------------------------------------------- queries

const LOG_SQL: &[&str] = &[
    "SELECT * FROM users LIMIT 500",
    "SELECT id, email FROM users WHERE created_at > now() - interval '7 days'",
    "UPDATE orders SET status = 'shipped' WHERE id = 4211",
    "SELECT count(*) FROM big_events",
    "DELETE FROM sessions WHERE expires_at < now()",
    "SELECT * FROM pg_catalog.pg_class",
    "SELEC * FROM orders",
];

/// `n` past log entries, oldest first, one every 37 s from the base time.
pub fn mock_query_log(n: usize) -> Vec<QueryLogEntry> {
    // run_query builds whole result sets; run each distinct statement once, not once per entry.
    let outcomes: Vec<_> = LOG_SQL
        .iter()
        .map(|sql| match run_query(sql) {
            QueryOutcome::Rows(rs) => (QueryStatus::Ok, Some(rs.rows.len() as u64)),
            QueryOutcome::Affected(n) => (QueryStatus::Ok, Some(n)),
            QueryOutcome::Error(e) => (QueryStatus::Err(e), None),
        })
        .collect();
    (0..n)
        .map(|i| {
            let r = cell_rand(i, 99);
            let k = (r % LOG_SQL.len() as u64) as usize;
            let sql = LOG_SQL[k];
            let (status, rows) = outcomes[k].clone();
            let (status, rows) = if r % 29 == 0 { (QueryStatus::Cancelled, None) } else { (status, rows) };
            QueryLogEntry {
                at: UNIX_EPOCH + Duration::from_micros(BASE_MICROS as u64) + Duration::from_secs(37 * i as u64),
                connection_id: ConnectionId(1 + (r >> 8) as u32 % 2),
                database: "app_db".into(),
                sql: sql.into(),
                duration: Duration::from_micros(200 + (r >> 16) % 250_000),
                rows,
                status,
                source: if sql.contains("pg_catalog") {
                    QuerySource::Introspection
                } else {
                    pick(&[QuerySource::Console, QuerySource::TableView], r >> 24)
                },
            }
        })
        .collect()
}

/// Fake execution, decided by the leading keyword: reads return rows, writes report
/// affected rows, anything unrecognised is a syntax error like Postgres would give.
pub fn run_query(sql: &str) -> QueryOutcome {
    let first = sql.split_whitespace().next().unwrap_or_default();
    let h = mix(SEED ^ sql.bytes().fold(0, |acc, b| mix(acc ^ b as u64)));
    match first.to_ascii_lowercase().as_str() {
        "" => QueryOutcome::Error("empty query".into()),
        "select" | "with" | "values" | "table" | "explain" | "show" => {
            let rows = (h % 200) as usize;
            QueryOutcome::Rows(ResultSet {
                columns: big_events_columns(),
                rows: (0..rows).map(|i| big_events_row((h as usize).wrapping_add(i) % BIG_EVENTS_ROWS)).collect(),
            })
        }
        "insert" | "update" | "delete" | "merge" => QueryOutcome::Affected(h % 1_000),
        "create" | "drop" | "alter" | "truncate" | "begin" | "commit" | "rollback" | "set" => QueryOutcome::Affected(0),
        _ => QueryOutcome::Error(format!("syntax error at or near \"{first}\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_output() {
        assert_eq!(workspaces(), workspaces());
        assert_eq!(mock_query_log(200), mock_query_log(200));
        assert_eq!(wide_table(5), wide_table(5));
        assert_eq!(run_query("select 1"), run_query("select 1"));
        // Pinned values catch changes that are deterministic within a run but not across versions.
        assert_eq!(big_events_row(0)[0], Value::Int(1));
        assert_eq!(big_events_row(999_999)[0], Value::Int(1_000_000));
        assert_eq!(big_events_row(999_999), big_events_row(999_999));
    }

    #[test]
    fn last_big_events_row_is_o1() {
        let start = std::time::Instant::now();
        let row = big_events_row(BIG_EVENTS_ROWS - 1);
        assert!(start.elapsed() < Duration::from_millis(50));
        assert_eq!(row.len(), big_events_columns().len());
    }

    #[test]
    fn mock_shape_matches_ticket() {
        let ws = workspaces();
        let conns: Vec<_> = ws.iter().flat_map(|w| &w.connections).collect();
        assert_eq!(ws.len(), 3);
        assert_eq!(conns.len(), 6);
        assert_eq!(conns.iter().filter(|c| c.is_production).count(), 1);
        let ids: std::collections::HashSet<_> = conns.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 6, "connection ids are unique");

        let prod = conns.iter().find(|c| c.is_production).unwrap();
        assert_eq!(prod.kind(), DbKind::Postgres);
        assert_eq!(prod.databases.len(), 3);
        assert!(prod.databases.iter().all(|d| d.schemas.len() == 4));
        let tables = |s: &Schema| s.objects.iter().filter(|o| o.kind == ObjectKind::Table).count();
        assert_eq!(tables(&prod.databases[0].schemas[0]), 2000);
        assert!(conns.iter().any(|c| c.kind() == DbKind::Sqlite && c.databases.len() == 1));

        assert_eq!(wide_table(1).columns.len(), 300);
        assert!(edge_cases().rows.iter().any(|r| r[1] == Value::Null));
    }

    fn all_objects() -> Vec<DbObject> {
        workspaces()
            .into_iter()
            .flat_map(|w| w.connections)
            .flat_map(|c| c.databases)
            .flat_map(|d| d.schemas)
            .flat_map(|s| s.objects)
            .collect()
    }

    #[test]
    fn tables_have_row_estimates_and_nothing_else_does() {
        for o in all_objects() {
            let want = o.kind == ObjectKind::Table;
            assert_eq!(o.row_estimate.is_some(), want, "{:?} {}: estimate presence", o.kind, o.name);
        }
    }

    #[test]
    fn only_id_columns_are_primary_keys() {
        for o in all_objects() {
            for c in &o.columns {
                let want = o.kind == ObjectKind::Table && c.name == "id";
                assert_eq!(c.is_pk, want, "{}.{}: is_pk", o.name, c.name);
            }
        }
        let result_sets = [big_events_columns(), wide_table(0).columns, edge_cases().columns];
        for c in result_sets.iter().flatten() {
            assert_eq!(c.is_pk, &*c.name == "id", "result column {}: is_pk", c.name);
        }
    }

    #[test]
    fn run_query_picks_outcome_from_sql() {
        assert!(matches!(run_query("  SELECT * FROM users"), QueryOutcome::Rows(_)));
        assert!(matches!(run_query("update t set a = 1"), QueryOutcome::Affected(_)));
        assert_eq!(run_query("SELEC 1"), QueryOutcome::Error("syntax error at or near \"SELEC\"".into()));
    }

    #[test]
    fn latency_is_clamped() {
        set_latency(5000);
        assert_eq!(latency(), Duration::from_secs(2));
        set_latency(0);
    }
}
