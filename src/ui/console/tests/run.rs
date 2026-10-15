use std::sync::Arc;

use gpui_kit::{AppContext as _, Entity, VisualTestContext};

use super::super::{Console, ConsoleEvent, Target};
use crate::db::Opened;
use crate::db::fake::{Fake, session};
use crate::model::{
    ColumnMeta, QueryError, QueryErrorKind, QueryLogEntry, QuerySource, QueryStatus, Row, Value,
};

/// A console on the mock workspace's first connection (app_db), made non-production, holding `text`, its editor focused,
/// and the log entries it emits.
pub(super) fn console(
    cx: &mut gpui_kit::TestAppContext,
    text: &str,
) -> (
    Arc<Fake>,
    VisualTestContext,
    Entity<Console>,
    std::rc::Rc<std::cell::RefCell<Vec<QueryLogEntry>>>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::bind_keys(cx);
    });
    let fake = Arc::new(Fake::default());
    let driver: Arc<dyn crate::db::Driver> = fake.clone();
    let text = text.to_string();
    let (window, console) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            let mut ws = crate::mock::workspaces().remove(0);
            ws.connections[0].is_production = false;
            let workspace = cx.new(|_| ws);
            let connection = Some(workspace.read(cx).connections[0].id);
            let target = Target {
                workspace,
                driver,
                connection,
                database: "app_db".into(),
            };
            cx.new(|cx| Console::new(target, &text, window, cx))
        })
        .expect("open test window")
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    let logged = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = logged.clone();
    vcx.update(|window, cx| {
        cx.subscribe(&console, move |_, ev: &ConsoleEvent, _| {
            if let ConsoleEvent::Logged(entry) = ev {
                sink.borrow_mut().push(entry.clone())
            }
        })
        .detach();
        let editor = console.read(cx).editor.clone();
        editor.update(cx, |e, cx| e.focus(window, cx));
    });
    vcx.run_until_parked();
    (fake, vcx, console, logged)
}

pub(super) fn cols(name: &str) -> Vec<ColumnMeta> {
    vec![ColumnMeta {
        name: name.into(),
        data_type: "int4".into(),
        ..Default::default()
    }]
}

fn rows(values: &[&str]) -> Vec<Row> {
    values
        .iter()
        .map(|v| vec![Value::Text((*v).into())].into_boxed_slice())
        .collect()
}

/// Answers the pending `open` with `rows` of column `name`, then its `fetch`.
fn answer_rows(fake: &Fake, cx: &mut VisualTestContext, name: &str, values: &[&str]) {
    fake.resolve_query("open app_db", Ok(Opened::Rows(cols(name))));
    cx.run_until_parked();
    fake.resolve_query("fetch app_db", Ok(rows(values)));
    cx.run_until_parked();
}

pub(super) fn connect(fake: &Fake, cx: &mut VisualTestContext) {
    fake.resolve("connect app_db", Ok(session("app_db")));
    cx.run_until_parked();
}

/// Each result tab's (first column name, row count).
fn results(console: &Entity<Console>, cx: &VisualTestContext) -> Vec<(String, usize)> {
    console.read_with(cx, |c, cx| {
        c.results
            .iter()
            .map(|g| {
                let g = g.grid.read(cx);
                (g.columns()[0].name.to_string(), g.row_count())
            })
            .collect()
    })
}

fn messages(console: &Entity<Console>, cx: &VisualTestContext) -> Vec<String> {
    console.read_with(cx, |c, _| c.messages.clone())
}

const SCRIPT: &str = "select 1 as a;\nselect 2 as b;\nselect 3 as c";

#[gpui_kit::test]
fn ctrl_enter_runs_the_statement_under_the_cursor_and_f5_reruns_it(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, console, _) = console(cx, SCRIPT);
    console.update(&mut cx, |c, cx| {
        c.editor
            .update(cx, |e, cx| e.set_selected_range(18..18, cx))
    });
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["connect app_db"]);
    connect(&fake, &mut cx);
    assert_eq!(fake.opened(), ["select 2 as b"]);
    answer_rows(&fake, &mut cx, "b", &["2"]);
    assert_eq!(results(&console, &cx), [("b".to_string(), 1)]);

    cx.simulate_keystrokes("f5");
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["open app_db"], "the session is reused");
    assert_eq!(fake.opened(), ["select 2 as b", "select 2 as b"]);
}

#[gpui_kit::test]
fn ctrl_enter_runs_the_selection_when_there_is_one(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, SCRIPT);
    console.update(&mut cx, |c, cx| {
        c.editor.update(cx, |e, cx| e.set_selected_range(0..8, cx))
    });
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    assert_eq!(fake.opened(), ["select 1"]);
}

#[gpui_kit::test]
fn ctrl_shift_enter_runs_the_script_into_result_tabs_and_affected_messages(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, console, logged) =
        console(cx, "select 1 as a;\nupdate t set x = 1;\nselect 2 as b");
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    answer_rows(&fake, &mut cx, "a", &["1", "11"]);
    fake.resolve_query("open app_db", Ok(Opened::Affected(3)));
    cx.run_until_parked();
    answer_rows(&fake, &mut cx, "b", &["2"]);
    assert_eq!(
        fake.opened(),
        ["select 1 as a", "update t set x = 1", "select 2 as b"]
    );
    assert_eq!(
        results(&console, &cx),
        [("a".to_string(), 2), ("b".to_string(), 1)]
    );
    assert_eq!(messages(&console, &cx), ["3 rows affected"]);
    let logged: Vec<_> = logged
        .borrow()
        .iter()
        .map(|e| (e.sql.clone(), e.rows, e.status.clone(), e.source))
        .collect();
    assert_eq!(
        logged,
        [
            (
                "select 1 as a".into(),
                Some(2),
                QueryStatus::Ok,
                QuerySource::Console
            ),
            (
                "update t set x = 1".into(),
                Some(3),
                QueryStatus::Ok,
                QuerySource::Console
            ),
            (
                "select 2 as b".into(),
                Some(1),
                QueryStatus::Ok,
                QuerySource::Console
            ),
        ]
    );

    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    assert!(
        results(&console, &cx).is_empty() && messages(&console, &cx).is_empty(),
        "a new run replaces the previous results"
    );
}

#[gpui_kit::test]
fn a_failing_statement_stops_the_script_and_shows_its_error(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, logged) = console(cx, SCRIPT);
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    answer_rows(&fake, &mut cx, "a", &["1"]);
    fake.resolve_query::<Opened>(
        "open app_db",
        Err(QueryError {
            kind: QueryErrorKind::Server,
            message: "column \"b\" does not exist".into(),
            position: None,
            statement: 0,
        }),
    );
    cx.run_until_parked();
    assert_eq!(
        fake.opened(),
        ["select 1 as a", "select 2 as b"],
        "the third never runs"
    );
    assert!(fake.pending().is_empty());
    assert_eq!(results(&console, &cx), [("a".to_string(), 1)]);
    assert_eq!(messages(&console, &cx), ["column \"b\" does not exist"]);
    let last = logged.borrow().last().cloned().expect("logged");
    assert_eq!(
        (last.sql, last.rows, last.status, last.source),
        (
            "select 2 as b".to_string(),
            None,
            QueryStatus::Err("column \"b\" does not exist".into()),
            QuerySource::Console
        )
    );
}
