use gpui_kit::{Entity, VisualTestContext};

use super::super::Console;
use super::run::{connect, console};
use crate::db::Opened;
use crate::model::{QueryError, QueryErrorKind};

const SCRIPT: &str = "update t set x = 1;\nselect a,\n  nope from t";

fn fail(kind: QueryErrorKind, position: Option<usize>) -> Result<Opened, QueryError> {
    Err(QueryError {
        kind,
        message: "column \"nope\" does not exist".into(),
        position,
        statement: 0,
    })
}

/// Runs `SCRIPT`; its first statement succeeds and its second fails with `error`.
fn run_failing(
    cx: &mut gpui_kit::TestAppContext,
    error: Result<Opened, QueryError>,
) -> (
    std::sync::Arc<crate::db::fake::Fake>,
    VisualTestContext,
    Entity<Console>,
) {
    let (fake, mut cx, console, _) = console(cx, SCRIPT);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
    cx.run_until_parked();
    fake.resolve_query("open app_db", error);
    cx.run_until_parked();
    (fake, cx, console)
}

/// The editor's diagnostics as (byte range, message).
fn diagnostics(
    console: &Entity<Console>,
    cx: &mut VisualTestContext,
) -> Vec<(std::ops::Range<usize>, String)> {
    console.update(cx, |c, cx| {
        c.editor.update(cx, |e, _| {
            e.diagnostics_mut()
                .expect("a code editor")
                .iter()
                .map(|d| (d.range.clone(), d.message.to_string()))
                .collect()
        })
    })
}

#[gpui_kit::test]
fn a_server_error_position_reports_the_editor_line_and_col_and_marks_the_token(
    cx: &mut gpui_kit::TestAppContext,
) {
    // "nope" sits at byte 12 of "select a,\n  nope from t", which starts at byte 20 of the script.
    let (_, mut cx, console) = run_failing(cx, fail(QueryErrorKind::Server, Some(12)));
    assert_eq!(
        console.read_with(&cx, |c, _| c.messages.clone()),
        [
            "1 rows affected",
            "Error at line 3, col 3: column \"nope\" does not exist"
        ]
    );
    assert_eq!(
        diagnostics(&console, &mut cx),
        [(32..36, "column \"nope\" does not exist".to_string())]
    );

    console.update(&mut cx, |c, cx| {
        c.editor.update(cx, |e, cx| e.set_selected_range(0..0, cx))
    });
    cx.simulate_input("-- ");
    assert!(
        diagnostics(&console, &mut cx).is_empty(),
        "an edit clears it"
    );
}

#[gpui_kit::test]
fn a_server_error_without_a_position_shows_the_message_alone(cx: &mut gpui_kit::TestAppContext) {
    let (_, mut cx, console) = run_failing(cx, fail(QueryErrorKind::Server, None));
    assert_eq!(
        console.read_with(&cx, |c, _| c.messages.clone()),
        ["1 rows affected", "column \"nope\" does not exist"]
    );
    assert!(diagnostics(&console, &mut cx).is_empty());
}

#[gpui_kit::test]
fn a_new_run_clears_the_diagnostic(cx: &mut gpui_kit::TestAppContext) {
    let (_, mut cx, console) = run_failing(cx, fail(QueryErrorKind::Server, Some(12)));
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    assert!(diagnostics(&console, &mut cx).is_empty());
}

#[gpui_kit::test]
fn a_disconnected_error_makes_the_next_run_reconnect(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console) = run_failing(cx, fail(QueryErrorKind::Disconnected, None));
    assert_eq!(
        console.read_with(&cx, |c, _| c.messages.clone()),
        [
            "1 rows affected",
            "Connection lost · Run again to reconnect"
        ]
    );
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["connect app_db"]);
}
