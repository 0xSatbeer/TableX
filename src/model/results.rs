use super::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnMeta {
    pub name: Arc<str>,
    pub data_type: Arc<str>,
    pub is_pk: bool,
    /// The labels of an enum column, in declared order; empty for every other type.
    pub enum_values: Arc<[Arc<str>]>,
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
    TimestampTz {
        micros: i64,
        offset_secs: i32,
    },
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
            Self::TimestampTz {
                micros,
                offset_secs,
            } => {
                write_timestamp(f, micros + *offset_secs as i64 * 1_000_000)?;
                let (sign, off) = if *offset_secs < 0 {
                    ('-', -offset_secs)
                } else {
                    ('+', *offset_secs)
                };
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
    write!(
        f,
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        tod / 3600,
        tod % 3600 / 60,
        tod % 60
    )?;
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
