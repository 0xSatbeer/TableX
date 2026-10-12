//! All database IO. The UI talks to a [`Driver`] and awaits its [`Reply`]s inside `cx.spawn`;
//! real sessions live on one shared tokio runtime (docs/adr/0001).

use std::error::Error as StdError;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::BoxFuture;

use crate::model::{Column, ConnectionParams, DbObject};

#[cfg(test)]
pub mod fake;
pub mod postgres;
pub mod sqlite;

/// A driver call's result, runtime-agnostic: awaiting it never needs tokio. `Err` holds the reason.
pub type Reply<T> = BoxFuture<'static, Result<T, String>>;

/// One open session, opened by the driver that reads from it.
#[derive(Clone)]
pub enum Session {
    Postgres(Arc<postgres::PgSession>),
    Sqlite(Arc<sqlite::SqliteSession>),
    /// The fake driver's session: the database it was opened on.
    #[cfg(test)]
    Fake(String),
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
}

/// The reason we show for a failure: a server's own message without severity or prefixes,
/// otherwise the innermost error's text.
pub fn reason(err: &(dyn StdError + 'static)) -> String {
    err.to_string()
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("queryx-db")
            .enable_all()
            .build()
            .expect("start the database runtime")
    })
}

/// Runs `work` on the database runtime.
fn spawn<T: Send + 'static>(work: impl Future<Output = Result<T, String>> + Send + 'static) -> Reply<T> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn(async move { tx.send(work.await).ok() });
    rx.map(|result| result.unwrap_or_else(|_| Err("the database task stopped".into()))).boxed()
}

/// Runs blocking `work` (SQLite) on the database runtime's blocking pool.
fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Reply<T> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn_blocking(move || tx.send(work()).ok());
    rx.map(|result| result.unwrap_or_else(|_| Err("the database task stopped".into()))).boxed()
}

/// A session of the wrong driver: a caller bug, reported instead of panicking.
fn foreign<T: Send + 'static>() -> Reply<T> {
    futures::future::ready(Err("session belongs to another driver".to_string())).boxed()
}
