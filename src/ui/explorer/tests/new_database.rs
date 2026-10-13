//! New Database from a Postgres row's + New menu: CREATE DATABASE runs non-atomically on its
//! database's session, then the connection re-reads its databases.

use super::*;
use crate::model::Role;
use crate::ui::object_dialog::NameOwnerDialog;

fn role(name: &str) -> Role {
    Role {
        name: name.into(),
        can_login: true,
    }
}

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

/// Connected `pg` (production or not) with + New ▸ Database picked on app, its roles (alice, u)
/// answered and shop typed as the name.
fn open_new(
    cx: &mut gpui_kit::TestAppContext,
    production: bool,
) -> (
    gpui_kit::VisualTestContext,
    Arc<crate::db::fake::Fake>,
    gpui_kit::Entity<NameOwnerDialog>,
) {
    use std::{cell::RefCell, rc::Rc};
    let conn = crate::model::Connection {
        is_production: production,
        ..pg(1, "pg", "app")
    };
    let (mut cx, explorer, _, fake) = open_explorer(cx, vec![conn]);
    connect_answered(&explorer, &fake, &mut cx, &["app"]);
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(move |_: &mut NameOwnerDialog, _, cx| *sink.borrow_mut() = Some(cx.entity()))
    });
    let ix = labels(&explorer, &cx)
        .iter()
        .position(|l| l == "app")
        .expect("app row");
    let at = cx
        .debug_bounds(format!("tree-label-{ix}").leak())
        .expect("app drawn")
        .center();
    cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    click(&mut cx, "menu-new-database");
    let dialog = opened.take().expect("+ New ▸ Database opened the dialog");
    fake.resolve("roles app", Ok(vec![role("alice"), role("u")]));
    cx.run_until_parked();
    let name = dialog.read_with(&cx, |d, _| d.name.clone());
    type_into(&mut cx, name, "shop");
    (cx, fake, dialog)
}

#[gpui_kit::test]
fn new_database_creates_it_owned_by_the_connection_user_and_refreshes_the_connection(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, fake, dialog) = open_new(cx, false);
    let owner = dialog.read_with(&cx, |d, cx| {
        d.owner
            .as_ref()
            .expect("owner loaded")
            .read(cx)
            .selected_value()
            .cloned()
    });
    assert_eq!(owner.as_deref(), Some("u"), "owner defaults to the user");
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[r#"CREATE DATABASE "shop" OWNER "u""#]),
            false
        )]
    );
    fake.resolve("execute app", Ok(()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("dialog-layer").is_none(), "dialog closed");
    assert_eq!(
        fake.pending(),
        ["databases app"],
        "the connection re-reads its databases"
    );
}

#[gpui_kit::test]
fn on_production_apply_stays_disabled_until_the_new_name_is_typed(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, fake, dialog) = open_new(cx, true);
    click(&mut cx, "object-apply");
    assert!(fake.executed().is_empty(), "nothing typed: nothing run");
    let confirm = dialog.read_with(&cx, |d, _| d.confirm.typed.clone());
    for typed in ["sho", "Shop", "app"] {
        type_into(&mut cx, confirm.clone(), typed);
        click(&mut cx, "object-apply");
        assert!(fake.executed().is_empty(), "{typed:?}: nothing run");
    }
    type_into(&mut cx, confirm, "shop");
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[r#"CREATE DATABASE "shop" OWNER "u""#]),
            false
        )]
    );
}
