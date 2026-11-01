use super::*;
use crate::db::sqlite::Sqlite;
use crate::model::ConnectionParams;
use futures::executor::block_on;
use std::path::Path;

fn columns(names: &[&str]) -> Vec<ColumnMeta> {
    names
        .iter()
        .map(|n| ColumnMeta {
            name: (*n).into(),
            data_type: "text".into(),
            is_pk: false,
            is_fk: false,
            enum_values: Arc::new([]),
        })
        .collect()
}

fn write(format: Format, cols: Vec<ColumnMeta>, rows: &[Vec<Value>]) -> String {
    let mut out = Writer::new(format, Vec::new(), cols, "t");
    out.header().unwrap();
    out.rows(rows).unwrap();
    String::from_utf8(out.finish().unwrap()).unwrap()
}

#[test]
fn each_format_escapes_nulls_quotes_and_newlines() {
    let rows = vec![
        vec![Value::Int(1), Value::Null],
        vec![Value::Int(2), Value::Text("say \"hi\"".into())],
        vec![Value::Int(3), Value::Text("a\nb".into())],
    ];
    // (format, bytes)
    let cases = [
        (
            Format::Csv,
            "id,note\n1,\n2,\"say \"\"hi\"\"\"\n3,\"a\nb\"\n",
        ),
        (
            Format::Tsv,
            "id\tnote\n1\t\n2\t\"say \"\"hi\"\"\"\n3\t\"a\nb\"\n",
        ),
        (
            Format::Json,
            "[\n  {\"id\": 1, \"note\": null},\n  {\"id\": 2, \"note\": \"say \\\"hi\\\"\"},\n  {\"id\": 3, \"note\": \"a\\nb\"}\n]\n",
        ),
        (
            Format::SqlInsert,
            "INSERT INTO t (id, note) VALUES (1, NULL);\nINSERT INTO t (id, note) VALUES (2, 'say \"hi\"');\nINSERT INTO t (id, note) VALUES (3, 'a\nb');\n",
        ),
        (
            Format::Markdown,
            "| id | note |\n| --- | --- |\n| 1 | NULL |\n| 2 | say \"hi\" |\n| 3 | a b |\n",
        ),
    ];
    for (format, want) in cases {
        assert_eq!(
            write(format, columns(&["id", "note"]), &rows),
            want,
            "{format:?}"
        );
    }
}

#[test]
fn json_split_over_batches_matches_one_batch() {
    let rows = vec![
        vec![Value::Int(1)],
        vec![Value::Int(2)],
        vec![Value::Int(3)],
    ];
    let mut out = Writer::new(Format::Json, Vec::new(), columns(&["n"]), "t");
    out.header().unwrap();
    out.rows(&rows[..1]).unwrap();
    out.rows(&rows[1..]).unwrap();
    let split = String::from_utf8(out.finish().unwrap()).unwrap();
    assert_eq!(split, write(Format::Json, columns(&["n"]), &rows));
}

/// A job over a fresh SQLite session on a 20k-row table `t`, writing CSV to `dir/out.csv`.
fn job(dir: &Path) -> Job {
    let db = dir.join("big.db");
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE t (n INTEGER);
             WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 20000)
             INSERT INTO t SELECT n FROM c;",
        )
        .unwrap();
    let session = block_on(Sqlite.connect(&ConnectionParams::Sqlite { path: db })).unwrap();
    Job {
        driver: Arc::new(Sqlite),
        session,
        sql: "SELECT n FROM t ORDER BY n".into(),
        format: Format::Csv,
        table: "t".into(),
        path: dir.join("out.csv"),
        cancelled: Arc::new(AtomicBool::new(false)),
    }
}

#[test]
fn a_streaming_export_writes_every_row_in_batches() {
    let dir = tempfile::tempdir().unwrap();
    let job = job(dir.path());
    let path = job.path.clone();
    let mut batches = Vec::new();
    let written = block_on(run(job, |n| batches.push(n)));
    assert_eq!(written, Ok(20_000));
    assert_eq!(batches, [5_000, 10_000, 15_000, 20_000]);
    let text = std::fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 20_001);
    assert_eq!((lines[0], lines[1], lines[20_000]), ("n", "1", "20000"));
}

#[test]
fn cancel_removes_the_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let job = job(dir.path());
    let (path, cancelled) = (job.path.clone(), job.cancelled.clone());
    let result = block_on(run(job, |_| cancelled.store(true, Ordering::Relaxed)));
    assert_eq!(result, Err("Cancelled".into()));
    assert!(!path.exists());
}

#[test]
fn a_failure_before_writing_leaves_an_existing_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    // (sql, error)
    let cases = [
        ("SELECT * FROM missing", "no such table: missing"),
        ("UPDATE t SET n = n", "The statement returns no rows"),
    ];
    for (sql, want) in cases {
        let mut job = job(dir.path());
        job.sql = sql.into();
        std::fs::write(&job.path, "kept").unwrap();
        let path = job.path.clone();
        let result = block_on(run(job, |_| {}));
        assert!(result.unwrap_err().contains(want), "{sql}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept", "{sql}");
        std::fs::remove_file(dir.path().join("big.db")).unwrap();
    }
}

#[test]
fn dropping_the_export_mid_stream_removes_the_partial_file() {
    use crate::db::fake::{self, Fake};
    use crate::model::Row;
    use futures::FutureExt as _;
    let dir = tempfile::tempdir().unwrap();
    let driver = Arc::new(Fake::default());
    let path = dir.path().join("out.csv");
    let job = Job {
        driver: driver.clone(),
        session: fake::session("app"),
        sql: "SELECT n FROM t".into(),
        format: Format::Csv,
        table: "t".into(),
        path: path.clone(),
        cancelled: Arc::new(AtomicBool::new(false)),
    };
    let mut export = Box::pin(run(job, |_| {}));
    assert!(export.as_mut().now_or_never().is_none());
    driver.resolve_query("open app", Ok(Opened::Rows(columns(&["n"]))));
    assert!(export.as_mut().now_or_never().is_none());
    let batch: Vec<Row> = (0..BATCH as i64)
        .map(|n| vec![Value::Int(n)].into())
        .collect();
    driver.resolve_query("fetch app", Ok(batch));
    assert!(export.as_mut().now_or_never().is_none());
    assert!(path.exists(), "the first batch is on disk");
    drop(export);
    assert!(!path.exists());
}
