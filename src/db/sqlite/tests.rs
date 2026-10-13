use super::*;
use futures::executor::block_on;
use std::path::{Path, PathBuf};

/// A SQLite file at `dir/name` holding `sql`.
fn file(dir: &Path, name: &str, sql: &str) -> PathBuf {
    let path = dir.join(name);
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(sql)
        .unwrap();
    path
}

fn params(path: &Path) -> ConnectionParams {
    ConnectionParams::Sqlite {
        path: path.to_path_buf(),
    }
}

#[test]
fn a_valid_file_is_one_database_named_after_it_with_schema_main() {
    let dir = tempfile::tempdir().unwrap();
    // (file name, database)
    let cases = [
        ("local.db", "local.db"),
        ("app.sqlite3", "app.sqlite3"),
        ("no extension", "no extension"),
    ];
    for (name, want) in cases {
        let session =
            block_on(Sqlite.connect(&params(&file(dir.path(), name, "CREATE TABLE t (a)"))))
                .expect(name);
        assert_eq!(
            block_on(Sqlite.databases(&session)),
            Ok(vec![want.to_string()]),
            "{name}"
        );
        assert_eq!(
            block_on(Sqlite.schemas(&session)),
            Ok(vec!["main".to_string()]),
            "{name}"
        );
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
        assert_eq!(
            block_on(Sqlite.connect(&params(&path))).map(drop),
            want,
            "connect {path:?}"
        );
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
        (
            "hello, I am a text file\n".repeat(100),
            Err("file is not a database".to_string()),
        ),
        (header_like, Err("file is not a database".to_string())),
        (String::new(), Ok(())), // SQLite treats an empty file as an empty database
    ];
    for (i, (contents, want)) in cases.into_iter().enumerate() {
        let path = dir.path().join(format!("{i}.db"));
        std::fs::write(&path, contents).unwrap();
        assert_eq!(
            block_on(Sqlite.connect(&params(&path))).map(drop),
            want,
            "connect {i}"
        );
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

/// Makes every read of the open SQLite file at `path` fail with EISDIR, as a vanished drive would
/// fail with EIO: a directory is put under its file descriptor.
fn break_reads_of(path: &Path) {
    unsafe extern "C" {
        fn dup2(old: i32, new: i32) -> i32;
    }
    use std::os::fd::AsRawFd;
    let path = path.canonicalize().unwrap();
    let fd = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| entry.ok())
        .find(|entry| std::fs::read_link(entry.path()).is_ok_and(|target| target == path))
        .and_then(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .expect("the session's file is open");
    let dir = std::fs::File::open(path.parent().unwrap()).unwrap();
    assert_eq!(unsafe { dup2(dir.as_raw_fd(), fd) }, fd);
}

#[test]
fn a_session_whose_file_fails_with_an_io_error_reports_closed() {
    let dir = tempfile::tempdir().unwrap();
    // (reads broken, call, fails, closed after)
    let cases = [
        (false, "objects main", false, false),
        (false, "objects nope", true, false), // an unknown schema is not an I/O error
        (false, "columns main/t", false, false),
        (true, "objects main", true, true),
        (true, "columns main/t", true, true),
    ];
    for (i, (broken, call, fails, closed)) in cases.into_iter().enumerate() {
        let path = file(dir.path(), &format!("{i}.db"), "CREATE TABLE t (a)");
        let session = block_on(Sqlite.connect(&params(&path))).unwrap();
        assert!(!session.is_closed(), "{call}: open after connect");
        if broken {
            break_reads_of(&path);
        }
        let got = match call {
            "objects main" => block_on(Sqlite.objects(&session, "main")).map(drop),
            "objects nope" => block_on(Sqlite.objects(&session, "nope")).map(drop),
            _ => block_on(Sqlite.columns(&session, "main", "t")).map(drop),
        };
        assert_eq!(got.is_err(), fails, "broken {broken} {call}: {got:?}");
        assert_eq!(
            session.is_closed(),
            closed,
            "broken {broken} {call}: {got:?}"
        );
    }
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
        (
            "CREATE VIEW z AS SELECT 1; CREATE VIEW m AS SELECT 2;",
            vec![(View, "m"), (View, "z")],
        ),
        (
            "CREATE TABLE t (x); CREATE INDEX t_x ON t (x); CREATE TRIGGER t_ai AFTER INSERT ON t BEGIN SELECT 1; END;",
            vec![(Table, "t")],
        ),
        (
            "CREATE TABLE \"we/ird name\" (x); CREATE TABLE c (x);",
            vec![(Table, "c"), (Table, "we/ird name")],
        ),
    ];
    for (i, (sql, want)) in cases.into_iter().enumerate() {
        let session =
            block_on(Sqlite.connect(&params(&file(dir.path(), &format!("{i}.db"), sql)))).unwrap();
        let objects = block_on(Sqlite.objects(&session, "main")).expect(sql);
        let got: Vec<_> = objects.iter().map(|o| (o.kind, o.name.as_str())).collect();
        assert_eq!(got, want, "{sql}");
        assert!(
            objects
                .iter()
                .all(|o| o.columns.is_none() && o.row_estimate.is_none()),
            "{sql}"
        );
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
        identity: false,
    };
    // (schema sql, object, columns)
    let cases = [
        (
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
            "t",
            vec![
                col("id", "INTEGER", true, None, true),
                col("name", "TEXT", true, None, false),
            ],
        ),
        (
            "CREATE TABLE t (a INT, b TEXT NOT NULL DEFAULT 'x', c, PRIMARY KEY (b, a))",
            "t",
            vec![
                col("a", "INT", true, None, true),
                col("b", "TEXT", false, Some("'x'"), true),
                col("c", "", true, None, false),
            ],
        ),
        (
            "CREATE TABLE t (price REAL, qty INT, total REAL GENERATED ALWAYS AS (price * qty) VIRTUAL)",
            "t",
            vec![
                col("price", "REAL", true, None, false),
                col("qty", "INT", true, None, false),
                col("total", "REAL", true, None, false),
            ],
        ),
        (
            "CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT); CREATE VIEW v AS SELECT id, x AS y FROM t",
            "v",
            vec![
                col("id", "INTEGER", true, None, false),
                col("y", "TEXT", true, None, false),
            ],
        ),
        (
            "CREATE TABLE \"we/ird\" (\"a b\" TEXT)",
            "we/ird",
            vec![col("a b", "TEXT", true, None, false)],
        ),
    ];
    for (i, (sql, object, want)) in cases.into_iter().enumerate() {
        let session =
            block_on(Sqlite.connect(&params(&file(dir.path(), &format!("{i}.db"), sql)))).unwrap();
        assert_eq!(
            block_on(Sqlite.columns(&session, "main", object)),
            Ok(want),
            "{sql}"
        );
    }
}

#[test]
fn ddl_and_metadata_calls_are_not_supported() {
    let dir = tempfile::tempdir().unwrap();
    let session =
        block_on(Sqlite.connect(&params(&file(dir.path(), "a.db", "CREATE TABLE t (a)")))).unwrap();
    let want = Err("not supported on SQLite".to_string());
    assert_eq!(
        block_on(Sqlite.execute(&session, vec!["DROP TABLE t".into()], true)),
        want
    );
    assert_eq!(block_on(Sqlite.roles(&session)).map(drop), want);
    assert_eq!(block_on(Sqlite.database_owner(&session)).map(drop), want);
    assert_eq!(
        block_on(Sqlite.schema_owner(&session, "main")).map(drop),
        want
    );
    assert_eq!(
        block_on(Sqlite.table_def(&session, "main", "t")).map(drop),
        want
    );
    assert_eq!(
        block_on(Sqlite.columns(&session, "main", "t")).map(|c| c.len()),
        Ok(1),
        "nothing ran"
    );
}
