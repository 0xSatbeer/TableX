use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::*;
use crate::model::Column;

fn column(name: &str, data_type: &str) -> Column {
    Column {
        name: name.into(),
        data_type: data_type.into(),
        nullable: true,
        default: None,
        is_pk: false,
        enum_values: Vec::new(),
        identity: false,
    }
}

fn object(kind: ObjectKind, name: &str, columns: &[&str]) -> DbObject {
    DbObject {
        kind,
        name: name.into(),
        columns: Some(columns.iter().map(|c| column(c, "int4")).collect()),
        keys: vec![],
        indexes: vec![],
    }
}

fn snapshot() -> SchemaSnapshot {
    let public = vec![
        object(ObjectKind::Table, "users", &["id", "name", "email"]),
        object(ObjectKind::Table, "orders", &["id", "user_id", "total"]),
        object(ObjectKind::View, "user_stats", &["user_id", "orders"]),
        object(ObjectKind::Table, "role", &["rolname", "rolsuper"]),
        object(ObjectKind::Table, "Mixed Case", &["x"]),
        object(ObjectKind::Function, "add_tax", &[]),
    ];
    let audit = vec![object(ObjectKind::Table, "events", &["at", "kind"])];
    SchemaSnapshot {
        objects: HashMap::from([("public".into(), public), ("audit".into(), audit)]),
        current: "public".into(),
    }
}

/// `|` marks the cursor.
fn labels(sql_with_cursor: &str) -> Vec<String> {
    let offset = sql_with_cursor.find('|').expect("cursor");
    let sql = sql_with_cursor.replacen('|', "", 1);
    complete(&sql, offset, DbKind::Postgres, &snapshot())
        .into_iter()
        .map(|i| i.label)
        .collect()
}

fn has(sql: &str, want: &[&str]) -> bool {
    let got = labels(sql);
    want.iter().all(|w| got.iter().any(|g| g == w))
}

#[test]
fn context_cases() {
    let cases: [(&str, &[&str], &[&str]); 9] = [
        (
            "SELECT u.| FROM users u",
            &["id", "name", "email"],
            &["total", "SELECT"],
        ),
        (
            "SELECT * FROM us|",
            &["users", "user_stats"],
            &["orders", "id"],
        ),
        (
            "SELECT * FROM public.|",
            &["users", "orders", "user_stats"],
            &["events"],
        ),
        ("SELECT * FROM audit.e|", &["events"], &["users"]),
        ("SEL|", &["SELECT"], &["users"]),
        ("|", &["SELECT", "INSERT"], &["users"]),
        (
            "SELECT * FROM |",
            &["users", "audit", "public"],
            &["SELECT", "id"],
        ),
        ("SELECT * FROM users, o|", &["orders"], &["id"]),
        ("SELECT 1; SEL|", &["SELECT"], &[]),
    ];
    for (sql, want, not) in cases {
        let got = labels(sql);
        for w in want {
            assert!(got.contains(&w.to_string()), "{sql}: want {w} in {got:?}");
        }
        for n in not {
            assert!(!got.contains(&n.to_string()), "{sql}: no {n} in {got:?}");
        }
    }
}

#[test]
fn prefix_first_then_fuzzy() {
    assert_eq!(labels("SELECT * FROM users u WHERE u.em|"), ["email"]);
    let got = labels("SELECT * FROM ust|");
    assert_eq!(got.first().map(String::as_str), Some("user_stats"));
    let got = labels("SELECT * FROM o|");
    assert_eq!(got.first().map(String::as_str), Some("orders"));
}

#[test]
fn expression_position_offers_scope_columns_then_functions_then_keywords() {
    let got = labels("SELECT * FROM users WHERE |");
    let pos = |l: &str| {
        got.iter()
            .position(|g| g == l)
            .unwrap_or_else(|| panic!("{l}"))
    };
    assert!(pos("email") < pos("add_tax"));
    assert!(pos("add_tax") < pos("AND"));
    assert!(!got.contains(&"total".to_string()));
}

#[test]
fn expression_follows_the_grammar() {
    assert!(labels("SELECT * FROM users WHERE name ILIKE |").is_empty());
    assert!(labels("SELECT * FROM users WHERE name = |").is_empty());
    assert!(has("SELECT * FROM users WHERE name = e|", &["email"]));
    assert!(!has("SELECT * FROM users WHERE name = e|", &["EXISTS"]));
    for sql in [
        "SELECT * FROM users WHERE name |",
        "SELECT * FROM users WHERE name ILIKE '%a%' |",
        "SELECT * FROM users WHERE name IS NOT |",
        "SELECT name |",
    ] {
        assert!(has(sql, &["AND"]), "{sql}");
        assert!(!has(sql, &["email", "add_tax"]), "{sql}");
    }
    assert!(has("SELECT * FROM users WHERE name IS |", &["NULL"]));
    assert!(has(
        "SELECT * FROM users WHERE id = 1 AND |",
        &["email", "AND"]
    ));
}

#[test]
fn keyword_named_tables_resolve() {
    assert!(has("SELECT r.| FROM role r", &["rolname"]));
    assert!(has("SELECT role.| FROM role", &["rolname"]));
    assert!(has("SELECT * FROM role WHERE |", &["rolsuper"]));
    assert!(has("SELECT o.| FROM role r, orders o", &["total"]));
}

#[test]
fn join_and_as_aliases_resolve() {
    assert!(has(
        "SELECT o.| FROM users AS u JOIN orders o ON",
        &["total", "user_id"]
    ));
    assert!(has(
        "SELECT * FROM users AS u JOIN orders o ON o.user_id = u.|",
        &["email"]
    ));
    assert!(has(
        "SELECT * FROM users u LEFT JOIN orders AS o ON o.|",
        &["total"]
    ));
    assert!(has("SELECT x.| FROM audit.events x", &["kind"]));
    assert!(has("UPDATE users u SET name = u.|", &["email"]));
    assert!(has("SELECT users.| FROM users", &["email"]));
    assert!(!has("SELECT o.| FROM users u JOIN orders o", &["email"]));
}

#[test]
fn quoted_identifiers_insert_quoted() {
    let snap = snapshot();
    let sql = "SELECT * FROM Mix";
    let items = complete(sql, sql.len(), DbKind::Postgres, &snap);
    let mixed = items
        .iter()
        .find(|i| i.label == "Mixed Case")
        .expect("Mixed Case");
    assert_eq!(mixed.insert, "\"Mixed Case\"");
    let sql = "SELECT u.| FROM \"Mixed Case\" u";
    assert!(has(sql, &["x"]));
    let sql = "SELECT * FROM users u WHERE u.\"em";
    let items = complete(sql, sql.len(), DbKind::Postgres, &snap);
    assert_eq!(items[0].insert, "\"email\"");
    let sql = "SELECT * FROM users u WHERE u.em";
    let items = complete(sql, sql.len(), DbKind::Postgres, &snap);
    assert_eq!(items[0].insert, "email");
    assert_eq!(items[0].detail, "int4");
    assert_eq!(items[0].kind, ItemKind::Column);
    assert_eq!(
        items[0].qualifier, " (u)",
        "a column's qualifier is its alias"
    );
}

#[test]
fn nothing_inside_strings_or_comments() {
    assert!(labels("SELECT 'us|").is_empty());
    assert!(labels("SELECT 1 -- us|").is_empty());
}

#[test]
fn sqlite_has_its_own_keywords() {
    let sql = "PRAG";
    let items = complete(sql, sql.len(), DbKind::Sqlite, &snapshot());
    assert!(items.iter().any(|i| i.label == "PRAGMA"));
    let items = complete(sql, sql.len(), DbKind::Postgres, &snapshot());
    assert!(!items.iter().any(|i| i.label == "PRAGMA"));
}

#[test]
fn five_thousand_tables_answer_alias_columns_fast() {
    let mut snap = snapshot();
    let public = snap.objects.get_mut("public").expect("public");
    for i in 0..5000 {
        public.push(object(
            ObjectKind::Table,
            &format!("table_{i}"),
            &["a", "b", "c"],
        ));
    }
    let sql = "SELECT u. FROM users u";
    let started = Instant::now();
    let items = complete(sql, 9, DbKind::Postgres, &snap);
    let took = started.elapsed();
    assert!(items.iter().any(|i| i.label == "email"));
    assert!(took < Duration::from_millis(50), "took {took:?}");
}
