//! What quick open asks of the explorer: loading every object, and opening one by key.

use super::*;

#[gpui_kit::test]
fn loading_all_objects_loads_unloaded_schemas_of_connected_connections_only_without_expanding(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut off = loaded(pg(2, "off", "off"), "off", "public", vec![]);
    off.status = ConnectionStatus::Disconnected;
    off.databases.as_mut().unwrap()[0].schemas.as_mut().unwrap()[0].objects = None;
    let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app"), off]);
    connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
    expand(&explorer, &mut cx, "app");
    fake.resolve("schemas app", Ok(strings(&["public", "empty"])));
    cx.run_until_parked();

    explorer.update_in(&mut cx, |e, _, cx| e.load_all_objects(cx));
    cx.run_until_parked();
    let mut pending = fake.pending();
    pending.sort();
    assert_eq!(
        pending,
        [
            "connect analytics",
            "objects app/empty",
            "objects app/public"
        ]
    );
    assert_eq!(explorer.read_with(&cx, |e, _| e.schemas_pending()), 3);

    fake.resolve(
        "connect analytics",
        Ok(crate::db::fake::session("analytics")),
    );
    cx.run_until_parked();
    fake.resolve("schemas analytics", Ok(strings(&["s"])));
    cx.run_until_parked();
    assert!(
        fake.pending().contains(&"objects analytics/s".to_string()),
        "a database that arrives loads its schemas: {:?}",
        fake.pending()
    );
    assert!(!is_expanded(&explorer, &cx, "analytics"), "nothing expands");
    assert!(!is_expanded(&explorer, &cx, "public"), "nothing expands");
}

#[gpui_kit::test]
fn open_key_reveals_the_object_and_emits_open_object(cx: &mut gpui_kit::TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    let conn = loaded(pg(1, "pg", "app"), "app", "public", vec![table("users")]);
    let (mut cx, explorer, _, _) = open_explorer(cx, vec![conn]);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&explorer, move |_, ev: &super::OpenObject, _| {
            sink.borrow_mut().push((ev.key.to_string(), ev.preview))
        })
    });
    explorer.update_in(&mut cx, |e, window, cx| {
        e.open_key("1/app/public/table/users", window, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        *opened.borrow(),
        [("1/app/public/table/users".to_string(), false)]
    );
    assert!(
        is_expanded(&explorer, &cx, "Tables"),
        "revealed in the tree"
    );
}
