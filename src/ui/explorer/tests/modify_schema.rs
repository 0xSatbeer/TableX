//! Modify… on a Postgres schema: Apply runs atomically on its database's session.

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

#[gpui_kit::test]
fn rename_and_owner_change_run_atomically_on_the_database_session_and_refresh_the_database(
    cx: &mut gpui_kit::TestAppContext,
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
        cx.observe_new(move |_: &mut NameOwnerDialog, _, cx| *sink.borrow_mut() = Some(cx.entity()))
    });
    let ix = labels(&explorer, &cx)
        .iter()
        .position(|l| l == "public")
        .expect("public row");
    let at = cx
        .debug_bounds(format!("tree-label-{ix}").leak())
        .expect("public drawn")
        .center();
    cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
    cx.run_until_parked();
    click(&mut cx, "menu-modify");
    let dialog: gpui_kit::Entity<NameOwnerDialog> =
        opened.take().expect("Modify… opened the dialog");
    fake.resolve("roles app", Ok(vec![role("alice"), role("bob")]));
    fake.resolve("schema_owner app/public", Ok("alice".to_string()));
    cx.run_until_parked();

    let name = dialog.read_with(&cx, |d, _| d.name.clone());
    cx.update(|window, cx| name.update(cx, |input, cx| input.focus(window, cx)));
    cx.simulate_keystrokes("ctrl-a backspace");
    cx.simulate_input("core");
    let owner = dialog.read_with(&cx, |d, _| d.owner.clone().expect("owner loaded"));
    owner.update_in(&mut cx, |s, window, cx| {
        s.set_selected_value(&"bob".to_string(), window, cx)
    });
    cx.run_until_parked();
    click(&mut cx, "object-apply");
    assert_eq!(
        fake.executed(),
        [(
            "app".to_string(),
            strings(&[
                r#"ALTER SCHEMA "public" RENAME TO "core""#,
                r#"ALTER SCHEMA "core" OWNER TO "bob""#,
            ]),
            true
        )]
    );
    fake.resolve_query("execute app", Ok(()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("dialog-layer").is_none(), "dialog closed");
    assert_eq!(
        fake.pending(),
        ["schemas app"],
        "the database re-reads its schemas"
    );
}
