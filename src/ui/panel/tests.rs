use std::time::Duration;

use super::filter::{LogFilter, StatusKind, empty_message, filter_rows};
use super::row::{format_duration, outcome, stamp};
use super::sql::sql_spans;
use crate::model::{ConnectionId, QueryLogEntry, QuerySource, QueryStatus};

mod filter;
mod rows;

fn entry(conn: u32, sql: &str, status: QueryStatus, source: QuerySource) -> QueryLogEntry {
    QueryLogEntry {
        at: std::time::UNIX_EPOCH,
        connection_id: ConnectionId(conn),
        database: "app_db".into(),
        sql: sql.into(),
        duration: Duration::from_millis(1),
        rows: None,
        status,
        source,
    }
}

#[test]
fn log_keeps_the_newest_10k_entries() {
    for (pushed, len, oldest) in [
        (0u32, 0, None),
        (1, 1, Some(0)),
        (9_999, 9_999, Some(0)),
        (10_000, 10_000, Some(0)),
        (10_001, 10_000, Some(1)),
        (10_005, 10_000, Some(5)),
    ] {
        let mut log = std::collections::VecDeque::new();
        for i in 0..pushed as usize {
            super::push_log(
                &mut log,
                entry(1, &format!("q{i}"), QueryStatus::Ok, QuerySource::Console),
            );
        }
        assert_eq!(log.len(), len, "length after {pushed} pushes");
        assert_eq!(
            log.front().map(|e| e.sql.clone()),
            oldest.map(|i| format!("q{i}")),
            "oldest after {pushed} pushes"
        );
        assert_eq!(
            log.back().map(|e| e.sql.clone()),
            (pushed as usize).checked_sub(1).map(|i| format!("q{i}")),
            "newest after {pushed} pushes"
        );
    }
}

#[gpui_kit::test]
fn app_launches_on_results(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let workspace = crate::mock::workspaces().remove(0);
    let (_, panel) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| {
                super::BottomPanel::new(
                    cx.new(|_| workspace),
                    crate::mock::mock_query_log(super::LOG_CAP),
                    window,
                    cx,
                )
            })
        })
        .expect("open test window")
    });
    assert_eq!(
        panel.read_with(cx, |p, _| p.tab),
        super::RESULTS,
        "active panel tab at launch"
    );
}

/// 0 c1 SELECT ok | 1 c2 SELECT ok | 2 c1 delete ok | 3 c3 select err | 4 c1 Select cancelled | 5 c2 UPDATE err
fn sample() -> Vec<QueryLogEntry> {
    let (ok, err, c) = (
        QueryStatus::Ok,
        || QueryStatus::Err("boom".into()),
        QuerySource::Console,
    );
    vec![
        entry(1, "SELECT 1", ok.clone(), c),
        entry(2, "SELECT 2", ok.clone(), c),
        entry(1, "delete from t", ok, c),
        entry(3, "select 3", err(), c),
        entry(1, "Select 4", QueryStatus::Cancelled, c),
        entry(2, "UPDATE t SET a = 1", err(), c),
    ]
}
