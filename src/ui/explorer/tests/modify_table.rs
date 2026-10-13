//! Modify… on a Postgres table: the editor lists its columns, and Apply runs `alter_table`
//! atomically on its database's session.

use super::*;
use crate::model::{ForeignKey, Index, TableDef};
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
}

/// Modify… on table `t`, its definition answered: `id` (the PK), `a` and `b`; index `t_a_idx`
/// and FK `t_b_fk`.
fn open_editor(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<TableEditor>,
    Arc<crate::db::fake::Fake>,
) {
    let (mut cx, editor, fake) = open_unloaded(cx);
    answer_def(&fake, &mut cx);
    // The dialog slides in on the wall clock; clicks read bounds from a still frame.
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    (cx, editor, fake)
}

/// Modify… on table `t`, its definition not yet answered.
fn open_unloaded(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<TableEditor>,
    Arc<crate::db::fake::Fake>,
) {
    use std::{cell::RefCell, rc::Rc};
    let (mut cx, explorer, _, fake) = tables_open(cx);
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(move |_: &mut TableEditor, _, cx| *sink.borrow_mut() = Some(cx.entity()))
    });
    let ix = labels(&explorer, &cx)
        .iter()
        .position(|l| l == "t")
        .expect("t row");
    let at = cx
        .debug_bounds(format!("tree-label-{ix}").leak())
        .expect("t drawn")
        .center();
    cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
    cx.run_until_parked();
    click(&mut cx, "menu-modify");
    let editor = opened.take().expect("Modify… opened the editor");
    (cx, editor, fake)
}

fn answer_def(fake: &crate::db::fake::Fake, cx: &mut gpui_kit::VisualTestContext) {
    let mut id = column("id");
    id.nullable = false;
    id.is_pk = true;
    let def = TableDef {
        name: "t".into(),
        columns: vec![id, column("a"), column("b")],
        indexes: vec![Index {
            name: "t_a_idx".into(),
            columns: vec!["a".into()],
            unique: false,
        }],
        foreign_keys: vec![ForeignKey {
            name: "t_b_fk".into(),
            columns: vec!["b".into()],
            ref_schema: "public".into(),
            ref_table: "u".into(),
            ref_columns: vec!["id".into()],
            on_delete: "NO ACTION".into(),
            on_update: "NO ACTION".into(),
        }],
    };
    fake.resolve("table_def app/public/t", Ok(def));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_editor_shows_the_columns_from_table_def(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, editor, _) = open_editor(cx);
    let names = editor.read_with(&cx, |e, cx| {
        e.rows
            .iter()
            .map(|r| r.name.read(cx).value().to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(names, ["id", "a", "b"]);
    for selector in ["column-0", "column-1", "column-2", "column-pk-0"] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector} drawn");
    }
    assert!(cx.debug_bounds("column-pk-1").is_none(), "only id is a key");
}

#[gpui_kit::test]
fn renames_a_type_change_an_added_and_a_removed_column_run_atomically_and_refresh_the_schema(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, editor, fake) = open_editor(cx);
    let name = editor.read_with(&cx, |e, _| e.name.clone());
    type_into(&mut cx, name, "t2");
    let (a, id_type) = editor.read_with(&cx, |e, _| (e.rows[1].name.clone(), e.rows[0].ty.clone()));
    type_into(&mut cx, a, "a2");
    type_into(&mut cx, id_type, "bigint");
    click(&mut cx, "column-remove-2");
    click(&mut cx, "column-add");
    let (c, c_type) = editor.read_with(&cx, |e, _| (e.rows[2].name.clone(), e.rows[2].ty.clone()));
    type_into(&mut cx, c, "c");
    type_into(&mut cx, c_type, "text");
    cx.run_until_parked();
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[
                r#"ALTER TABLE "public"."t" RENAME TO "t2""#,
                r#"ALTER TABLE "public"."t2" DROP COLUMN "b""#,
                r#"ALTER TABLE "public"."t2" RENAME COLUMN "a" TO "a2""#,
                r#"ALTER TABLE "public"."t2" ALTER COLUMN "id" TYPE bigint"#,
                r#"ALTER TABLE "public"."t2" ADD COLUMN "c" text"#,
            ]),
            true
        )]
    );
    fake.resolve("execute app", Ok(()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("dialog-layer").is_none(), "dialog closed");
    assert_eq!(
        fake.pending(),
        ["objects app/public"],
        "the schema re-reads its objects"
    );
}

#[gpui_kit::test]
fn a_name_typed_before_the_definition_loads_is_kept(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, editor, fake) = open_unloaded(cx);
    let name = editor.read_with(&cx, |e, _| e.name.clone());
    type_into(&mut cx, name, "t2");
    answer_def(&fake, &mut cx);
    assert_eq!(
        editor.read_with(&cx, |e, _| e.statements()),
        strings(&[r#"ALTER TABLE "public"."t" RENAME TO "t2""#])
    );
}

#[gpui_kit::test]
fn the_editor_shows_the_indexes_and_fks_from_table_def(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, editor, _) = open_editor(cx);
    let (index, fk) = editor.read_with(&cx, |e, cx| {
        let value = |i: &gpui_kit::Entity<gpui_kit::component::input::InputState>| {
            i.read(cx).value().to_string()
        };
        let (i, f) = (&e.indexes[0], &e.foreign_keys[0]);
        (
            [value(&i.name), value(&i.columns)],
            [
                value(&f.name),
                value(&f.columns),
                value(&f.ref_schema),
                value(&f.ref_table),
                value(&f.ref_columns),
            ],
        )
    });
    assert_eq!(index, ["t_a_idx", "a"]);
    assert_eq!(fk, ["t_b_fk", "b", "public", "u", "id"]);
    for selector in ["index-0", "fk-0", "index-add", "fk-add"] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector} drawn");
    }
}

#[gpui_kit::test]
fn an_added_unique_index_and_cascading_fk_and_a_removed_index_run_in_alter_table_order(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, editor, fake) = open_editor(cx);
    click(&mut cx, "index-remove-0");
    click(&mut cx, "index-add");
    click(&mut cx, "index-unique-0");
    let (name, columns) = editor.read_with(&cx, |e, _| {
        (e.indexes[0].name.clone(), e.indexes[0].columns.clone())
    });
    type_into(&mut cx, name, "t_ab_key");
    type_into(&mut cx, columns, "a, b");
    click(&mut cx, "fk-add");
    let fk = editor.read_with(&cx, |e, _| {
        let f = &e.foreign_keys[1];
        (
            f.on_delete.clone(),
            [
                f.name.clone(),
                f.columns.clone(),
                f.ref_table.clone(),
                f.ref_columns.clone(),
            ],
        )
    });
    fk.0.update_in(&mut cx, |s, window, cx| {
        s.set_selected_value(&"CASCADE", window, cx)
    });
    for (input, text) in fk.1.into_iter().zip(["t_a_fk", "a", "v", "id"]) {
        type_into(&mut cx, input, text);
    }
    cx.run_until_parked();
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[
                r#"DROP INDEX "public"."t_a_idx""#,
                r#"CREATE UNIQUE INDEX "t_ab_key" ON "public"."t" ("a", "b")"#,
                r#"ALTER TABLE "public"."t" ADD CONSTRAINT "t_a_fk" FOREIGN KEY ("a") REFERENCES "public"."v" ("id") ON DELETE CASCADE ON UPDATE NO ACTION"#,
            ]),
            true
        )]
    );
}
