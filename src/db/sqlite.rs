//! The SQLite driver: rusqlite on the shared runtime's blocking pool.

use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::ready;

use super::{Driver, Reply, Session, blocking, foreign};
use crate::model::{Column, ConnectionParams, DbObject, ObjectKind};

pub struct Sqlite;

pub struct SqliteSession {
    /// The file name, shown as the session's one database.
    name: String,
    conn: Mutex<rusqlite::Connection>,
}

/// `ident` as a quoted SQLite identifier.
fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
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
    // Waits this long on a file another process holds locked (rusqlite's default, made explicit).
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    // Reads the header now, so a bad file fails at connect rather than at the first listing.
    conn.query_row("PRAGMA schema_version", [], |_| Ok(())).map_err(|e| e.to_string())?;
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

    fn objects(&self, session: &Session, schema: &str) -> Reply<Vec<DbObject>> {
        let Session::Sqlite(session) = session else { return foreign() };
        let (session, schema) = (session.clone(), schema.to_string());
        blocking(move || {
            let sql = format!(
                "SELECT type, name FROM {}.sqlite_schema \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY type, name",
                quote(&schema)
            );
            let conn = session.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |row| {
                let kind = if row.get::<_, String>(0)? == "view" { ObjectKind::View } else { ObjectKind::Table };
                Ok(DbObject { kind, name: row.get(1)?, columns: Vec::new(), keys: Vec::new(), indexes: Vec::new(), row_estimate: None })
            });
            rows.and_then(Iterator::collect).map_err(|e| e.to_string())
        })
    }

    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>> {
        let Session::Sqlite(session) = session else { return foreign() };
        let (session, schema, object) = (session.clone(), schema.to_string(), object.to_string());
        blocking(move || {
            // table_xinfo also lists generated columns; hidden = 1 marks a virtual table's hidden ones.
            let sql = "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_xinfo(?1, ?2) WHERE hidden <> 1 ORDER BY cid";
            let conn = session.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
            let rows = stmt.query_map([&object, &schema], |row| {
                Ok(Column {
                    name: row.get(0)?,
                    data_type: row.get(1)?,
                    nullable: row.get::<_, i64>(2)? == 0,
                    default: row.get(3)?,
                    is_pk: row.get::<_, i64>(4)? > 0,
                    enum_values: Vec::new(),
                })
            });
            rows.and_then(Iterator::collect).map_err(|e| e.to_string())
        })
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

    #[test]
    fn a_file_that_is_not_sqlite_fails_at_connect() {
        let dir = tempfile::tempdir().unwrap();
        let header_like = format!("SQLite format 3\0{}", "x".repeat(200));
        // (contents, reason)
        let cases = [
            ("hello, I am a text file\n".repeat(100), Err("file is not a database".to_string())),
            (header_like, Err("file is not a database".to_string())),
            (String::new(), Ok(())), // SQLite treats an empty file as an empty database
        ];
        for (i, (contents, want)) in cases.into_iter().enumerate() {
            let path = dir.path().join(format!("{i}.db"));
            std::fs::write(&path, contents).unwrap();
            assert_eq!(block_on(Sqlite.connect(&params(&path))).map(drop), want, "connect {i}");
            assert_eq!(block_on(Sqlite.test(&params(&path))), want, "test {i}");
        }
    }

    #[test]
    fn an_exclusively_locked_file_is_locked_after_the_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let path = file(dir.path(), "local.db", "CREATE TABLE t (a)");
        let holder = rusqlite::Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let started = std::time::Instant::now();
        let got = block_on(Sqlite.connect(&params(&path))).map(drop);
        let took = started.elapsed();
        assert_eq!(got, Err("database is locked".to_string()));
        let busy = std::time::Duration::from_secs(5);
        assert!(took >= busy && took < busy * 2, "took {took:?}");
    }

    #[test]
    fn objects_are_tables_and_views_sorted_by_name_without_internal_tables() {
        use crate::model::ObjectKind::{Table, View};
        let dir = tempfile::tempdir().unwrap();
        // (schema sql, objects in main)
        let cases = [
            (
                "CREATE TABLE b (x); CREATE TABLE a (id INTEGER PRIMARY KEY AUTOINCREMENT); CREATE VIEW v AS SELECT 1;",
                vec![(Table, "a"), (Table, "b"), (View, "v")],
            ),
            ("", vec![]),
            ("CREATE VIEW z AS SELECT 1; CREATE VIEW m AS SELECT 2;", vec![(View, "m"), (View, "z")]),
            (
                "CREATE TABLE t (x); CREATE INDEX t_x ON t (x); CREATE TRIGGER t_ai AFTER INSERT ON t BEGIN SELECT 1; END;",
                vec![(Table, "t")],
            ),
            ("CREATE TABLE \"we/ird name\" (x); CREATE TABLE c (x);", vec![(Table, "c"), (Table, "we/ird name")]),
        ];
        for (i, (sql, want)) in cases.into_iter().enumerate() {
            let session = block_on(Sqlite.connect(&params(&file(dir.path(), &format!("{i}.db"), sql)))).unwrap();
            let objects = block_on(Sqlite.objects(&session, "main")).expect(sql);
            let got: Vec<_> = objects.iter().map(|o| (o.kind, o.name.as_str())).collect();
            assert_eq!(got, want, "{sql}");
            assert!(objects.iter().all(|o| o.columns.is_empty() && o.row_estimate.is_none()), "{sql}");
        }
    }

    #[test]
    fn columns_come_in_ordinal_order_with_type_nullability_default_and_pk() {
        let dir = tempfile::tempdir().unwrap();
        let col = |name: &str, data_type: &str, nullable, default: Option<&str>, is_pk| Column {
            name: name.into(),
            data_type: data_type.into(),
            nullable,
            default: default.map(Into::into),
            is_pk,
            enum_values: Vec::new(),
        };
        // (schema sql, object, columns)
        let cases = [
            ("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", "t", vec![col("id", "INTEGER", true, None, true), col("name", "TEXT", true, None, false)]),
            (
                "CREATE TABLE t (a INT, b TEXT NOT NULL DEFAULT 'x', c, PRIMARY KEY (b, a))",
                "t",
                vec![col("a", "INT", true, None, true), col("b", "TEXT", false, Some("'x'"), true), col("c", "", true, None, false)],
            ),
            (
                "CREATE TABLE t (price REAL, qty INT, total REAL GENERATED ALWAYS AS (price * qty) VIRTUAL)",
                "t",
                vec![col("price", "REAL", true, None, false), col("qty", "INT", true, None, false), col("total", "REAL", true, None, false)],
            ),
            (
                "CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT); CREATE VIEW v AS SELECT id, x AS y FROM t",
                "v",
                vec![col("id", "INTEGER", true, None, false), col("y", "TEXT", true, None, false)],
            ),
            ("CREATE TABLE \"we/ird\" (\"a b\" TEXT)", "we/ird", vec![col("a b", "TEXT", true, None, false)]),
        ];
        for (i, (sql, object, want)) in cases.into_iter().enumerate() {
            let session = block_on(Sqlite.connect(&params(&file(dir.path(), &format!("{i}.db"), sql)))).unwrap();
            assert_eq!(block_on(Sqlite.columns(&session, "main", object)), Ok(want), "{sql}");
        }
    }
}
