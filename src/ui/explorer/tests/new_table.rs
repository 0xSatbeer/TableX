//! "+ New ▸ Table" on a Postgres row: the table editor opens empty in the row's schema, and Apply
//! runs `create_table` atomically on its database's session.

use super::*;
use crate::ui::table_editor::TableEditor;

fn click(cx: &mut gpui_kit::VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} drawn"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

fn type_into(
    cx: &mut gpui_kit::VisualTestContext,
    input: gpui_kit::Entity<gpui_kit::component::input::InputState>,
    text: &str,
) {
    cx.update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
    cx.simulate_keystrokes("ctrl-a backspace");
    cx.simulate_input(text);
    cx.run_until_parked();
}

/// "+ New ▸ Table" picked on the `row` row of `pg` (production or not), with the tables of
/// app/public listed.
fn open_new(
    cx: &mut gpui_kit::TestAppContext,
    row: &str,
    production: bool,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<TableEditor>,
    Arc<crate::db::fake::Fake>,
) {
    use std::{cell::RefCell, rc::Rc};
    let (mut cx, explorer, workspace, fake) = tables_open(cx);
    workspace.update(&mut cx, |ws, _| {
        ws.connections[0].is_production = production
    });
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(move |_: &mut TableEditor, _, cx| *sink.borrow_mut() = Some(cx.entity()))
    });
    let ix = labels(&explorer, &cx)
        .iter()
        .position(|l| l == row)
        .unwrap_or_else(|| panic!("{row} row"));
    let at = cx
        .debug_bounds(format!("tree-label-{ix}").leak())
        .unwrap_or_else(|| panic!("{row} drawn"))
        .center();
    cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    click(&mut cx, "menu-new-table");
    let editor = opened.take().expect("+ New ▸ Table opened the editor");
    // The dialog slides in on the wall clock; clicks read bounds from a still frame.
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    (cx, editor, fake)
}

/// Names the table `n` and its one column `a int`.
fn fill_minimal(cx: &mut gpui_kit::VisualTestContext, editor: &gpui_kit::Entity<TableEditor>) {
    let (name, a, a_type) = editor.read_with(cx, |e, _| {
        (e.name.clone(), e.rows[0].name.clone(), e.rows[0].ty.clone())
    });
    type_into(cx, name, "n");
    type_into(cx, a, "a");
    type_into(cx, a_type, "int");
}

#[gpui_kit::test]
fn the_new_table_goes_into_the_rows_schema(cx: &mut gpui_kit::TestAppContext) {
    for (row, schema) in [
        ("app", "public"),
        ("public", "public"),
        ("sibling", "sibling"),
        ("t", "public"),
    ] {
        let (mut cx, editor, fake) = open_new(cx, row, false);
        assert!(
            fake.pending().iter().all(|p| !p.starts_with("table_def")),
            "{row}: create mode loads no definition"
        );
        let (name, rows) = editor.read_with(&cx, |e, cx| {
            (e.name.read(cx).value().to_string(), e.rows.len())
        });
        assert_eq!(
            (name.as_str(), rows),
            ("", 1),
            "{row}: empty name, one column row"
        );
        fill_minimal(&mut cx, &editor);
        assert_eq!(
            editor.read_with(&cx, |e, _| e.statements()),
            [format!(r#"CREATE TABLE "{schema}"."n" ("a" int)"#)],
            "{row}"
        );
    }
}

#[gpui_kit::test]
fn two_columns_an_index_and_an_fk_run_atomically_and_refresh_the_schema(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, editor, fake) = open_new(cx, "public", false);
    fill_minimal(&mut cx, &editor);
    click(&mut cx, "column-add");
    let (b, b_type) = editor.read_with(&cx, |e, _| (e.rows[1].name.clone(), e.rows[1].ty.clone()));
    type_into(&mut cx, b, "b");
    type_into(&mut cx, b_type, "int");
    click(&mut cx, "tab-indexes");
    click(&mut cx, "index-add");
    let (index_name, index_columns) = editor.read_with(&cx, |e, _| {
        (e.indexes[0].name.clone(), e.indexes[0].columns.clone())
    });
    type_into(&mut cx, index_name, "n_a_idx");
    type_into(&mut cx, index_columns, "a");
    click(&mut cx, "tab-fks");
    click(&mut cx, "fk-add");
    let fk = editor.read_with(&cx, |e, _| {
        let f = &e.foreign_keys[0];
        [
            f.name.clone(),
            f.columns.clone(),
            f.ref_table.clone(),
            f.ref_columns.clone(),
        ]
    });
    for (input, text) in fk.into_iter().zip(["n_b_fk", "b", "u", "id"]) {
        type_into(&mut cx, input, text);
    }
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[
                r#"CREATE TABLE "public"."n" ("a" int, "b" int)"#,
                r#"CREATE INDEX "n_a_idx" ON "public"."n" ("a")"#,
                r#"ALTER TABLE "public"."n" ADD CONSTRAINT "n_b_fk" FOREIGN KEY ("b") REFERENCES "public"."u" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION"#,
            ]),
            true
        )]
    );
    fake.resolve_query("execute app", Ok(()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("dialog-layer").is_none(), "dialog closed");
    assert_eq!(
        fake.pending(),
        ["objects app/public"],
        "the schema re-reads its objects"
    );
}

#[gpui_kit::test]
fn on_production_apply_waits_for_the_new_table_name(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, editor, fake) = open_new(cx, "public", true);
    fill_minimal(&mut cx, &editor);
    click(&mut cx, "object-apply");
    assert!(fake.executed().is_empty(), "nothing typed: nothing run");
    let confirm = editor.read_with(&cx, |e, _| e.confirm.typed.clone());
    type_into(&mut cx, confirm, "n");
    click(&mut cx, "object-apply");
    assert_eq!(fake.executed().len(), 1, "the new name confirms");
}

#[gpui_kit::test]
fn an_identity_pk_column_is_created_inline(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, editor, fake) = open_new(cx, "public", false);
    fill_minimal(&mut cx, &editor);
    click(&mut cx, "column-identity");
    click(&mut cx, "column-pk");
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[
                r#"CREATE TABLE "public"."n" ("a" int GENERATED BY DEFAULT AS IDENTITY NOT NULL, PRIMARY KEY ("a"))"#,
            ]),
            true
        )]
    );
}
