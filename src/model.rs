//! Domain types the UI binds to. Filled by `mock` today and by drivers in Phase 5.
//! No UI imports here.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: u32,
    pub name: String,
    /// A connection belongs to exactly one workspace, so the workspace owns it.
    pub connections: Vec<Connection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbKind {
    Postgres,
    Sqlite,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionParams {
    Postgres { host: String, port: u16, user: String, database: String, ssl: SslMode },
    Sqlite { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub id: ConnectionId,
    pub name: String,
    /// Index into the connection color palette (QX-002).
    pub color: u8,
    pub is_production: bool,
    pub status: ConnectionStatus,
    pub params: ConnectionParams,
    pub databases: Vec<Database>,
}

impl Connection {
    pub fn kind(&self) -> DbKind {
        match self.params {
            ConnectionParams::Postgres { .. } => DbKind::Postgres,
            ConnectionParams::Sqlite { .. } => DbKind::Sqlite,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Database {
    pub name: String,
    /// SQLite has one implicit `main` schema.
    pub schemas: Vec<Schema>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub name: String,
    pub objects: Vec<DbObject>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Sequence,
    Function,
    Procedure,
    Type,
    Index,
    Trigger,
}

impl ObjectKind {
    pub const ALL: [Self; 9] = [
        Self::Table,
        Self::View,
        Self::MaterializedView,
        Self::Sequence,
        Self::Function,
        Self::Procedure,
        Self::Type,
        Self::Index,
        Self::Trigger,
    ];

    /// Stable lowercase name, also the icon file stem.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::View => "view",
            Self::MaterializedView => "matview",
            Self::Sequence => "sequence",
            Self::Function => "function",
            Self::Procedure => "procedure",
            Self::Type => "type",
            Self::Index => "index",
            Self::Trigger => "trigger",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DbObject {
    pub kind: ObjectKind,
    pub name: String,
    /// Tables and views; empty for other kinds.
    pub columns: Vec<Column>,
    pub keys: Vec<Key>,
    pub indexes: Vec<Index>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub is_pk: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Primary { name: String, columns: Vec<String> },
    Foreign { name: String, columns: Vec<String>, ref_table: String, ref_columns: Vec<String> },
    Unique { name: String, columns: Vec<String> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColumnMeta {
    pub name: Arc<str>,
    pub data_type: Arc<str>,
}

pub type Row = Box<[Value]>;

#[derive(Clone, Debug, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Row>,
}

/// One cell. Kept at 24 bytes; format with `Display` only for visible cells.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(Arc<str>),
    Bytes(Arc<[u8]>),
    Json(Arc<str>),
    /// Microseconds since the Unix epoch, UTC, no zone (`timestamp`).
    Timestamp(i64),
    /// Microseconds since the Unix epoch plus the zone offset it is shown in (`timestamptz`).
    TimestampTz { micros: i64, offset_secs: i32 },
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("NULL"),
            Self::Bool(b) => f.write_str(if *b { "true" } else { "false" }),
            Self::Int(n) => write!(f, "{n}"),
            Self::Float(x) => write!(f, "{x}"),
            Self::Text(s) | Self::Json(s) => f.write_str(s),
            Self::Bytes(b) => {
                f.write_str("\\x")?;
                b.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
            }
            Self::Timestamp(micros) => write_timestamp(f, *micros),
            Self::TimestampTz { micros, offset_secs } => {
                write_timestamp(f, micros + *offset_secs as i64 * 1_000_000)?;
                let (sign, off) = if *offset_secs < 0 { ('-', -offset_secs) } else { ('+', *offset_secs) };
                write!(f, "{sign}{:02}", off / 3600)?;
                if off % 3600 != 0 {
                    write!(f, ":{:02}", off % 3600 / 60)?;
                }
                Ok(())
            }
        }
    }
}

/// Postgres style: `2026-01-02 10:42:01`, fractional seconds only when non-zero.
fn write_timestamp(f: &mut fmt::Formatter<'_>, micros: i64) -> fmt::Result {
    let secs = micros.div_euclid(1_000_000);
    let frac = micros.rem_euclid(1_000_000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let tod = secs.rem_euclid(86_400);
    write!(f, "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", tod / 3600, tod % 3600 / 60, tod % 60)?;
    if frac != 0 {
        let s = format!("{frac:06}");
        write!(f, ".{}", s.trim_end_matches('0'))?;
    }
    Ok(())
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

#[derive(Clone, Debug, PartialEq)]
pub enum QueryOutcome {
    Rows(ResultSet),
    Affected(u64),
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum QueryStatus {
    Ok,
    Err(String),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuerySource {
    Console,
    TableView,
    Introspection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryLogEntry {
    pub at: SystemTime,
    pub connection_id: ConnectionId,
    pub database: String,
    pub sql: String,
    pub duration: Duration,
    /// Rows returned or affected; `None` when the statement failed or was cancelled.
    pub rows: Option<u64>,
    pub status: QueryStatus,
    pub source: QuerySource,
}

#[cfg(test)]
mod tests {
    use super::*;

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
            (Value::Bytes(Arc::from(&[0xde, 0xad, 0x01][..])), "\\xdead01"),
            (Value::Timestamp(ts), "2026-01-02 10:42:01"),
            (Value::Timestamp(ts + 120_000), "2026-01-02 10:42:01.12"),
            (Value::Timestamp(-1), "1969-12-31 23:59:59.999999"),
            (Value::TimestampTz { micros: ts, offset_secs: 19_800 }, "2026-01-02 16:12:01+05:30"),
            (Value::TimestampTz { micros: ts, offset_secs: -8 * 3600 }, "2026-01-02 02:42:01-08"),
            (Value::Timestamp(951_782_400_000_000), "2000-02-29 00:00:00"),
        ];
        for (value, want) in cases {
            assert_eq!(value.to_string(), want);
        }
    }
}
