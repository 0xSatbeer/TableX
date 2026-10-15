use gpui_kit::{Entity, VisualTestContext};

use super::super::{Console, ConsoleEvent};
use super::run::{connect, console};
use crate::db::Opened;
use crate::db::fake::{Fake, session};

fn moved(
    console: &Entity<Console>,
    cx: &mut VisualTestContext,
) -> std::rc::Rc<std::cell::Cell<usize>> {
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let sink = count.clone();
    cx.update(|_, cx| {
        cx.subscribe(console, move |_, ev: &ConsoleEvent, _| {
            if let ConsoleEvent::Moved = ev {
                sink.set(sink.get() + 1)
            }
        })
        .detach()
    });
    count
}

/// Lets the context lists load on a fresh `database` session.
fn lists(fake: &Fake, cx: &mut VisualTestContext, database: &str) {
    fake.resolve(&format!("connect {database}"), Ok(session(database)));
    cx.run_until_parked();
    fake.resolve(
        &format!("databases {database}"),
        Ok(vec!["app_db".to_string(), "other_db".into()]),
    );
    fake.resolve(
        &format!("schemas {database}"),
        Ok(vec!["public".to_string(), "sales".into()]),
    );
    cx.run_until_parked();
}

#[gpui_kit::test]
fn picking_a_schema_sets_the_search_path_and_moves_the_console(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "select 1");
    let moves = moved(&console, &mut cx);
    console.update(&mut cx, |c, cx| c.load_context(cx));
    cx.run_until_parked();
    lists(&fake, &mut cx, "app_db");
    assert_eq!(
        console.read_with(&cx, |c, _| c.schemas.clone()),
        ["public", "sales"]
    );
    console.update(&mut cx, |c, cx| c.pick_schema("sales".into(), cx));
    cx.run_until_parked();
    fake.resolve_query("open app_db", Ok(Opened::Affected(0)));
    cx.run_until_parked();
    assert_eq!(fake.opened(), ["SET search_path TO \"sales\""]);
    assert_eq!(console.read_with(&cx, |c, _| c.tail()), "app_db › sales");
    assert_eq!(moves.get(), 1);
}

#[gpui_kit::test]
fn picking_a_database_reconnects_on_it(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "select 1");
    let moves = moved(&console, &mut cx);
    console.update(&mut cx, |c, cx| c.load_context(cx));
    cx.run_until_parked();
    lists(&fake, &mut cx, "app_db");
    console.update_in(&mut cx, |c, window, cx| {
        c.pick_database("other_db".into(), window, cx)
    });
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["close app_db", "connect other_db"]);
    fake.resolve("close app_db", Ok(()));
    lists(&fake, &mut cx, "other_db");
    assert_eq!(console.read_with(&cx, |c, _| c.tail()), "other_db");
    assert_eq!(moves.get(), 1);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["open other_db"], "runs on the new session");
}

#[gpui_kit::test]
fn switching_database_with_an_open_transaction_asks_first(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "update t set a = 1");
    console.update_in(&mut cx, |c, window, cx| c.set_manual(true, window, cx));
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    for _ in 0..2 {
        fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
        cx.run_until_parked();
    }
    console.update_in(&mut cx, |c, window, cx| {
        c.pick_database("other_db".into(), window, cx)
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("db-switch-title").is_some(), "asks first");
    assert!(fake.pending().is_empty(), "nothing happens until answered");
    let bounds = cx.debug_bounds("db-switch-confirm").expect("confirm drawn");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(fake.opened().last().map(String::as_str), Some("ROLLBACK"));
    fake.resolve_query("open app_db", Ok(Opened::Affected(0)));
    cx.run_until_parked();
    assert!(fake.pending().contains(&"connect other_db".to_string()));
    assert_eq!(console.read_with(&cx, |c, _| c.mode()), "MANUAL");
}

#[gpui_kit::test]
fn the_schema_cannot_change_inside_an_open_transaction(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "update t set a = 1");
    console.update_in(&mut cx, |c, window, cx| c.set_manual(true, window, cx));
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    for _ in 0..2 {
        fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
        cx.run_until_parked();
    }
    console.update(&mut cx, |c, cx| c.pick_schema("sales".into(), cx));
    cx.run_until_parked();
    assert!(fake.pending().is_empty(), "no SET a rollback could undo");
    assert_eq!(console.read_with(&cx, |c, _| c.tail()), "app_db");
}
