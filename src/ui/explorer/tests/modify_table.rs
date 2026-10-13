//! Modify… on a Postgres table: the editor lists its columns, and Apply runs `alter_table`
//! atomically on its database's session.

use super::*;
use crate::model::TableDef;
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

/// Modify… on table `t`, its definition answered: `id` (the PK), `a` and `b`.
fn open_editor(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<TableEditor>,
    Arc<crate::db::fake::Fake>,
) {
    let (mut cx, editor, fake) = open_unloaded(cx);
    answer_def(&fake, &mut cx);
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
        indexes: vec![],
        foreign_keys: vec![],
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
