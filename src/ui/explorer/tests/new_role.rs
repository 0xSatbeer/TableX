//! + New ▸ Role and User on a Postgres row: the statements run atomically on its database's session.

use super::*;
use crate::model::Role;
use crate::ui::object_dialog::RoleDialog;

fn role(name: &str) -> Role {
    Role {
        name: name.into(),
        can_login: false,
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
    cx.simulate_input(text);
    cx.run_until_parked();
}

/// Connected `pg` with database app, + New ▸ `item` picked on app's `row` and its roles (alice,
/// bob) answered.
fn open_new(
    cx: &mut gpui_kit::TestAppContext,
    row: &str,
    item: &'static str,
) -> (
    gpui_kit::VisualTestContext,
    Arc<crate::db::fake::Fake>,
    gpui_kit::Entity<RoleDialog>,
) {
    use std::{cell::RefCell, rc::Rc};
    let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
    connect_answered(&explorer, &fake, &mut cx, &["app"]);
    expand(&explorer, &mut cx, "app");
    answer(&fake, "schemas app", Ok(()));
    cx.run_until_parked();
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(move |_: &mut RoleDialog, _, cx| *sink.borrow_mut() = Some(cx.entity()))
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
    click(&mut cx, item);
    let dialog = opened.take().expect("+ New opened the dialog");
    fake.resolve("roles app", Ok(vec![role("alice"), role("bob")]));
    cx.run_until_parked();
    (cx, fake, dialog)
}

#[gpui_kit::test]
fn new_user_creates_a_login_role_with_its_password_and_membership_atomically(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, fake, dialog) = open_new(cx, "public", "menu-new-user");
    let (name, password) = dialog.read_with(&cx, |d, _| {
        (
            d.name.clone(),
            d.password.clone().expect("a password field"),
        )
    });
    assert!(
        password.read_with(&cx, |p, _| p.presentation().is_masked()),
        "password masked"
    );
    type_into(&mut cx, name, "carol");
    type_into(&mut cx, password, "s3cret");
    click(&mut cx, "member-bob");
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[
                r#"CREATE ROLE "carol" LOGIN PASSWORD 's3cret'"#,
                r#"GRANT "bob" TO "carol""#,
            ]),
            true
        )]
    );
    fake.resolve("execute app", Ok(()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("dialog-layer").is_none(), "dialog closed");
    assert!(fake.pending().is_empty(), "nothing refreshes");
}

#[gpui_kit::test]
fn new_role_creates_a_nologin_role(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, fake, dialog) = open_new(cx, "app", "menu-new-role");
    assert!(
        cx.debug_bounds("field-password").is_none(),
        "no password for a role"
    );
    let name = dialog.read_with(&cx, |d, _| d.name.clone());
    type_into(&mut cx, name, "readers");
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[r#"CREATE ROLE "readers" NOLOGIN"#]),
            true
        )]
    );
}
