//! Passwords from the keychain or the prompt on connect, and the keychain entry on delete.

use super::*;
use crate::model::{ConnectionId, ConnectionParams, Secret};
use crate::persist::{Keychain as _, Keys, MemKeychain};
use crate::ui::password_prompt::PasswordPrompt;

/// Connection 1 with no password in memory, its "Save password" box as `save_password`.
fn unsaved(save_password: bool) -> crate::model::Connection {
    let mut conn = pg(1, "pg", "app");
    conn.params = pg_params(None, "app");
    conn.save_password = save_password;
    conn
}

fn install(cx: &mut gpui_kit::VisualTestContext, entries: &[(u32, &str)]) -> Arc<MemKeychain> {
    let keychain = Arc::new(MemKeychain::default());
    for (id, password) in entries {
        keychain
            .set(ConnectionId(*id), &Secret::new(*password))
            .unwrap();
    }
    let installed: Arc<dyn crate::persist::Keychain> = keychain.clone();
    cx.update(|_, cx| cx.set_global(Keys::new(installed)));
    keychain
}

fn password_of(
    workspace: &gpui_kit::Entity<crate::model::Workspace>,
    cx: &gpui_kit::VisualTestContext,
) -> Option<String> {
    workspace.read_with(cx, |ws, _| match &ws.connections[0].params {
        ConnectionParams::Postgres { password, .. } => password.as_ref().map(|p| p.expose().into()),
        ConnectionParams::Sqlite { .. } => None,
    })
}

/// Expands connection 0 and returns the prompt it opened, if any.
fn expand_watching_prompt(
    explorer: &gpui_kit::Entity<Explorer>,
    cx: &mut gpui_kit::VisualTestContext,
) -> Option<gpui_kit::Entity<PasswordPrompt>> {
    use std::{cell::RefCell, rc::Rc};
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(move |_: &mut PasswordPrompt, _, cx| *sink.borrow_mut() = Some(cx.entity()))
    });
    expand_connection(explorer, cx);
    cx.run_until_parked();
    opened.take()
}

#[gpui_kit::test]
fn a_saved_password_comes_from_the_keychain_without_a_prompt(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![unsaved(true)]);
    install(&mut cx, &[(1, "kp")]);
    let prompt = expand_watching_prompt(&explorer, &mut cx);
    assert!(prompt.is_none(), "no prompt");
    assert_eq!(fake.pending(), ["connect app"]);
    assert_eq!(password_of(&workspace, &cx).as_deref(), Some("kp"));
}

#[gpui_kit::test]
fn with_no_stored_password_the_prompt_asks_and_connects_with_the_typed_one(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (save box, keychain entries)
    let cases: [(bool, &[(u32, &str)]); 3] = [(true, &[]), (false, &[]), (false, &[(1, "kp")])];
    for (save, entries) in cases {
        let label = format!("save {save} entries {entries:?}");
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![unsaved(save)]);
        install(&mut cx, entries);
        let prompt = expand_watching_prompt(&explorer, &mut cx).expect("a prompt");
        assert!(fake.pending().is_empty(), "{label}: waits for the password");
        assert_eq!(status_of(&workspace, &cx), ConnectionStatus::Disconnected);
        let field = prompt.read_with(&cx, |p, _| p.password.clone());
        cx.update(|window, cx| field.update(cx, |f, cx| f.focus(window, cx)));
        cx.simulate_input("typed");
        let bounds = cx.debug_bounds("prompt-connect").expect("Connect drawn");
        cx.simulate_click(bounds.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(fake.pending(), ["connect app"], "{label}");
        assert_eq!(status_of(&workspace, &cx), ConnectionStatus::Connecting);
        assert_eq!(
            password_of(&workspace, &cx).as_deref(),
            Some("typed"),
            "{label}"
        );
    }
}

#[gpui_kit::test]
fn cancel_on_the_prompt_leaves_the_connection_disconnected(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![unsaved(false)]);
    install(&mut cx, &[]);
    expand_watching_prompt(&explorer, &mut cx).expect("a prompt");
    let bounds = cx.debug_bounds("prompt-cancel").expect("Cancel drawn");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert!(fake.pending().is_empty());
    assert_eq!(status_of(&workspace, &cx), ConnectionStatus::Disconnected);
    assert_eq!(password_of(&workspace, &cx), None);
}

#[gpui_kit::test]
fn deleting_a_connection_removes_its_keychain_entry(cx: &mut gpui_kit::TestAppContext) {
    let other = crate::model::Connection {
        save_password: true,
        ..pg(2, "other", "app")
    };
    let (mut cx, explorer, _workspace, _fake) = open_explorer(cx, vec![unsaved(true), other]);
    let keychain = install(&mut cx, &[(1, "kp"), (2, "kp2")]);
    explorer.update(&mut cx, |e, cx| e.remove_connection(ConnectionId(1), cx));
    assert_eq!(keychain.get(ConnectionId(1)), Ok(None));
    assert_eq!(keychain.get(ConnectionId(2)), Ok(Some(Secret::new("kp2"))));
}

struct Broken;

impl crate::persist::Keychain for Broken {
    fn get(&self, _: ConnectionId) -> Result<Option<Secret>, String> {
        Err("no secret service".into())
    }
    fn set(&self, _: ConnectionId, _: &Secret) -> Result<(), String> {
        Err("no secret service".into())
    }
    fn delete(&self, _: ConnectionId) -> Result<(), String> {
        Err("no secret service".into())
    }
}

#[gpui_kit::test]
fn a_keychain_error_falls_back_to_the_prompt(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, explorer, _workspace, fake) = open_explorer(cx, vec![unsaved(true)]);
    cx.update(|_, cx| cx.set_global(Keys::new(Arc::new(Broken))));
    assert!(expand_watching_prompt(&explorer, &mut cx).is_some());
    assert!(fake.pending().is_empty());
    let again = cx.update(|_, cx| Keys::first_error(cx, "again"));
    assert_eq!(again, None, "the error was already shown once");
}
