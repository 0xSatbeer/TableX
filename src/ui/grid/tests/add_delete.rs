//! Added and deleted rows as INSERTs and DELETEs, batched with the UPDATEs.

use std::collections::{BTreeSet, HashMap};

use super::super::submit::changes;
use super::*;
use gpui_kit::AppContext as _;

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
            vec![Value::Int(3), Value::Text("c@x".into()), Value::Null].into(),
        ],
    }
}

const USERS: (&str, &str) = ("public", "users");

fn sql(
    all: &dyn GridSource,
    added: usize,
    edits: &HashMap<(usize, usize), Value>,
    deleted: &[usize],
) -> Vec<String> {
    let deleted: BTreeSet<usize> = deleted.iter().copied().collect();
    changes(
        (USERS.0, USERS.1, &["id".into()]),
        all,
        added,
        edits,
        &deleted,
    )
    .into_iter()
    .map(|(_, sql)| sql)
    .collect()
}

#[gpui_kit::test]
fn an_added_row_inserts_only_its_filled_cells(cx: &mut gpui_kit::TestAppContext) {
    let grid = cx.new(|cx| Grid::new(std::rc::Rc::new(users()), cx));
    grid.update(cx, |g, cx| {
        g.add_row(cx);
        g.set_cell((0, 1), Value::Text("d@x".into()), cx);
        g.set_cell((0, 2), Value::Text("Dee".into()), cx);
        g.add_row(cx);
        assert_eq!(
            sql(&*g.all, g.added, &g.edits, &[]),
            [
                r#"INSERT INTO "public"."users" DEFAULT VALUES"#,
                r#"INSERT INTO "public"."users" ("email", "name") VALUES ('d@x', 'Dee')"#,
            ],
            "a blank row takes every default; the filled one names only its cells"
        );
    });
}

#[gpui_kit::test]
fn deletes_go_first_keyed_by_the_primary_key_then_updates_then_inserts(
    cx: &mut gpui_kit::TestAppContext,
) {
    let grid = cx.new(|cx| Grid::new(std::rc::Rc::new(users()), cx));
    grid.update(cx, |g, cx| {
        g.set_cell((1, 2), Value::Text("Bo".into()), cx);
        g.set_cell((2, 2), Value::Text("gone".into()), cx);
        for r in [0, 2] {
            (g.anchor, g.head) = ((r, 0), (r, 0));
            g.delete_rows(cx);
        }
        g.add_row(cx);
        assert_eq!(
            g.deleted,
            BTreeSet::from([1, 3]),
            "marks move down with the new row"
        );
        g.set_cell((0, 2), Value::Text("Dee".into()), cx);
        assert_eq!(
            sql(&*g.all, g.added, &g.edits, &[1, 3]),
            [
                r#"DELETE FROM "public"."users" WHERE "id" = 1"#,
                r#"DELETE FROM "public"."users" WHERE "id" = 3"#,
                r#"UPDATE "public"."users" SET "name" = 'Bo' WHERE "id" = 2"#,
                r#"INSERT INTO "public"."users" ("name") VALUES ('Dee')"#,
            ],
            "a deleted row's edits are dropped"
        );
        (g.anchor, g.head) = ((0, 0), (0, 0));
        g.delete_rows(cx);
        assert_eq!(
            g.deleted,
            BTreeSet::from([0, 2]),
            "and back up when it goes"
        );
    });
}

#[gpui_kit::test]
fn deleting_an_added_row_removes_it_with_no_sql(cx: &mut gpui_kit::TestAppContext) {
    let grid = cx.new(|cx| Grid::new(std::rc::Rc::new(users()), cx));
    grid.update(cx, |g, cx| {
        g.set_cell((1, 2), Value::Text("Bo".into()), cx);
        g.add_row(cx);
        g.set_cell((0, 1), Value::Text("d@x".into()), cx);
        assert_eq!(g.row_count(), 4);
        (g.anchor, g.head) = ((0, 0), (0, 0));
        g.delete_rows(cx);
        assert_eq!(g.row_count(), 3, "the added row is gone");
        assert_eq!(
            sql(&*g.all, g.added, &g.edits, &[]),
            [r#"UPDATE "public"."users" SET "name" = 'Bo' WHERE "id" = 2"#],
            "its insert is gone and the other edit is back on its row"
        );
        (g.anchor, g.head) = ((0, 0), (1, 0));
        g.delete_rows(cx);
        assert_eq!(
            g.deleted,
            BTreeSet::from([0, 1]),
            "a loaded row is only marked"
        );
    });
}
