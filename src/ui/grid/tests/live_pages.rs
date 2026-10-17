//! The SQL a table tab reads its pages and count with.

use super::super::live::{count_sql, page_sql};

#[test]
fn pages_order_by_the_primary_key_and_quote_names() {
    assert_eq!(
        page_sql(
            "pub\"lic",
            "order items",
            &["a".into(), "b".into()],
            false,
            ("", ""),
            1_000
        ),
        r#"SELECT * FROM "pub""lic"."order items" ORDER BY "a", "b" LIMIT 500 OFFSET 1000"#
    );
}

#[test]
fn a_keyless_table_has_no_order_and_sqlite_reads_its_rowid() {
    assert_eq!(
        page_sql("public", "log", &[], false, ("", ""), 0),
        r#"SELECT * FROM "public"."log" LIMIT 500 OFFSET 0"#
    );
    assert_eq!(
        page_sql("main", "log", &[], true, ("", ""), 0),
        r#"SELECT rowid, * FROM "main"."log" LIMIT 500 OFFSET 0"#
    );
}

#[test]
fn count_reads_the_whole_table() {
    assert_eq!(
        count_sql("public", "users", ""),
        r#"SELECT COUNT(*) FROM "public"."users""#
    );
}

#[test]
fn a_filter_is_inserted_raw_and_its_order_replaces_the_key() {
    assert_eq!(
        page_sql(
            "public",
            "users",
            &["id".into()],
            false,
            ("a = 1 OR b", "email DESC"),
            0
        ),
        r#"SELECT * FROM "public"."users" WHERE (a = 1 OR b) ORDER BY email DESC LIMIT 500 OFFSET 0"#
    );
    assert_eq!(
        page_sql("public", "users", &["id".into()], false, ("a = 1", ""), 0),
        r#"SELECT * FROM "public"."users" WHERE (a = 1) ORDER BY "id" LIMIT 500 OFFSET 0"#
    );
    assert_eq!(
        count_sql("public", "users", "a = 1"),
        r#"SELECT COUNT(*) FROM "public"."users" WHERE (a = 1)"#
    );
}
