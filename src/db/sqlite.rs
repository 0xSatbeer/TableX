//! The SQLite driver: rusqlite on the shared runtime's blocking pool.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::ready;

use super::{Driver, Reply, Session, blocking, foreign};
use crate::model::{Column, ConnectionParams, DbObject, ObjectKind, Role, TableDef};

pub struct Sqlite;

pub struct SqliteSession {
    /// The file name, shown as the session's one database.
    name: String,
    conn: Mutex<rusqlite::Connection>,
    /// Set by an I/O error (the file vanished or its drive failed), so the explorer drops the session.
    closed: AtomicBool,
}

impl SqliteSession {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// The reason for `err`, marking the session closed when it is an I/O error.
    fn fail(&self, err: rusqlite::Error) -> String {
        if err.sqlite_error_code() == Some(rusqlite::ErrorCode::SystemIoFailure) {
            self.closed.store(true, Ordering::Relaxed);
        }
        err.to_string()
    }
}

/// `ident` as a quoted SQLite identifier.
fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Opens the file `params` names.
fn open(params: &ConnectionParams) -> Result<SqliteSession, String> {
    let ConnectionParams::Sqlite { path } = params else {
        return Err("not a SQLite connection".into());
    };
    if !path.is_file() {
        return Err(format!("No file at {}", path.display()));
    }
    // Without SQLITE_OPEN_CREATE, so a typo never leaves an empty database behind.
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = rusqlite::Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    // Waits this long on a file another process holds locked (rusqlite's default, made explicit).
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    // Reads the header now, so a bad file fails at connect rather than at the first listing.
    conn.query_row("PRAGMA schema_version", [], |_| Ok(()))
        .map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    Ok(SqliteSession {
        name,
        conn: Mutex::new(conn),
        closed: AtomicBool::new(false),
    })
}

impl Driver for Sqlite {
    fn connect(&self, params: &ConnectionParams) -> Reply<Session> {
        let params = params.clone();
        blocking(move || Ok(Session::Sqlite(Arc::new(open(&params)?))))
    }

    fn test(&self, params: &ConnectionParams) -> Reply<()> {
        let params = params.clone();
        blocking(move || open(&params).map(drop))
    }

    fn databases(&self, session: &Session) -> Reply<Vec<String>> {
        let Session::Sqlite(session) = session else {
            return foreign();
        };
        ready(Ok(vec![session.name.clone()])).boxed()
    }

    fn schemas(&self, session: &Session) -> Reply<Vec<String>> {
        let Session::Sqlite(_) = session else {
            return foreign();
        };
        ready(Ok(vec!["main".to_string()])).boxed()
    }

    fn objects(&self, session: &Session, schema: &str) -> Reply<Vec<DbObject>> {
        let Session::Sqlite(session) = session else {
            return foreign();
        };
        let (session, schema) = (session.clone(), schema.to_string());
        blocking(move || {
            let sql = format!(
                "SELECT type, name FROM {}.sqlite_schema \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY type, name",
                quote(&schema)
            );
            let conn = session
                .conn
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut stmt = conn.prepare(&sql).map_err(|e| session.fail(e))?;
            let rows = stmt.query_map([], |row| {
                let kind = if row.get::<_, String>(0)? == "view" {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                };
                Ok(DbObject {
                    kind,
                    name: row.get(1)?,
                    columns: None,
                    keys: Vec::new(),
                    indexes: Vec::new(),
                    row_estimate: None,
                })
            });
            rows.and_then(Iterator::collect)
                .map_err(|e| session.fail(e))
        })
    }

    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>> {
        let Session::Sqlite(session) = session else {
            return foreign();
        };
        let (session, schema, object) = (session.clone(), schema.to_string(), object.to_string());
        blocking(move || {
            // table_xinfo also lists generated columns; hidden = 1 marks a virtual table's hidden ones.
            let sql = "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_xinfo(?1, ?2) WHERE hidden <> 1 ORDER BY cid";
            let conn = session
                .conn
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut stmt = conn.prepare(sql).map_err(|e| session.fail(e))?;
            let rows = stmt.query_map([&object, &schema], |row| {
                Ok(Column {
                    name: row.get(0)?,
                    data_type: row.get(1)?,
                    nullable: row.get::<_, i64>(2)? == 0,
                    default: row.get(3)?,
                    is_pk: row.get::<_, i64>(4)? > 0,
                    enum_values: Vec::new(),
                    identity: false,
                })
            });
            rows.and_then(Iterator::collect)
                .map_err(|e| session.fail(e))
        })
    }

    fn execute(&self, _: &Session, _: Vec<String>, _: bool) -> Reply<()> {
        unsupported()
    }

    fn roles(&self, _: &Session) -> Reply<Vec<Role>> {
        unsupported()
    }

    fn database_owner(&self, _: &Session) -> Reply<String> {
        unsupported()
    }

    fn schema_owner(&self, _: &Session, _: &str) -> Reply<String> {
        unsupported()
    }

    fn table_def(&self, _: &Session, _: &str, _: &str) -> Reply<TableDef> {
        unsupported()
    }
}

fn unsupported<T: Send + 'static>() -> Reply<T> {
    ready(Err("not supported on SQLite".into())).boxed()
}

#[cfg(test)]
mod tests;
