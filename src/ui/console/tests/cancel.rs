use super::run::{connect, console};
use crate::db::Opened;
use crate::model::{QueryError, QueryErrorKind, QueryStatus};

#[gpui_kit::test]
fn esc_during_a_run_cancels_it_stops_the_script_and_logs_cancelled(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, console, logged) = console(cx, "select pg_sleep(9);\nselect 2");
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["open app_db", "cancel app_db"]);
    fake.resolve("cancel app_db", Ok(()));
    fake.resolve_query::<Opened>(
        "open app_db",
        Err(QueryError {
            kind: QueryErrorKind::Cancelled,
            message: "canceling statement due to user request".into(),
            position: None,
            statement: 0,
        }),
    );
    cx.run_until_parked();
    assert_eq!(fake.opened(), ["select pg_sleep(9)"], "the script stops");
    assert!(fake.pending().is_empty());
    let messages = console.read_with(&cx, |c, _| c.messages.clone());
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].starts_with("Cancelled after ") && messages[0].ends_with(" ms"),
        "{messages:?}"
    );
    let last = logged.borrow().last().cloned().expect("logged");
    assert_eq!(last.status, QueryStatus::Cancelled);
    assert!(!console.read_with(&cx, |c, _| c.running));
}

#[gpui_kit::test]
fn esc_while_connecting_says_cancelled_not_a_connection_error(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "select 1");
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["connect app_db"]);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let (messages, error) = console.read_with(&cx, |c, _| (c.messages.clone(), c.error.clone()));
    assert_eq!(messages, ["Cancelled"]);
    assert_eq!(error, None, "the Results tab shows no error");
}

#[gpui_kit::test]
fn a_failed_connect_tells_the_results_tab_to_check_the_connection(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, console, _) = console(cx, "select 1");
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    fake.resolve::<crate::db::Session>("connect app_db", Err("connection refused".into()));
    cx.run_until_parked();
    let error = console.read_with(&cx, |c, _| c.error.clone());
    assert_eq!(
        error.as_deref(),
        Some("connection refused · Check the connection and Run again")
    );
}
