//! The SQLite driver: rusqlite on the shared runtime's blocking pool.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::{BoxFuture, ready};

use super::{Driver, Opened, QueryReply, Reply, Session, blocking, foreign, foreign_query};
use crate::model::{
    Column, ColumnMeta, ConnectionParams, DbObject, ObjectKind, QueryError, QueryErrorKind,
    ResultSet, Role, Row, TableDef, Value,
};

mod worker;

use worker::Job;

pub struct Sqlite;

pub struct SqliteSession {
    /// The file name, shown as the session's one database.
    name: String,
    /// The worker thread that owns the connection and any open result.
    jobs: mpsc::Sender<Job>,
    interrupt: rusqlite::InterruptHandle,
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

    /// Runs `work` on the worker, beside any open result; `None` when the worker is gone.
    fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> BoxFuture<'static, Option<rusqlite::Result<T>>> {
        self.ask(|tx| {
            Job::Run(Box::new(move |conn| {
                tx.send(work(conn)).ok();
            }))
        })
    }

    /// Sends the job `job` builds around a reply channel; `None` when the worker is gone.
    fn ask<T: Send + 'static>(
        &self,
        job: impl FnOnce(oneshot::Sender<T>) -> Job,
    ) -> BoxFuture<'static, Option<T>> {
        let (tx, rx) = oneshot::channel();
        if self.jobs.send(job(tx)).is_err() {
            return ready(None).boxed();
        }
        rx.map(Result::ok).boxed()
    }

    fn reply<T>(&self, got: Option<rusqlite::Result<T>>) -> Result<T, String> {
        match got {
            Some(got) => got.map_err(|e| self.fail(e)),
            None => Err("the session is closed".into()),
        }
    }

    fn query_reply<T>(&self, got: Option<rusqlite::Result<T>>) -> Result<T, QueryError> {
        let err = match got {
            Some(Ok(value)) => return Ok(value),
            Some(Err(err)) => err,
            None => return Err(disconnected("the session is closed".into())),
        };
        let interrupted =
            err.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted);
        let message = self.fail(err);
        let kind = match (self.is_closed(), interrupted) {
            (true, _) => QueryErrorKind::Disconnected,
            (false, true) => QueryErrorKind::Cancelled,
            (false, false) => QueryErrorKind::Server,
        };
        Err(QueryError {
            kind,
            message,
            position: None,
            statement: 0,
        })
    }
}

fn disconnected(message: String) -> QueryError {
    QueryError {
        kind: QueryErrorKind::Disconnected,
        message,
        position: None,
        statement: 0,
    }
}

/// `ident` as a quoted SQLite identifier.
fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Opens the file `params` names, with a worker thread to run its statements.
fn open(params: &ConnectionParams) -> Result<SqliteSession, String> {
    let (name, conn) = open_file(params)?;
    let interrupt = conn.get_interrupt_handle();
    let (jobs, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("queryx-sqlite".into())
        .spawn(move || worker::work(conn, rx))
        .map_err(|e| e.to_string())?;
    Ok(SqliteSession {
        name,
        jobs,
        interrupt,
        closed: AtomicBool::new(false),
    })
}

/// Opens the file `params` names: its name, shown as the session's database, and the connection.
fn open_file(params: &ConnectionParams) -> Result<(String, rusqlite::Connection), String> {
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
    Ok((name, conn))
}

impl Driver for Sqlite {
    fn connect(&self, params: &ConnectionParams) -> Reply<Session> {
        let params = params.clone();
        blocking(move || Ok(Session::Sqlite(Arc::new(open(&params)?))))
    }

    fn test(&self, params: &ConnectionParams) -> Reply<()> {
        let params = params.clone();
        blocking(move || open_file(&params).map(drop))
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
        let schema = schema.to_string();
        let objects = session.run(move |conn| {
            let sql = format!(
                "SELECT type, name FROM {}.sqlite_schema \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY type, name",
                quote(&schema)
            );
            let mut stmt = conn.prepare(&sql)?;
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
            })?;
            rows.collect()
        });
        let session = session.clone();
        objects.map(move |got| session.reply(got)).boxed()
    }

    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>> {
        let Session::Sqlite(session) = session else {
            return foreign();
        };
        let (schema, object) = (schema.to_string(), object.to_string());
        let columns = session.run(move |conn| {
            // table_xinfo also lists generated columns; hidden = 1 marks a virtual table's hidden ones.
            let sql = "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_xinfo(?1, ?2) WHERE hidden <> 1 ORDER BY cid";
            let mut stmt = conn.prepare(sql)?;
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
            })?;
            rows.collect()
        });
        let session = session.clone();
        columns.map(move |got| session.reply(got)).boxed()
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

    /// `autocommit` is ignored: SQLite commits each statement unless the caller's BEGIN is open.
    fn open(&self, session: &Session, sql: String, _: bool) -> QueryReply<Opened> {
        let Session::Sqlite(session) = session else {
            return foreign_query();
        };
        let opened = session.ask(|tx| Job::Open(sql, tx));
        let session = session.clone();
        opened.map(move |got| session.query_reply(got)).boxed()
    }

    fn fetch(&self, session: &Session, max: usize) -> QueryReply<Vec<Row>> {
        let Session::Sqlite(session) = session else {
            return foreign_query();
        };
        let rows = session.ask(|tx| Job::Fetch(max, tx));
        let session = session.clone();
        rows.map(move |got| session.query_reply(got)).boxed()
    }

    fn close(&self, session: &Session) -> Reply<()> {
        let Session::Sqlite(session) = session else {
            return foreign();
        };
        // A gone worker has nothing open either.
        session.ask(Job::Close).map(|_| Ok(())).boxed()
    }

    fn cancel(&self, session: &Session) -> Reply<()> {
        let Session::Sqlite(session) = session else {
            return foreign();
        };
        session.interrupt.interrupt();
        ready(Ok(())).boxed()
    }

    fn query(&self, session: &Session, sql: String) -> QueryReply<ResultSet> {
        let Session::Sqlite(session) = session else {
            return foreign_query();
        };
        let result = session.run(move |conn| {
            let mut stmt = conn.prepare(&sql)?;
            let columns = worker::columns(&stmt);
            let width = columns.len();
            let mut rows = stmt.raw_query();
            let rows = worker::page(&mut rows, width, usize::MAX)?;
            Ok(ResultSet { columns, rows })
        });
        let session = session.clone();
        result.map(move |got| session.query_reply(got)).boxed()
    }
}

fn unsupported<T: Send + 'static>() -> Reply<T> {
    ready(Err("not supported on SQLite".into())).boxed()
}

#[cfg(test)]
mod tests;
