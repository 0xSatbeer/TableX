//! `logs/queries.jsonl` (C9): the query log, one JSON object per line, newest last.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::{ConnectionId, QueryLogEntry, QuerySource, QueryStatus};

/// Lines kept when the log is loaded; older ones are dropped from the file.
pub const QUERY_LOG_KEEP: usize = 5000;

#[derive(Serialize, Deserialize)]
struct Line {
    at_ms: u64,
    connection: u32,
    database: String,
    sql: String,
    ms: u64,
    rows: Option<u64>,
    status: Status,
    error: Option<String>,
    source: Source,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Ok,
    Error,
    Cancelled,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Source {
    Console,
    Table,
    Introspection,
}

pub fn query_log_path(root: &Path) -> PathBuf {
    root.join("logs").join("queries.jsonl")
}

/// `entry` as its C9 line, without the newline.
pub fn query_log_line(entry: &QueryLogEntry) -> String {
    let (status, error) = match &entry.status {
        QueryStatus::Ok => (Status::Ok, None),
        QueryStatus::Err(e) => (Status::Error, Some(e.clone())),
        QueryStatus::Cancelled => (Status::Cancelled, None),
    };
    let line = Line {
        at_ms: entry
            .at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        connection: entry.connection_id.0,
        database: entry.database.clone(),
        sql: entry.sql.clone(),
        ms: entry.duration.as_millis() as u64,
        rows: entry.rows,
        status,
        error,
        source: match entry.source {
            QuerySource::Console => Source::Console,
            QuerySource::TableView => Source::Table,
            QuerySource::Introspection => Source::Introspection,
        },
    };
    serde_json::to_string(&line).expect("log line serializes")
}

fn parse_line(text: &str) -> Option<QueryLogEntry> {
    let line: Line = serde_json::from_str(text).ok()?;
    Some(QueryLogEntry {
        at: UNIX_EPOCH + Duration::from_millis(line.at_ms),
        connection_id: ConnectionId(line.connection),
        database: line.database,
        sql: line.sql,
        duration: Duration::from_millis(line.ms),
        rows: line.rows,
        status: match line.status {
            Status::Ok => QueryStatus::Ok,
            Status::Error => QueryStatus::Err(line.error.unwrap_or_default()),
            Status::Cancelled => QueryStatus::Cancelled,
        },
        source: match line.source {
            Source::Console => QuerySource::Console,
            Source::Table => QuerySource::TableView,
            Source::Introspection => QuerySource::Introspection,
        },
    })
}

/// Appends `entry` as the newest line.
pub fn append_query_log(root: &Path, entry: &QueryLogEntry) -> std::io::Result<()> {
    let path = query_log_path(root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    // One write per line, so concurrent appends never interleave mid-line.
    file.write_all(format!("{}\n", query_log_line(entry)).as_bytes())
}

/// The newest `QUERY_LOG_KEEP` entries, oldest first; malformed lines are skipped. Past the
/// limit the file is rewritten without the oldest lines.
pub fn load_query_log(root: &Path) -> Vec<QueryLogEntry> {
    let path = query_log_path(root);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let kept = &lines[lines.len().saturating_sub(QUERY_LOG_KEEP)..];
    if kept.len() < lines.len() {
        let _ = super::write_atomic(&path, (kept.join("\n") + "\n").as_bytes());
    }
    kept.iter().filter_map(|l| parse_line(l)).collect()
}
