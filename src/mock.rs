//! Deterministic fake data for the UI phase. Phase 5 deletes this file; nothing outside
//! `main.rs` wiring may depend on it.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
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

/// The Mock driver's Test: a host ending in `.invalid` fails to resolve.
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

mod schema;

pub use schema::workspaces;

// ---------------------------------------------------------------- result sets

fn meta(cols: &[(&str, &str)]) -> Vec<ColumnMeta> {
    cols.iter()
        .map(|(n, t)| ColumnMeta {
            name: (*n).into(),
            data_type: (*t).into(),
            is_pk: *n == "id",
            ..Default::default()
        })
        .collect()
}

/// Labels of the `event_kind` enum that `big_events.event` is declared with.
const EVENT_KINDS: [&str; 6] = [
    "page_view",
    "signup",
    "login",
    "purchase",
    "logout",
    "error",
];

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
        Value::TimestampTz {
            micros: at,
            offset_secs: 0,
        },
        Value::Int((r(2) % 50_000) as i64 + 1),
        Value::Text(pick(&EVENT_KINDS, r(3)).into()),
        null_or(
            4,
            Value::Float(((r(4) % 2_000_000) as f64 - 500_000.) / 100.),
        ),
        Value::Bool(r(5) % 10 != 0),
        Value::Json(format!(r#"{{"v":{},"path":"/p/{}"}}"#, r(6) % 4, r(6) % 977).into()),
        Value::Text(pick(&["DE", "IN", "US", "BR", "JP", "FR"], r(7)).into()),
        Value::Bytes(Arc::from(&r(8).to_be_bytes()[..])),
        Value::Int((r(9) % 5_000) as i64),
        null_or(
            10,
            Value::Text(pick(&["", "google.com", "news.ycombinator.com", "t.co"], r(10)).into()),
        ),
        Value::Timestamp(at),
    ])
}

/// `wide_table`: 300 columns for column-virtualization tests.
pub fn wide_table(rows: usize) -> ResultSet {
    let columns = (0..WIDE_TABLE_COLS)
        .map(|c| ColumnMeta {
            name: format!("col_{c:03}").into(),
            data_type: "integer".into(),
            is_pk: false,
            ..Default::default()
        })
        .collect();
    let rows = (0..rows)
        .map(|row| {
            (0..WIDE_TABLE_COLS)
                .map(|col| Value::Int((cell_rand(row, col) % 100_000) as i64))
                .collect()
        })
        .collect();
    ResultSet { columns, rows }
}

/// One row per awkward cell the grid has to render well.
pub fn edge_cases() -> ResultSet {
    let text = |s: &str| Value::Text(s.into());
    let cases = [
        ("null", Value::Null),
        ("empty string", text("")),
        (
            "10 KB text",
            text(&"lorem ipsum dolor sit amet ".repeat(380)),
        ),
        (
            "multiline",
            text("first line\nsecond line\n\tindented third"),
        ),
        ("emoji", text("ship it 🚀🔥 👩‍💻")),
        ("rtl", text("مرحبا بالعالم")),
        ("cjk", text("数据库工具 データベース 데이터베이스")),
        (
            "json",
            Value::Json(r#"{"id": 7, "tags": ["a", "b"], "nested": {"ok": true}}"#.into()),
        ),
        (
            "bytea",
            Value::Bytes(Arc::from(&[0x00, 0xff, 0x10, 0x80][..])),
        ),
        ("negative int", Value::Int(i64::MIN)),
        ("negative float", Value::Float(-0.000_123)),
        (
            "timestamptz +05:30",
            Value::TimestampTz {
                micros: BASE_MICROS,
                offset_secs: 19_800,
            },
        ),
        (
            "timestamptz -08",
            Value::TimestampTz {
                micros: BASE_MICROS,
                offset_secs: -28_800,
            },
        ),
        ("bool", Value::Bool(false)),
    ];
    ResultSet {
        columns: meta(&[("case", "text"), ("value", "any")]),
        rows: cases
            .into_iter()
            .map(|(name, v)| Box::new([text(name), v]) as Row)
            .collect(),
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
            let (status, rows) = if r.is_multiple_of(29) {
                (QueryStatus::Cancelled, None)
            } else {
                (status, rows)
            };
            QueryLogEntry {
                at: UNIX_EPOCH
                    + Duration::from_micros(BASE_MICROS as u64)
                    + Duration::from_secs(37 * i as u64),
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
                rows: (0..rows)
                    .map(|i| big_events_row((h as usize).wrapping_add(i) % BIG_EVENTS_ROWS))
                    .collect(),
            })
        }
        "insert" | "update" | "delete" | "merge" => QueryOutcome::Affected(h % 1_000),
        "create" | "drop" | "alter" | "truncate" | "begin" | "commit" | "rollback" | "set" => {
            QueryOutcome::Affected(0)
        }
        _ => QueryOutcome::Error(format!("syntax error at or near \"{first}\"")),
    }
}

#[cfg(test)]
mod tests;
