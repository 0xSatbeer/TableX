use gpui_kit::{Entity, VisualTestContext};

use super::super::{CAP, Console, PAGE};
use super::run::{cols, connect, console};
use crate::db::Opened;
use crate::db::fake::Fake;
use crate::model::{Row, Value};

/// `n` one-column rows numbered from `from`.
fn numbered(from: usize, n: usize) -> Vec<Row> {
    (from..from + n)
        .map(|i| vec![Value::Text(i.to_string().into())].into_boxed_slice())
        .collect()
}

/// Runs a select whose first fetch returns `first` rows.
fn run_select(
    cx: &mut gpui_kit::TestAppContext,
    first: usize,
) -> (std::sync::Arc<Fake>, VisualTestContext, Entity<Console>) {
    let (fake, mut cx, console, _) = console(cx, "select n from t");
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    fake.resolve_query("open app_db", Ok(Opened::Rows(cols("n"))));
    cx.run_until_parked();
    fake.resolve_query("fetch app_db", Ok(numbered(0, first)));
    cx.run_until_parked();
    (fake, cx, console)
}

fn label(console: &Entity<Console>, cx: &VisualTestContext) -> String {
    console.read_with(cx, |c, _| c.results[0].label())
}

/// The first cell of the page shown, or the dropped-rows notice.
fn shown(console: &Entity<Console>, cx: &VisualTestContext) -> String {
    console.read_with(cx, |c, cx| {
        let r = &c.results[0];
        r.notice().unwrap_or_else(|| {
            let grid = r.grid.read(cx);
            format!("{} rows", grid.row_count())
        })
    })
}

fn next(console: &Entity<Console>, cx: &mut VisualTestContext) {
    console.update(cx, |c, cx| c.next_page(0, cx));
    cx.run_until_parked();
}

fn prev(console: &Entity<Console>, cx: &mut VisualTestContext) {
    console.update(cx, |c, cx| c.prev_page(0, cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn next_fetches_page_by_page_until_the_result_is_exhausted(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console) = run_select(cx, PAGE);
    assert_eq!(label(&console, &cx), "1–500 of 500+");
    next(&console, &mut cx);
    assert_eq!(fake.pending(), ["fetch app_db"]);
    fake.resolve_query("fetch app_db", Ok(numbered(500, PAGE)));
    cx.run_until_parked();
    assert_eq!(label(&console, &cx), "501–1000 of 500+");
    next(&console, &mut cx);
    fake.resolve_query("fetch app_db", Ok(numbered(1000, 234)));
    cx.run_until_parked();
    assert_eq!(label(&console, &cx), "1001–1234 of 1234");
    assert_eq!(shown(&console, &cx), "234 rows");
    assert!(!console.read_with(&cx, |c, _| c.results[0].can_next()));
    next(&console, &mut cx);
    assert!(
        fake.pending().is_empty(),
        "an exhausted result fetches nothing"
    );
}

#[gpui_kit::test]
fn previous_shows_pages_from_memory_without_fetching(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console) = run_select(cx, PAGE);
    next(&console, &mut cx);
    fake.resolve_query("fetch app_db", Ok(numbered(500, 100)));
    cx.run_until_parked();
    prev(&console, &mut cx);
    assert!(fake.pending().is_empty());
    assert_eq!(label(&console, &cx), "1–500 of 600");
    next(&console, &mut cx);
    assert!(
        fake.pending().is_empty(),
        "a page already read is not fetched again"
    );
    assert_eq!(label(&console, &cx), "501–600 of 600");
}

#[gpui_kit::test]
fn past_the_cap_the_oldest_pages_are_dropped(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console) = run_select(cx, PAGE);
    let pages = CAP / PAGE + 1;
    for p in 1..pages {
        next(&console, &mut cx);
        fake.resolve_query("fetch app_db", Ok(numbered(p * PAGE, PAGE)));
        cx.run_until_parked();
    }
    for _ in 1..pages {
        prev(&console, &mut cx);
    }
    assert!(fake.pending().is_empty());
    assert_eq!(
        shown(&console, &cx),
        "Rows before 501 were dropped to save memory · Run again"
    );
    next(&console, &mut cx);
    assert_eq!(
        shown(&console, &cx),
        "500 rows",
        "the next page is still in memory"
    );
}

#[gpui_kit::test]
fn earlier_results_of_a_script_close_after_their_first_page(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "select 1 as a;\nselect 2 as b");
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    for name in ["a", "b"] {
        fake.resolve_query("open app_db", Ok(Opened::Rows(cols(name))));
        cx.run_until_parked();
        fake.resolve_query("fetch app_db", Ok(numbered(0, PAGE)));
        cx.run_until_parked();
    }
    let can_next = console.read_with(&cx, |c, _| {
        c.results.iter().map(|r| r.can_next()).collect::<Vec<_>>()
    });
    assert_eq!(can_next, [false, true], "only the last result stays open");
    console.update(&mut cx, |c, cx| c.next_page(0, cx));
    cx.run_until_parked();
    assert!(fake.pending().is_empty());
}

#[gpui_kit::test]
fn close_session_closes_the_open_result(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console) = run_select(cx, PAGE);
    console.update(&mut cx, |c, cx| c.close_session(cx));
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["close app_db"]);
}
