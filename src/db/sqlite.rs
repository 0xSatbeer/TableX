//! The SQLite driver: rusqlite on the shared runtime's blocking pool.

use std::sync::Mutex;

use futures::FutureExt;

use super::{Driver, Reply, Session};
use crate::model::{Column, ConnectionParams, DbObject};

pub struct Sqlite;

pub struct SqliteSession {
    /// The file name, shown as the session's one database.
    name: String,
    conn: Mutex<rusqlite::Connection>,
}

fn not_yet<T: Send + 'static>() -> Reply<T> {
    futures::future::ready(Err("not implemented".to_string())).boxed()
}

impl Driver for Sqlite {
    fn connect(&self, _params: &ConnectionParams) -> Reply<Session> {
        not_yet()
    }

    fn test(&self, _params: &ConnectionParams) -> Reply<()> {
        not_yet()
    }

    fn databases(&self, _session: &Session) -> Reply<Vec<String>> {
        not_yet()
    }

    fn schemas(&self, _session: &Session) -> Reply<Vec<String>> {
        not_yet()
    }

    fn objects(&self, _session: &Session, _schema: &str) -> Reply<Vec<DbObject>> {
        not_yet()
    }

    fn columns(&self, _session: &Session, _schema: &str, _object: &str) -> Reply<Vec<Column>> {
        not_yet()
    }
}
