//! Drop… on an object: on production the name is typed first; the DROP runs atomically on the
//! object's database, then its schema re-reads and its tabs are told to close.

use super::*;
use crate::ui::object_dialog::DropDialog;

#[gpui_kit::test]
fn drop_on_production_needs_the_name_typed_then_runs_the_drop_and_refreshes_the_schema(
    cx: &mut gpui_kit::TestAppContext,
) {
    use std::{cell::RefCell, rc::Rc};
    let mut conn = pg(1, "pg", "app");
    conn.is_production = true;
    let (mut cx, explorer, _, fake) = open_explorer(cx, vec![conn]);
    connect_answered(&explorer, &fake, &mut cx, &["app"]);
    for step in ["app", "public", "Tables"] {
        expand(&explorer, &mut cx, step);
        for pending in fake.pending() {
            answer(&fake, &pending, Ok(()));
        }
        cx.run_until_parked();
    }
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(move |_: &mut DropDialog, _, cx| *sink.borrow_mut() = Some(cx.entity()))
    });
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let sink = dropped.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&explorer, move |_, ev: &super::super::ObjectDropped, _| {
            sink.borrow_mut().push(ev.key.clone())
        })
    });
    right_click(&explorer, &mut cx, "t");
    click(&mut cx, "menu-drop");
    let dialog = opened.take().expect("Drop… opened the dialog");
    assert!(cx.debug_bounds("object-sql").is_some(), "the SQL is shown");

    click(&mut cx, "object-apply");
    assert!(fake.executed().is_empty(), "Apply waits for the name");
    let typed = dialog.read_with(&cx, |d, _| d.confirm.typed.clone());
    cx.update(|window, cx| typed.update(cx, |input, cx| input.focus(window, cx)));
    cx.simulate_input("t");
    cx.run_until_parked();
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[r#"DROP TABLE "public"."t""#]),
            true
        )]
    );
    fake.resolve_query("execute app", Ok(()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("dialog-layer").is_none(), "dialog closed");
    assert_eq!(
        fake.pending(),
        ["objects app/public"],
        "the schema re-reads"
    );
    assert_eq!(*dropped.borrow(), ["1/app/public/table/t".to_string()]);
}
