use super::*;

fn parts(sql: &str) -> Vec<&str> {
    split(sql).into_iter().map(|r| &sql[r]).collect()
}

#[test]
fn split_cases() {
    let cases: [(&str, &[&str]); 14] = [
        ("", &[]),
        ("  ;; \n ;", &[]),
        ("SELECT 1", &["SELECT 1"]),
        (" SELECT 1 ;\n SELECT 2; ", &["SELECT 1", "SELECT 2"]),
        ("SELECT 'a;b'; SELECT 2", &["SELECT 'a;b'", "SELECT 2"]),
        ("SELECT 'it''s;'; X", &["SELECT 'it''s;'", "X"]),
        ("SELECT \"a;\"\"b\"; X", &["SELECT \"a;\"\"b\"", "X"]),
        ("SELECT $$a;b$$; X", &["SELECT $$a;b$$", "X"]),
        ("SELECT $f$ $$; $f$; X", &["SELECT $f$ $$; $f$", "X"]),
        ("SELECT $1; X", &["SELECT $1", "X"]),
        ("SELECT 1 -- c;\n; X", &["SELECT 1 -- c;", "X"]),
        (
            "SELECT /* a; /* b; */ c; */ 1; X",
            &["SELECT /* a; /* b; */ c; */ 1", "X"],
        ),
        ("-- only;\n; /* only; */ ; X", &["X"]),
        ("SELECT 'unterminated; X", &["SELECT 'unterminated; X"]),
    ];
    for (sql, want) in cases {
        assert_eq!(parts(sql), want, "{sql:?}");
    }
}

#[test]
fn at_cursor_picks_the_statement() {
    let sql = "SELECT 1;  SELECT 22;\n\n";
    let at = |o| at_cursor(sql, o).map(|r| &sql[r]);
    assert_eq!(at(0), Some("SELECT 1"));
    assert_eq!(at(3), Some("SELECT 1"));
    assert_eq!(at(8), Some("SELECT 1"));
    assert_eq!(at(9), Some("SELECT 1"));
    assert_eq!(at(10), Some("SELECT 1"));
    assert_eq!(at(11), Some("SELECT 22"));
    assert_eq!(at(20), Some("SELECT 22"));
    assert_eq!(at(sql.len()), Some("SELECT 22"));
    assert_eq!(at_cursor("  ", 1), None);
}

#[test]
fn read_only_cases() {
    let cases = [
        ("SELECT 1", true),
        ("  select * from t", true),
        ("-- hi\n/* x */ SELECT 1", true),
        ("WITH x AS (SELECT 1) SELECT * FROM x", true),
        (
            "WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x",
            false,
        ),
        ("with x as (insert into t values (1)) select 1", false),
        ("WITH x AS (SELECT 'delete') SELECT \"update\"", true),
        ("EXPLAIN SELECT 1", true),
        ("EXPLAIN ANALYZE DELETE FROM t", false),
        ("explain (analyze) select 1", false),
        ("EXPLAIN ANALYSE DELETE FROM t", false),
        ("SHOW search_path", true),
        ("VALUES (1), (2)", true),
        ("TABLE users", true),
        ("PRAGMA table_info(t)", true),
        ("PRAGMA foo = 1", false),
        ("PRAGMA foo('=')", true),
        ("INSERT INTO t VALUES (1)", false),
        ("UPDATE t SET a = 1", false),
        ("DROP TABLE t", false),
        ("SELECTED", false),
        ("", false),
    ];
    for (sql, want) in cases {
        assert_eq!(is_read_only(sql), want, "{sql:?}");
    }
}

#[test]
fn line_col_is_one_based() {
    let sql = "SELECT\n  é x\nfoo";
    assert_eq!(line_col(sql, 0), (1, 1));
    assert_eq!(line_col(sql, 3), (1, 4));
    assert_eq!(line_col(sql, 7), (2, 1));
    assert_eq!(line_col(sql, 11), (2, 4));
    assert_eq!(line_col(sql, sql.len()), (3, 4));
}
