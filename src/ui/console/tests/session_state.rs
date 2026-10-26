//! The console's session and transaction rules, without an editor.

use super::super::session::ConsoleSession;
use crate::db::{Session, fake};

fn on(session: Option<Session>) -> Option<String> {
    match session? {
        Session::Fake(f) => Some(f.database.clone()),
        _ => None,
    }
}

#[test]
fn a_manual_run_begins_only_outside_a_transaction_and_auto_never_does() {
    let mut s = ConsoleSession::default();
    let run = s.start_run();
    assert_eq!((run.begin, run.autocommit), (false, true), "auto-commit");
    assert!(s.set_manual(true));
    s.ran_on(fake::session("app"));
    let run = s.start_run();
    assert_eq!(
        (run.begin, run.autocommit, run.session.is_some()),
        (true, false, true)
    );
    s.began();
    assert_eq!(s.mode(), "TX");
    assert!(!s.start_run().begin, "inside the transaction");
}

#[test]
fn a_session_dropped_with_a_transaction_open_rolled_it_back_and_begins_again() {
    let mut s = ConsoleSession::default();
    s.set_manual(true);
    let session = fake::session("app");
    s.ran_on(session.clone());
    s.began();
    fake::close(&session);
    let run = s.start_run();
    assert!(run.rolled_back && run.begin && run.session.is_none());
    assert_eq!(s.mode(), "MANUAL");
}

#[test]
fn leaving_manual_waits_for_the_open_transaction_to_end() {
    let mut s = ConsoleSession::default();
    s.set_manual(true);
    s.ran_on(fake::session("app"));
    s.began();
    assert!(!s.set_manual(false));
    assert_eq!(s.mode(), "TX");
    s.ended();
    assert!(s.set_manual(false));
    assert_eq!(s.mode(), "AUTO-COMMIT");
}

#[test]
fn listing_the_context_keeps_its_session_only_when_no_run_kept_one() {
    let mut s = ConsoleSession::default();
    s.ran_on(fake::session("run"));
    s.keep(fake::session("context"));
    assert_eq!(
        on(s.open()).as_deref(),
        Some("run"),
        "the run's session stays"
    );
    s.lost();
    s.keep(fake::session("context"));
    assert_eq!(on(s.open()).as_deref(), Some("context"));
}

#[test]
fn taking_the_session_says_whether_to_roll_back() {
    let mut s = ConsoleSession::default();
    assert!(s.take().is_none());
    s.ran_on(fake::session("app"));
    s.began();
    let (_, rollback) = s.take().expect("the session");
    assert!(rollback);
    assert!(!s.in_transaction && s.open().is_none());
}
