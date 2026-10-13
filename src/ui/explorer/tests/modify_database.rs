//! Modify… on a Postgres database: the dialog's fields, Apply and its failure, production confirmation.

use super::*;
use crate::model::Role;
use crate::ui::object_dialog::NameOwnerDialog;

fn role(name: &str) -> Role {
    Role {
        name: name.into(),
        can_login: true,
    }
}

/// Connected `pg` (production or not) listing databases app and shop, with Modify… picked on shop
/// and its roles (alice, bob) and owner (alice) answered.
fn open_modify(
    cx: &mut gpui_kit::TestAppContext,
    production: bool,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<super::Explorer>,
    Arc<crate::db::fake::Fake>,
    gpui_kit::Entity<NameOwnerDialog>,
) {
    use std::{cell::RefCell, rc::Rc};
    let conn = crate::model::Connection {
        is_production: production,
        ..pg(1, "pg", "app")
    };
    let (mut cx, explorer, _, fake) = open_explorer(cx, vec![conn]);
    connect_answered(&explorer, &fake, &mut cx, &["app", "shop"]);
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(move |_: &mut NameOwnerDialog, _, cx| *sink.borrow_mut() = Some(cx.entity()))
    });
    let ix = labels(&explorer, &cx)
        .iter()
        .position(|l| l == "shop")
        .expect("shop row");
    let at = cx
        .debug_bounds(format!("tree-label-{ix}").leak())
        .expect("shop drawn")
        .center();
    cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
    cx.run_until_parked();
    click(&mut cx, "menu-modify");
    let dialog = opened.take().expect("Modify… opened the dialog");
    answer(&fake, "connect shop", Ok(()));
    cx.run_until_parked();
    assert!(
        dialog.read_with(&cx, |d, _| d.owner.is_none()),
        "loading until roles and owner arrive"
    );
    fake.resolve("roles shop", Ok(vec![role("alice"), role("bob")]));
    cx.run_until_parked();
    assert!(
        dialog.read_with(&cx, |d, _| d.owner.is_none()),
        "still waiting for the owner"
    );
    fake.resolve("database_owner shop", Ok("alice".to_string()));
    cx.run_until_parked();
    (cx, explorer, fake, dialog)
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

/// Renames shop to store and hands it to bob.
fn rename_and_reown(
    cx: &mut gpui_kit::VisualTestContext,
    dialog: &gpui_kit::Entity<NameOwnerDialog>,
) {
    let name = dialog.read_with(cx, |d, _| d.name.clone());
    type_into(cx, name, "store");
    let owner = dialog.read_with(cx, |d, _| d.owner.clone().expect("owner loaded"));
    owner.update_in(cx, |s, window, cx| {
        s.set_selected_value(&"bob".to_string(), window, cx)
    });
    cx.run_until_parked();
}

fn statements() -> Vec<String> {
    strings(&[
        r#"ALTER DATABASE "shop" RENAME TO "store""#,
        r#"ALTER DATABASE "store" OWNER TO "bob""#,
    ])
}

#[gpui_kit::test]
fn the_dialog_shows_the_current_name_and_owner_among_the_roles(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, _, _, dialog) = open_modify(cx, false);
    let (name, roles, owner) = dialog.read_with(&cx, |d, cx| {
        let owner = d
            .owner
            .as_ref()
            .expect("owner loaded")
            .read(cx)
            .selected_value()
            .cloned();
        (d.name.read(cx).value().to_string(), d.roles.clone(), owner)
    });
    assert_eq!(name, "shop");
    assert_eq!(roles, strings(&["alice", "bob"]));
    assert_eq!(owner.as_deref(), Some("alice"));
    assert!(cx.debug_bounds("object-sql").is_some(), "SQL preview drawn");
    assert!(
        cx.debug_bounds("object-confirm").is_none(),
        "no confirmation off production"
    );
}

#[gpui_kit::test]
fn apply_runs_both_statements_one_by_one_on_a_postgres_session_and_refreshes_the_connection(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, _, fake, dialog) = open_modify(cx, false);
    rename_and_reown(&mut cx, &dialog);
    click(&mut cx, "object-apply");
    answer(&fake, "connect postgres", Ok(()));
    cx.run_until_parked();
    assert_eq!(
        fake.executed(),
        [("postgres".to_string(), statements(), false)]
    );
    fake.resolve("execute postgres", Ok(()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("dialog-layer").is_none(), "dialog closed");
    assert_eq!(
        fake.pending(),
        ["databases app"],
        "the connection re-reads its databases"
    );
}

#[gpui_kit::test]
fn a_failed_apply_shows_the_error_and_keeps_the_dialog_open(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, _, fake, dialog) = open_modify(cx, false);
    rename_and_reown(&mut cx, &dialog);
    click(&mut cx, "object-apply");
    answer(&fake, "connect postgres", Ok(()));
    cx.run_until_parked();
    fake.resolve::<()>("execute postgres", Err("permission denied".into()));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("dialog-layer").is_some(),
        "dialog kept open"
    );
    assert!(cx.debug_bounds("object-error").is_some(), "error shown");
    assert_eq!(
        dialog.read_with(&cx, |d, _| d.error.clone()).as_deref(),
        Some("permission denied")
    );
}

#[gpui_kit::test]
fn on_production_apply_stays_disabled_until_the_database_name_is_typed(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, _, fake, dialog) = open_modify(cx, true);
    rename_and_reown(&mut cx, &dialog);
    assert!(
        cx.debug_bounds("object-confirm").is_some(),
        "confirmation shown"
    );
    let confirm = dialog.read_with(&cx, |d, _| d.confirm.typed.clone());
    for typed in ["sho", "Shop", "store"] {
        type_into(&mut cx, confirm.clone(), typed);
        click(&mut cx, "object-apply");
        assert!(fake.pending().is_empty(), "{typed:?}: nothing run");
    }
    type_into(&mut cx, confirm, "shop");
    click(&mut cx, "object-apply");
    assert_eq!(fake.pending(), ["connect postgres"]);
}
