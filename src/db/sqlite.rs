//! The SQLite driver: rusqlite on the shared runtime's blocking pool.

use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::ready;

use super::{Driver, Reply, Session, blocking, foreign};
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

/// Opens the file `params` names.
fn open(params: &ConnectionParams) -> Result<SqliteSession, String> {
    let ConnectionParams::Sqlite { path } = params else { return Err("not a SQLite connection".into()) };
    if !path.is_file() {
        return Err(format!("No file at {}", path.display()));
    }
    // Without SQLITE_OPEN_CREATE, so a typo never leaves an empty database behind.
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = rusqlite::Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
    Ok(SqliteSession { name, conn: Mutex::new(conn) })
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
        let Session::Sqlite(session) = session else { return foreign() };
        ready(Ok(vec![session.name.clone()])).boxed()
    }

    fn schemas(&self, session: &Session) -> Reply<Vec<String>> {
        let Session::Sqlite(_) = session else { return foreign() };
        ready(Ok(vec!["main".to_string()])).boxed()
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

    #[test]
    fn a_missing_path_or_a_directory_is_no_file_and_nothing_is_created() {
        let dir = tempfile::tempdir().unwrap();
        // (path, created by the test itself)
        let cases = [
            (dir.path().join("local.db"), false),
            (dir.path().join("nested/local.db"), false),
            (dir.path().to_path_buf(), true),
            (dir.path().join(""), true),
        ];
        for (path, existed) in cases {
            let want = Err(format!("No file at {}", path.display()));
            assert_eq!(block_on(Sqlite.connect(&params(&path))).map(drop), want, "connect {path:?}");
            assert_eq!(block_on(Sqlite.test(&params(&path))), want, "test {path:?}");
            assert_eq!(path.exists(), existed, "{path:?}");
        }
    }
}
