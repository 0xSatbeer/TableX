use std::time::{Duration, Instant};

use super::*;
use crate::db::Opened;
use crate::model::{QueryErrorKind, Value};

fn ints(rows: &[crate::model::Row]) -> Vec<i64> {
    rows.iter()
        .map(|row| match row[0] {
            Value::Int(n) => n,
            ref other => panic!("not an int: {other:?}"),
        })
        .collect()
}

#[test]
fn open_and_fetch_page_through_a_series_and_the_short_page_closes_it() {
    let Some(pg) = test_pg() else { return };
    let session = session(&pg);
    let driver = Postgres::default();
    let opened = block_on(driver.open(&session, "SELECT 'other'".into(), true));
    assert!(matches!(opened, Ok(Opened::Rows(_))));

    let opened = block_on(driver.open(
        &session,
        "SELECT * FROM generate_series(1, 1234) n".into(),
        true,
    ));
    let Ok(Opened::Rows(columns)) = opened else {
        panic!("{opened:?}")
    };
    assert_eq!((&*columns[0].name, &*columns[0].data_type), ("n", "int4"));
    let pages: Vec<_> = (0..3)
        .map(|_| block_on(driver.fetch(&session, 500)).expect("fetch"))
        .collect();
    assert_eq!(
        pages.iter().map(Vec::len).collect::<Vec<_>>(),
        [500, 500, 234],
        "open replaced the first result"
    );
    assert_eq!(ints(&pages[0])[0], 1);
    assert_eq!(*ints(&pages[2]).last().unwrap(), 1234);
    assert_eq!(
        block_on(driver.fetch(&session, 500)),
        Ok(Vec::new()),
        "the result is closed"
    );
    assert_eq!(
        block_on(driver.close(&session)),
        Ok(()),
        "closing nothing never fails"
    );
}

#[test]
fn open_of_an_insert_reports_the_rows_affected() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    admin(&pg, &format!("CREATE TABLE {s}.t (a int)"));
    let session = session(&pg);
    let sql = format!("INSERT INTO {s}.t SELECT generate_series(1, 3)");
    assert_eq!(
        block_on(Postgres::default().open(&session, sql, true)),
        Ok(Opened::Affected(3))
    );
    assert_eq!(admin(&pg, &format!("SELECT count(*) FROM {s}.t")), ["3"]);
}

#[test]
fn a_syntax_error_gives_the_byte_position_of_the_bad_token() {
    let Some(pg) = test_pg() else { return };
    let session = session(&pg);
    let sql = "SELECT 'héllo' AS a FORM t";
    let err = block_on(Postgres::default().open(&session, sql.into(), true)).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Server);
    assert_eq!(err.message, "syntax error at or near \"FORM\"");
    assert_eq!(
        err.position,
        sql.find("FORM"),
        "a byte offset, past the two-byte é"
    );
    let err = block_on(Postgres::default().query(&session, sql.into())).unwrap_err();
    assert_eq!(err.position, sql.find("FORM"));
    let sql = "SELECT 'é' +";
    let err = block_on(Postgres::default().open(&session, sql.into(), true)).unwrap_err();
    assert_eq!(err.message, "syntax error at end of input");
    assert_eq!(err.position, Some(sql.len()), "one past the end");
}

#[test]
fn cancel_stops_a_running_fetch_and_the_session_stays_usable() {
    let Some(pg) = test_pg() else { return };
    let session = session(&pg);
    let driver = Postgres::default();
    let opened = block_on(driver.open(&session, "SELECT pg_sleep(10)".into(), true));
    assert!(matches!(opened, Ok(Opened::Rows(_))), "{opened:?}");
    let started = Instant::now();
    let fetch = driver.fetch(&session, 500);
    std::thread::sleep(Duration::from_millis(300));
    block_on(driver.cancel(&session)).expect("cancel");
    let err = block_on(fetch).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Cancelled, "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(2));

    let txid = block_on(driver.query(&session, "SELECT txid_current_if_assigned()".into()))
        .expect("query");
    assert_eq!(
        txid.rows,
        [Box::new([Value::Null]) as crate::model::Row],
        "rolled back"
    );
}

#[test]
fn values_of_each_type_decode_as_specified() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    admin(
        &pg,
        &format!("CREATE TYPE {s}.mood AS ENUM ('sad', 'happy')"),
    );
    let session = session(&pg);
    let sql = format!(
        "SELECT true, 1::int2, 2::int4, 3::int8, 1.5::float4, 2.25::float8, 1.10::numeric, \
         'a'::text, 'b'::varchar, 'c'::name, 'd'::char(2), 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid, \
         'happy'::{s}.mood, '\\x0102'::bytea, '{{\"a\":1}}'::json, '{{\"a\":1}}'::jsonb, \
         '2026-01-02 10:42:01.5'::timestamp, '2026-01-02 10:42:01+02'::timestamptz, \
         '2026-01-02'::date, '10:42:01'::time, '1 day 2 hours'::interval, '10.0.0.1'::inet, \
         '{{1,2}}'::int4[], 'infinity'::timestamp, NULL::int4"
    );
    let rs = block_on(Postgres::default().query(&session, sql)).expect("query");
    let text = |s: &str| Value::Text(s.into());
    let want = [
        Value::Bool(true),
        Value::Int(1),
        Value::Int(2),
        Value::Int(3),
        Value::Float(1.5),
        Value::Float(2.25),
        text("1.10"),
        text("a"),
        text("b"),
        text("c"),
        text("d "),
        text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"),
        text("happy"),
        Value::Bytes([1u8, 2].into()),
        Value::Json("{\"a\":1}".into()),
        Value::Json("{\"a\": 1}".into()),
    ];
    assert_eq!(&rs.rows[0][..want.len()], &want);
    let rest: Vec<String> = rs.rows[0][want.len()..]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        rest,
        [
            "2026-01-02 10:42:01.5",
            "2026-01-02 08:42:01+00",
            "2026-01-02",
            "10:42:01",
            "1 day 02:00:00",
            "10.0.0.1",
            "{1,2}",
            "infinity",
            "NULL"
        ]
    );
    assert!(matches!(rs.rows[0][16], Value::Timestamp(_)));
    assert!(matches!(
        rs.rows[0][17],
        Value::TimestampTz { offset_secs: 0, .. }
    ));
    assert!(matches!(rs.rows[0][18], Value::Text(_)));
    let types: Vec<&str> = rs.columns.iter().map(|c| &*c.data_type).collect();
    assert_eq!(
        types[6..13],
        [
            "numeric", "text", "varchar", "name", "bpchar", "uuid", "mood"
        ]
    );
    assert_eq!(&*rs.columns[12].enum_values, ["sad".into(), "happy".into()]);
}

#[test]
fn an_autocommit_portal_commits_when_exhausted_or_closed() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    admin(&pg, &format!("CREATE TABLE {s}.t (a int)"));
    let session = session(&pg);
    let driver = Postgres::default();
    let txid = || {
        let rs = block_on(driver.query(&session, "SELECT txid_current_if_assigned()".into()));
        rs.expect("query").rows[0][0].clone()
    };
    let insert = format!("INSERT INTO {s}.t SELECT generate_series(1, 10) RETURNING a");

    block_on(driver.open(&session, insert.clone(), true)).expect("open");
    assert_eq!(
        block_on(driver.fetch(&session, 500)).expect("fetch").len(),
        10
    );
    assert_eq!(txid(), Value::Null, "exhausted");

    block_on(driver.open(&session, insert, true)).expect("open");
    assert_eq!(block_on(driver.fetch(&session, 2)).expect("fetch").len(), 2);
    assert_ne!(txid(), Value::Null, "still open mid-result");
    block_on(driver.close(&session)).expect("close");
    assert_eq!(txid(), Value::Null, "closed");
    assert_eq!(
        admin(&pg, &format!("SELECT count(*) FROM {s}.t")),
        ["20"],
        "both committed"
    );
}

#[test]
fn query_returns_the_whole_result_set() {
    let Some(pg) = test_pg() else { return };
    let session = session(&pg);
    let rs = block_on(Postgres::default().query(
        &session,
        "SELECT n, n::text AS s FROM generate_series(1, 3) n".into(),
    ))
    .expect("query");
    let names: Vec<&str> = rs.columns.iter().map(|c| &*c.name).collect();
    assert_eq!(names, ["n", "s"]);
    assert_eq!(ints(&rs.rows), [1, 2, 3]);
    assert_eq!(rs.rows[2][1], Value::Text("3".into()));
}

#[test]
fn result_columns_read_from_a_table_carry_its_key_marks() {
    let Some(pg) = test_pg() else { return };
    let session = session(&pg);
    let driver = Postgres::default();
    for ddl in [
        "CREATE TEMP TABLE qx_parent (id int PRIMARY KEY)",
        "CREATE TEMP TABLE qx_child (id int PRIMARY KEY, parent int REFERENCES qx_parent, note text)",
    ] {
        block_on(driver.open(&session, ddl.into(), true)).expect(ddl);
    }
    let opened = block_on(driver.open(
        &session,
        "SELECT id, parent, note, id + 1 AS computed FROM qx_child".into(),
        true,
    ));
    let Ok(Opened::Rows(columns)) = opened else {
        panic!("{opened:?}")
    };
    let marks: Vec<_> = columns.iter().map(|c| (c.is_pk, c.is_fk)).collect();
    assert_eq!(
        marks,
        [(true, false), (false, true), (false, false), (false, false)]
    );
}
