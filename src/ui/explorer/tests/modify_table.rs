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

/// Double-clicks `selector`, which opens a list row as a form.
fn double_click(cx: &mut gpui_kit::VisualTestContext, selector: &'static str) {
    let position = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} drawn"))
        .center();
    let button = gpui_kit::MouseButton::Left;
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position,
        button,
        modifiers: Default::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui_kit::MouseUpEvent {
        position,
        button,
        modifiers: Default::default(),
        click_count: 2,
    });
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
    open_editor_with(cx, |_| {})
}

/// `open_editor`, its definition changed by `edit` first.
fn open_editor_with(
    cx: &mut gpui_kit::TestAppContext,
    edit: impl FnOnce(&mut TableDef),
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<TableEditor>,
    Arc<crate::db::fake::Fake>,
) {
    let (mut cx, editor, fake) = open_unloaded(cx);
    answer_def_with(&fake, &mut cx, edit);
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
    answer_def_with(fake, cx, |_| {})
}

fn answer_def_with(
    fake: &crate::db::fake::Fake,
    cx: &mut gpui_kit::VisualTestContext,
    edit: impl FnOnce(&mut TableDef),
) {
    let mut id = column("id");
    id.nullable = false;
    id.is_pk = true;
    let mut def = TableDef {
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
        primary_key: Some("t_pkey".into()),
        unique_keys: vec![],
    };
    edit(&mut def);
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
    double_click(&mut cx, "column-1");
    type_into(&mut cx, a, "a2");
    double_click(&mut cx, "column-0");
    type_into(&mut cx, id_type, "bigint");
    click(&mut cx, "column-2");
    click(&mut cx, "column-remove");
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
    click(&mut cx, "tab-indexes");
    for selector in ["index-0", "index-add"] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector} drawn");
    }
    click(&mut cx, "tab-fks");
    for selector in ["fk-0", "fk-add"] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector} drawn");
    }
}

#[gpui_kit::test]
fn an_added_unique_index_and_cascading_fk_and_a_removed_index_run_in_alter_table_order(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, editor, fake) = open_editor(cx);
    click(&mut cx, "tab-indexes");
    click(&mut cx, "index-0");
    click(&mut cx, "index-remove");
    click(&mut cx, "index-add");
    click(&mut cx, "index-unique");
    let (name, columns) = editor.read_with(&cx, |e, _| {
        (e.indexes[0].name.clone(), e.indexes[0].columns.clone())
    });
    type_into(&mut cx, name, "t_ab_key");
    type_into(&mut cx, columns, "a, b");
    click(&mut cx, "tab-fks");
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

#[gpui_kit::test]
fn a_double_click_opens_a_row_as_a_form_and_escape_closes_only_the_form(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, _editor, _fake) = open_editor(cx);
    click(&mut cx, "column-1");
    assert!(
        cx.debug_bounds("column-not-null").is_none(),
        "a click only selects"
    );
    double_click(&mut cx, "column-1");
    assert!(
        cx.debug_bounds("column-not-null").is_some(),
        "the form is open"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("column-not-null").is_none(),
        "escape closed the form"
    );
    assert!(
        cx.debug_bounds("dialog-layer").is_some(),
        "the dialog stays open"
    );
    click(&mut cx, "column-add");
    assert!(
        cx.debug_bounds("column-not-null").is_some(),
        "a new row opens as a form"
    );
}

fn unique_key(name: &str, columns: &[&str]) -> Index {
    Index {
        name: name.into(),
        columns: columns.iter().map(|c| c.to_string()).collect(),
        unique: true,
    }
}

#[gpui_kit::test]
fn the_column_form_checkboxes_reflect_and_toggle_the_flags(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, editor, _) = open_editor_with(cx, |def| {
        def.columns[2].identity = true;
        def.unique_keys.push(unique_key("t_b_key", &["b"]));
    });
    double_click(&mut cx, "column-2");
    // Each click flips the box, so the flag it lands on shows what the box held.
    let flags = |cx: &mut gpui_kit::VisualTestContext| {
        editor.read_with(cx, |e, _| {
            let form = e.form.as_ref().expect("loaded");
            let b = &form.columns[2].1;
            [!b.nullable, b.identity, form.is_unique(2), b.is_pk]
        })
    };
    assert_eq!(flags(&mut cx), [false, true, true, false]);
    click(&mut cx, "column-identity");
    click(&mut cx, "column-unique");
    assert_eq!(flags(&mut cx), [false, false, false, false]);
    click(&mut cx, "column-pk");
    assert_eq!(
        flags(&mut cx),
        [true, false, false, true],
        "ticking Primary key ticks Not null"
    );
    click(&mut cx, "column-not-null");
    assert_eq!(flags(&mut cx), [false, false, false, true]);
}

#[gpui_kit::test]
fn ticking_unique_and_auto_increment_adds_the_constraint_and_identity(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, _, fake) = open_editor(cx);
    double_click(&mut cx, "column-1");
    click(&mut cx, "column-unique");
    click(&mut cx, "column-identity");
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[
                r#"ALTER TABLE "public"."t" ALTER COLUMN "a" ADD GENERATED BY DEFAULT AS IDENTITY"#,
                r#"ALTER TABLE "public"."t" ADD CONSTRAINT "t_a_key" UNIQUE ("a")"#,
            ]),
            true
        )]
    );
}

#[gpui_kit::test]
fn moving_the_pk_drops_the_old_one_by_name_then_adds_the_new(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, _, fake) = open_editor(cx);
    double_click(&mut cx, "column-0");
    click(&mut cx, "column-pk");
    double_click(&mut cx, "column-1");
    click(&mut cx, "column-pk");
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[
                r#"ALTER TABLE "public"."t" DROP CONSTRAINT "t_pkey""#,
                r#"ALTER TABLE "public"."t" ALTER COLUMN "a" SET NOT NULL"#,
                r#"ALTER TABLE "public"."t" ADD PRIMARY KEY ("a")"#,
            ]),
            true
        )]
    );
}

#[gpui_kit::test]
fn a_multi_column_unique_constraint_is_listed_after_the_indexes_and_removable(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, editor, _) = open_editor_with(cx, |def| {
        def.unique_keys.push(unique_key("t_ab_key", &["a", "b"]));
    });
    click(&mut cx, "tab-indexes");
    double_click(&mut cx, "index-1");
    assert!(
        cx.debug_bounds("index-unique").is_none(),
        "a constraint opens no form"
    );
    click(&mut cx, "index-remove");
    assert!(cx.debug_bounds("index-1").is_none(), "the line is gone");
    assert_eq!(
        editor.read_with(&cx, |e, _| e.statements()),
        strings(&[r#"ALTER TABLE "public"."t" DROP CONSTRAINT "t_ab_key""#])
    );
}

#[gpui_kit::test]
fn renaming_a_column_through_another_ones_name_leaves_that_ones_key_alone(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, editor, _) = open_editor_with(cx, |def| {
        def.unique_keys.push(unique_key("t_b_key", &["b"]));
    });
    let a = editor.read_with(&cx, |e, _| e.rows[1].name.clone());
    double_click(&mut cx, "column-1");
    type_into(&mut cx, a.clone(), "b");
    cx.run_until_parked();
    type_into(&mut cx, a, "c");
    cx.run_until_parked();
    let unique = editor.read_with(&cx, |e, _| {
        let form = e.form.as_ref().expect("loaded");
        [form.is_unique(1), form.is_unique(2)]
    });
    assert_eq!(unique, [false, true], "b keeps its key");
    assert_eq!(
        editor.read_with(&cx, |e, _| e.statements()),
        strings(&[r#"ALTER TABLE "public"."t" RENAME COLUMN "a" TO "c""#])
    );
}
