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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use std::path::{Path, PathBuf};

    /// A SQLite file at `dir/name` holding `sql`.
    fn file(dir: &Path, name: &str, sql: &str) -> PathBuf {
        let path = dir.join(name);
        rusqlite::Connection::open(&path).unwrap().execute_batch(sql).unwrap();
        path
    }

    fn params(path: &Path) -> ConnectionParams {
        ConnectionParams::Sqlite { path: path.to_path_buf() }
    }

    #[test]
    fn a_valid_file_is_one_database_named_after_it_with_schema_main() {
        let dir = tempfile::tempdir().unwrap();
        // (file name, database)
        let cases = [("local.db", "local.db"), ("app.sqlite3", "app.sqlite3"), ("no extension", "no extension")];
        for (name, want) in cases {
            let session = block_on(Sqlite.connect(&params(&file(dir.path(), name, "CREATE TABLE t (a)")))).expect(name);
            assert_eq!(block_on(Sqlite.databases(&session)), Ok(vec![want.to_string()]), "{name}");
            assert_eq!(block_on(Sqlite.schemas(&session)), Ok(vec!["main".to_string()]), "{name}");
        }
    }
}
