use gpui_kit::{Entity, VisualTestContext};

use super::super::Console;
use super::run::{connect, console};
use crate::db::Opened;
use crate::db::fake::Fake;
use crate::model::{QueryError, QueryErrorKind};

fn manual(console: &Entity<Console>, cx: &mut VisualTestContext) {
    console.update_in(cx, |c, window, cx| c.set_manual(true, window, cx));
    cx.run_until_parked();
}

fn affected(fake: &Fake, cx: &mut VisualTestContext) {
    fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
    cx.run_until_parked();
}

fn mode(console: &Entity<Console>, cx: &VisualTestContext) -> &'static str {
    console.read_with(cx, |c, _| c.mode())
}

fn ran(sql: &[&str]) -> Vec<(String, bool)> {
    sql.iter().map(|s| (s.to_string(), false)).collect()
}

#[gpui_kit::test]
fn manual_mode_begins_once_runs_without_autocommit_and_commit_ends_the_transaction(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, console, _) = console(cx, "update t set a = 1;\nupdate t set b = 2");
    assert_eq!(mode(&console, &cx), "AUTO-COMMIT");
    manual(&console, &mut cx);
    assert_eq!(mode(&console, &cx), "MANUAL");
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    for _ in 0..3 {
        affected(&fake, &mut cx);
    }
    assert_eq!(
        fake.opened_with(),
        ran(&["BEGIN", "update t set a = 1", "update t set b = 2"])
    );
    assert_eq!(mode(&console, &cx), "TX");
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    affected(&fake, &mut cx);
    affected(&fake, &mut cx);
    assert_eq!(fake.opened_with().len(), 5, "no second BEGIN");
    console.update(&mut cx, |c, cx| c.end_transaction("COMMIT", cx));
    cx.run_until_parked();
    affected(&fake, &mut cx);
    assert_eq!(fake.opened_with().last(), ran(&["COMMIT"]).last());
    assert_eq!(mode(&console, &cx), "MANUAL");
    assert!(fake.pending().is_empty());
}

#[gpui_kit::test]
fn an_error_inside_a_manual_transaction_leaves_it_open(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "select x");
    manual(&console, &mut cx);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    affected(&fake, &mut cx);
    fake.resolve_query::<Opened>(
        "open app_db",
        Err(QueryError {
            kind: QueryErrorKind::Server,
            message: "column \"x\" does not exist".into(),
            position: None,
            statement: 0,
        }),
    );
    cx.run_until_parked();
    assert_eq!(mode(&console, &cx), "TX");
}

#[gpui_kit::test]
fn switching_to_auto_commit_with_an_open_transaction_asks_and_can_roll_back(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, console, _) = console(cx, "update t set a = 1");
    manual(&console, &mut cx);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    affected(&fake, &mut cx);
    affected(&fake, &mut cx);
    console.update_in(&mut cx, |c, window, cx| c.set_manual(false, window, cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("tx-switch-title").is_some(), "asks first");
    assert_eq!(mode(&console, &cx), "TX", "nothing changes until answered");
    let bounds = cx
        .debug_bounds("tx-switch-rollback")
        .expect("Roll back drawn");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    affected(&fake, &mut cx);
    assert_eq!(fake.opened_with().last(), ran(&["ROLLBACK"]).last());
    assert_eq!(mode(&console, &cx), "AUTO-COMMIT");
}

#[gpui_kit::test]
fn a_run_after_the_session_dropped_says_the_transaction_rolled_back_and_begins_again(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, console, _) = console(cx, "update t set a = 1");
    manual(&console, &mut cx);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    affected(&fake, &mut cx);
    affected(&fake, &mut cx);
    let session = console.read_with(&cx, |c, _| c.session.open().expect("a session"));
    crate::db::fake::close(&session);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    assert_eq!(mode(&console, &cx), "MANUAL", "the badge drops TX at once");
    connect(&fake, &mut cx);
    affected(&fake, &mut cx);
    let opened = fake.opened_with();
    assert_eq!(
        opened[2..],
        ran(&["BEGIN", "update t set a = 1"])[..],
        "begins again on the new session"
    );
    assert_eq!(mode(&console, &cx), "TX");
    let messages = console.read_with(&cx, |c, _| c.messages.clone());
    assert_eq!(
        messages,
        ["Connection lost · the open transaction was rolled back"]
    );
}
