//! Export: which results can export every row, and an All rows export cancelled mid-stream.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::AppContext as _;

use super::*;
use crate::db::fake::{self, Fake};
use crate::db::{Opened, Reply, Session};
use crate::export::{BATCH, Format};
use crate::model::{ConnectionId, ResultSet, Row};
use crate::ui::grid::{Rerun, Table};

fn result() -> ResultSet {
    ResultSet {
        columns: vec![col("n", "int4", false)],
        rows: vec![vec![Value::Int(1)].into()],
    }
}

fn rerun(driver: Arc<Fake>, sql: &str) -> Rerun {
    Rerun {
        driver,
        sql: sql.into(),
        connect: Rc::new(|| -> Reply<Session> {
            Box::pin(futures::future::ready(Ok(fake::session("app"))))
        }),
    }
}

#[gpui_kit::test]
fn all_rows_is_off_only_for_writing_statements_and_results_without_a_query(
    cx: &mut gpui_kit::TestAppContext,
) {
    let driver = Arc::new(Fake::default());
    // (console statement, or none for a result in memory; All rows off)
    let cases = [
        (None, true),
        (Some("SELECT * FROM t"), false),
        (Some("WITH x AS (SELECT 1) SELECT * FROM x"), false),
        (Some("DELETE FROM t RETURNING *"), true),
        (Some("INSERT INTO t VALUES (1) RETURNING *"), true),
    ];
    for (sql, off) in cases {
        let grid = cx.new(|cx| {
            let mut g = Grid::new(Rc::new(result()), cx);
            g.rerun = sql.map(|sql| rerun(driver.clone(), sql));
            g
        });
        let got = grid.read_with(cx, |g, _| g.all_rows_off().is_some());
        assert_eq!(got, off, "{sql:?}");
    }
    let workspace = cx.new(|_| crate::mock::workspaces().remove(0));
    let handle = gpui_kit::VisualContext::window_handle(cx.add_empty_window());
    let sessions =
        cx.new(|cx| crate::sessions::Sessions::new(workspace, driver.clone(), handle, cx));
    let table = Table {
        driver,
        connection: ConnectionId(1),
        database: "app".into(),
        sessions,
        schema: "public".into(),
        name: "users".into(),
        sqlite_table: false,
        view: false,
        dialect: crate::model::DbKind::Postgres,
    };
    let grid = cx.new(|cx| Grid::table(table, cx));
    assert_eq!(
        grid.read_with(cx, |g, _| g.all_rows_off()),
        None,
        "table tab"
    );
}

#[gpui_kit::test]
fn cancel_stops_the_exports_own_session_and_removes_the_partial_file(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let driver = Arc::new(Fake::default());
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| {
                let mut g = Grid::new(Rc::new(result()), cx);
                g.rerun = Some(rerun(driver.clone(), "SELECT n FROM t"));
                g
            })
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("result.csv");
    let to = path.clone();
    grid.update_in(&mut cx, |g, window, cx| {
        g.export_all_rows(Format::Csv, to, window, cx)
    });
    cx.run_until_parked();
    driver.resolve_query("open app", Ok(Opened::Rows(vec![col("n", "int4", false)])));
    cx.run_until_parked();
    let batch: Vec<Row> = (0..BATCH as i64)
        .map(|n| vec![Value::Int(n)].into())
        .collect();
    driver.resolve_query("fetch app", Ok(batch.clone()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("grid-export-progress").is_some());
    assert!(path.exists(), "the first batch is on disk");

    let cancel = cx.debug_bounds("grid-export-cancel").expect("Cancel drawn");
    cx.simulate_click(cancel.center(), Default::default());
    cx.run_until_parked();
    assert!(driver.pending().contains(&"cancel app".to_string()));
    // The fetch under way still lands; the export stops at the flag, then closes the result.
    driver.resolve_query("fetch app", Ok(batch));
    cx.run_until_parked();
    driver.resolve::<()>("close app", Ok(()));
    cx.run_until_parked();
    assert!(!path.exists());
    assert!(cx.debug_bounds("grid-export-progress").is_none());
    assert!(cx.debug_bounds("grid-export").is_some());
}
