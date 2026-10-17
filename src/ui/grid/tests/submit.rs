//! The UPDATEs pending edits become, and which tables can't take them.

use std::collections::HashMap;

use super::super::submit::{read_only_note, updates};
use super::*;
use crate::model::ResultSet;

fn users() -> ResultSet {
    ResultSet {
        columns: vec![
            col("id", "int4", true),
            col("email", "text", false),
            col("name", "text", false),
        ],
        rows: vec![
            vec![Value::Int(1), Value::Text("a@x".into()), Value::Null].into(),
            vec![Value::Int(2), Value::Text("b@x".into()), Value::Null].into(),
        ],
    }
}

#[test]
fn two_edited_cells_in_one_row_give_one_update_keyed_by_the_primary_key() {
    let edits = HashMap::from([
        ((1, 2), Value::Text("O'Neil".into())),
        ((1, 1), Value::Null),
    ]);
    assert_eq!(
        updates(("public", "users", &["id".into()]), &users(), &edits),
        [(
            1,
            r#"UPDATE "public"."users" SET "email" = NULL, "name" = 'O''Neil' WHERE "id" = 2"#
                .to_string()
        )]
    );
}

#[test]
fn an_edited_key_is_matched_by_its_old_value_and_rows_go_in_order() {
    let edits = HashMap::from([((1, 0), Value::Int(9)), ((0, 2), Value::Int(5))]);
    let sql: Vec<_> = updates(("public", "users", &["id".into()]), &users(), &edits)
        .into_iter()
        .collect();
    assert_eq!(
        sql,
        [
            (
                0,
                r#"UPDATE "public"."users" SET "name" = 5 WHERE "id" = 1"#.to_string()
            ),
            (
                1,
                r#"UPDATE "public"."users" SET "id" = 9 WHERE "id" = 2"#.to_string()
            ),
        ]
    );
}

#[test]
fn a_keyless_sqlite_table_is_keyed_by_its_rowid() {
    let rs = ResultSet {
        columns: vec![col("rowid", "integer", false), col("msg", "text", false)],
        rows: vec![vec![Value::Int(7), Value::Text("hi".into())].into()],
    };
    let edits = HashMap::from([((0, 1), Value::Text("bye".into()))]);
    assert_eq!(
        updates(("main", "log", &["rowid".into()]), &rs, &edits),
        [(
            0,
            r#"UPDATE "main"."log" SET "msg" = 'bye' WHERE "rowid" = 7"#.to_string()
        )]
    );
}

#[test]
fn a_postgres_table_without_a_key_and_every_view_are_read_only() {
    assert_eq!(
        read_only_note(false, false, &[]),
        Some("No primary key: read-only")
    );
    assert_eq!(
        read_only_note(true, false, &["id".into()]),
        Some("Views are read-only")
    );
    assert_eq!(
        read_only_note(true, false, &[]),
        Some("Views are read-only")
    );
    assert_eq!(
        read_only_note(false, true, &[]),
        None,
        "SQLite uses the rowid"
    );
    assert_eq!(read_only_note(false, false, &["id".into()]), None);
}
