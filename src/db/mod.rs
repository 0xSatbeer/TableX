//! All database IO. The UI talks to a [`Driver`] and awaits its [`Reply`]s inside `cx.spawn`;
//! real sessions live on one shared tokio runtime (docs/adr/0001).

use std::error::Error as StdError;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::BoxFuture;

use crate::model::{
    Column, ColumnMeta, ConnectionParams, DbObject, QueryError, QueryErrorKind, ResultSet, Role,
    Row, TableDef, Trigger,
};

#[cfg(test)]
pub mod fake;
pub mod mock;
pub mod postgres;
pub mod sqlite;
mod tls;

/// A driver call's result, runtime-agnostic: awaiting it never needs tokio. `Err` holds the reason.
pub type Reply<T> = BoxFuture<'static, Result<T, String>>;

/// A query call's result; like `Reply`, but the failure says where and why.
pub type QueryReply<T> = BoxFuture<'static, Result<T, QueryError>>;

/// What `open` produced for one statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Opened {
    /// A row-returning statement; its rows wait on the server for `fetch`.
    Rows(Vec<ColumnMeta>),
    /// Any other statement, run to completion: rows affected (0 when the server gives none).
    Affected(u64),
}

/// One open session, opened by the driver that reads from it.
#[derive(Clone)]
pub enum Session {
    Postgres(Arc<postgres::PgSession>),
    Sqlite(Arc<sqlite::SqliteSession>),
    Mock(Arc<mock::MockSession>),
    #[cfg(test)]
    Fake(Arc<fake::FakeSession>),
}

impl Session {
    /// Whether the server side has gone (closed, restarted), so the session can't be used again.
    pub fn is_closed(&self) -> bool {
        match self {
            Session::Postgres(session) => session.is_closed(),
            Session::Sqlite(session) => session.is_closed(),
            Session::Mock(_) => false,
            #[cfg(test)]
            Session::Fake(session) => session.is_closed(),
        }
    }
}

pub trait Driver: Send + Sync {
    /// Opens a session to the database `params` name (for SQLite, its file).
    fn connect(&self, params: &ConnectionParams) -> Reply<Session>;
    /// Opens a throwaway session exactly as `connect` does, then closes it.
    fn test(&self, params: &ConnectionParams) -> Reply<()>;
    fn databases(&self, session: &Session) -> Reply<Vec<String>>;
    /// The schemas of the database `session` is open on.
    fn schemas(&self, session: &Session) -> Reply<Vec<String>>;
    /// Every object in `schema`, sorted by name within each kind; columns are not loaded.
    fn objects(&self, session: &Session, schema: &str) -> Reply<Vec<DbObject>>;
    /// The columns of table or view `object`, in ordinal order.
    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>>;
    /// Runs `statements` in order; in one transaction when `atomic`, else each on its own.
    fn execute(&self, session: &Session, statements: Vec<String>, atomic: bool) -> QueryReply<()>;
    /// Every role on the server, sorted by name.
    fn roles(&self, session: &Session) -> Reply<Vec<Role>>;
    /// The owner of the database `session` is open on.
    fn database_owner(&self, session: &Session) -> Reply<String>;
    fn schema_owner(&self, session: &Session, schema: &str) -> Reply<String>;
    fn table_def(&self, session: &Session, schema: &str, table: &str) -> Reply<TableDef>;
    /// The triggers on table `table`, sorted by name.
    #[allow(dead_code)] // read by the table view's DDL tab, a later ticket
    fn triggers(&self, session: &Session, schema: &str, table: &str) -> Reply<Vec<Trigger>>;
    /// A view's SELECT (PG `pg_get_viewdef`, SQLite `sqlite_master.sql`).
    #[allow(dead_code)]
    fn view_def(&self, session: &Session, schema: &str, view: &str) -> Reply<String>;
    /// Runs one statement. Row-returning statements stay open for `fetch`. With `autocommit`,
    /// Postgres opens the portal inside an implicit transaction that commits on close/exhaustion.
    fn open(&self, session: &Session, sql: String, autocommit: bool) -> QueryReply<Opened>;
    /// Up to `max` more rows of the open result; fewer than `max` means it is exhausted and closed.
    fn fetch(&self, session: &Session, max: usize) -> QueryReply<Vec<Row>>;
    /// Closes the open result, if any. Never fails on a session with nothing open.
    #[allow(dead_code)] // the query methods' other callers (paging, cancel, table views) come in later tickets
    fn close(&self, session: &Session) -> Reply<()>;
    /// Stops whatever `session` is running; that call fails with `QueryErrorKind::Cancelled`.
    #[allow(dead_code)]
    fn cancel(&self, session: &Session) -> Reply<()>;
    /// Runs one row-returning statement to completion (table pages, counts). Never leaves a result open.
    #[allow(dead_code)]
    fn query(&self, session: &Session, sql: String) -> QueryReply<ResultSet>;
}

/// The real drivers: Postgres or SQLite, picked by the params or the session.
#[derive(Default)]
pub struct Live {
    postgres: postgres::Postgres,
}

impl Live {
    fn on(&self, session: &Session) -> &dyn Driver {
        match session {
            Session::Sqlite(_) => &sqlite::Sqlite,
            _ => &self.postgres,
        }
    }

    fn by(&self, params: &ConnectionParams) -> &dyn Driver {
        match params {
            ConnectionParams::Postgres { .. } => &self.postgres,
            ConnectionParams::Sqlite { .. } => &sqlite::Sqlite,
        }
    }
}

impl Driver for Live {
    fn connect(&self, params: &ConnectionParams) -> Reply<Session> {
        self.by(params).connect(params)
    }

    fn test(&self, params: &ConnectionParams) -> Reply<()> {
        self.by(params).test(params)
    }

    fn databases(&self, session: &Session) -> Reply<Vec<String>> {
        self.on(session).databases(session)
    }

    fn schemas(&self, session: &Session) -> Reply<Vec<String>> {
        self.on(session).schemas(session)
    }

    fn objects(&self, session: &Session, schema: &str) -> Reply<Vec<DbObject>> {
        self.on(session).objects(session, schema)
    }

    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>> {
        self.on(session).columns(session, schema, object)
    }

    fn execute(&self, session: &Session, statements: Vec<String>, atomic: bool) -> QueryReply<()> {
        self.on(session).execute(session, statements, atomic)
    }

    fn triggers(&self, session: &Session, schema: &str, table: &str) -> Reply<Vec<Trigger>> {
        self.on(session).triggers(session, schema, table)
    }

    fn view_def(&self, session: &Session, schema: &str, view: &str) -> Reply<String> {
        self.on(session).view_def(session, schema, view)
    }

    fn roles(&self, session: &Session) -> Reply<Vec<Role>> {
        self.on(session).roles(session)
    }

    fn database_owner(&self, session: &Session) -> Reply<String> {
        self.on(session).database_owner(session)
    }

    fn schema_owner(&self, session: &Session, schema: &str) -> Reply<String> {
        self.on(session).schema_owner(session, schema)
    }

    fn table_def(&self, session: &Session, schema: &str, table: &str) -> Reply<TableDef> {
        self.on(session).table_def(session, schema, table)
    }

    fn open(&self, session: &Session, sql: String, autocommit: bool) -> QueryReply<Opened> {
        self.on(session).open(session, sql, autocommit)
    }

    fn fetch(&self, session: &Session, max: usize) -> QueryReply<Vec<Row>> {
        self.on(session).fetch(session, max)
    }

    fn close(&self, session: &Session) -> Reply<()> {
        self.on(session).close(session)
    }

    fn cancel(&self, session: &Session) -> Reply<()> {
        self.on(session).cancel(session)
    }

    fn query(&self, session: &Session, sql: String) -> QueryReply<ResultSet> {
        self.on(session).query(session, sql)
    }
}

/// The reason we show for a failure: a server's own message without severity or prefixes,
/// otherwise the innermost error's text.
pub fn reason(err: &(dyn StdError + 'static)) -> String {
    let mut err = err;
    loop {
        if let Some(server) = err.downcast_ref::<tokio_postgres::error::DbError>() {
            return server.message().to_string();
        }
        match err.source() {
            Some(source) => err = source,
            None => return err.to_string(),
        }
    }
}

/// The reason for our own connect timeout of `after`.
pub fn timeout_reason(after: std::time::Duration) -> String {
    format!("timeout expired after {} s", after.as_secs_f64())
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tablex-db")
            .enable_all()
            .build()
            .expect("start the database runtime")
    })
}

/// Runs `work` on the database runtime.
fn spawn<T: Send + 'static>(
    work: impl Future<Output = Result<T, String>> + Send + 'static,
) -> Reply<T> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn(async move { tx.send(work.await).ok() });
    rx.map(|result| result.unwrap_or_else(|_| Err("the database task stopped".into())))
        .boxed()
}

/// `spawn` for query calls.
fn spawn_query<T: Send + 'static>(
    work: impl Future<Output = Result<T, QueryError>> + Send + 'static,
) -> QueryReply<T> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn(async move { tx.send(work.await).ok() });
    rx.map(|result| result.unwrap_or_else(|_| Err(server_error("the database task stopped"))))
        .boxed()
}

/// Runs blocking `work` (SQLite) on the database runtime's blocking pool.
fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Reply<T> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn_blocking(move || tx.send(work()).ok());
    rx.map(|result| result.unwrap_or_else(|_| Err("the database task stopped".into())))
        .boxed()
}

/// A query failure the server or file gave as a plain reason.
fn server_error(message: impl Into<String>) -> QueryError {
    QueryError {
        kind: QueryErrorKind::Server,
        message: message.into(),
        position: None,
        statement: 0,
    }
}

/// A session of the wrong driver: a caller bug, reported instead of panicking.
fn foreign<T: Send + 'static>() -> Reply<T> {
    futures::future::ready(Err("session belongs to another driver".to_string())).boxed()
}

/// `foreign` for query calls.
fn foreign_query<T: Send + 'static>() -> QueryReply<T> {
    futures::future::ready(Err(server_error("session belongs to another driver"))).boxed()
}

#[cfg(test)]
mod tests;
