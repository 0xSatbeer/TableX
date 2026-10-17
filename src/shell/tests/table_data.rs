use super::support::*;
use crate::db::fake::{self, Fake};
use crate::model::{Column, ColumnMeta, QueryError, QueryErrorKind, QuerySource, ResultSet, Value};
use std::sync::Arc;

const PAGE_1: &str = r#"SELECT * FROM "public"."users" ORDER BY "id" LIMIT 500 OFFSET 0"#;

fn column(name: &str, is_pk: bool) -> Column {
    Column {
        name: name.into(),
        data_type: "int4".into(),
        nullable: !is_pk,
        default: None,
        is_pk,
        enum_values: Vec::new(),
        identity: false,
    }
}

fn rows(n: usize) -> ResultSet {
    ResultSet {
        columns: vec![ColumnMeta {
            name: "id".into(),
            data_type: "int4".into(),
            ..Default::default()
        }],
        rows: (0..n).map(|i| vec![Value::Int(i as i64)].into()).collect(),
    }
}

/// A Normal-mode shell on the sample workspace, with `users` opened from the tree and its
/// connect and columns calls answered, so the page-1 query is pending.
fn open_users(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    Arc<Fake>,
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<crate::shell::Shell>,
) {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let fake = Arc::new(Fake::default());
    let driver = fake.clone();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| {
                crate::shell::Shell::with_driver(
                    crate::shell::Mode::Normal,
                    crate::mock::workspaces().remove(0),
                    driver,
                    window,
                    cx,
                )
            })
        })
        .expect("open test window")
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    let explorer = shell.read_with(&vcx, |this, _| this.explorer.clone());
    explorer.update_in(&mut vcx, |e, window, cx| {
        e.reveal("1/app_db/public/table/users", window, cx)
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    vcx.run_until_parked();
    fake.resolve(
        "columns app_db/public/users",
        Ok(vec![column("id", true), column("email", false)]),
    );
    vcx.run_until_parked();
    (fake, window, shell)
}

fn grid_rows(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cx: &gpui_kit::VisualTestContext,
) -> usize {
    shell.read_with(cx, |this, cx| {
        match &this.tabs.active().expect("a tab").content {
            crate::shell::TabContent::Grid(grid) => grid.read(cx).row_count(),
            crate::shell::TabContent::Console(_) => panic!("a grid tab is active"),
        }
    })
}

fn answer(
    fake: &Fake,
    result: Result<ResultSet, QueryError>,
    cx: &mut gpui_kit::VisualTestContext,
) {
    fake.resolve_query("query app_db", result);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn opening_a_table_reads_page_one_in_key_order_and_logs_it(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    assert_eq!(fake.queried(), [PAGE_1]);
    assert!(
        cx.debug_bounds("table-loading").is_some(),
        "loading while the page is read"
    );
    answer(&fake, Ok(rows(500)), &mut cx);
    assert_eq!(grid_rows(&shell, &cx), 500);
    assert!(cx.debug_bounds("table-loading").is_none());
    assert!(
        cx.debug_bounds("grid-count:500+").is_some(),
        "a full page: more may follow"
    );
    let logged = shell.read_with(&cx, |this, cx| {
        let entry = this
            .bottom_panel
            .read(cx)
            .logged()
            .last()
            .cloned()
            .expect("logged");
        (entry.sql, entry.source, entry.rows)
    });
    assert_eq!(
        logged,
        (PAGE_1.to_string(), QuerySource::TableView, Some(500))
    );
}

#[gpui_kit::test]
fn next_reads_the_following_page_and_stops_after_a_short_one(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(rows(500)), &mut cx);
    click(&mut cx, "grid-next");
    assert_eq!(
        fake.queried().last().map(String::as_str),
        Some(r#"SELECT * FROM "public"."users" ORDER BY "id" LIMIT 500 OFFSET 500"#)
    );
    answer(&fake, Ok(rows(3)), &mut cx);
    assert_eq!(grid_rows(&shell, &cx), 3);
    assert!(
        cx.debug_bounds("grid-count:503").is_some(),
        "a short page ends the table"
    );
    click(&mut cx, "grid-next");
    assert_eq!(
        fake.queried().len(),
        2,
        "Next is disabled after a short page"
    );
}

#[gpui_kit::test]
fn clicking_the_count_runs_count_and_shows_the_total(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, _shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(rows(500)), &mut cx);
    click(&mut cx, "grid-count:500+");
    assert_eq!(
        fake.queried().last().map(String::as_str),
        Some(r#"SELECT COUNT(*) FROM "public"."users""#)
    );
    let mut total = rows(1);
    total.rows = vec![vec![Value::Int(1234)].into()];
    answer(&fake, Ok(total), &mut cx);
    assert!(cx.debug_bounds("grid-count:1,234").is_some());
}

#[gpui_kit::test]
fn f5_reads_the_same_page_again(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(rows(500)), &mut cx);
    click(&mut cx, "grid-next");
    answer(&fake, Ok(rows(500)), &mut cx);
    cx.simulate_keystrokes("f5");
    cx.run_until_parked();
    let queried = fake.queried();
    assert_eq!(queried.len(), 3);
    assert_eq!(queried[2], queried[1], "the same page again");
    answer(&fake, Ok(rows(2)), &mut cx);
    assert_eq!(grid_rows(&shell, &cx), 2);
}

#[gpui_kit::test]
fn a_failed_page_shows_the_error_and_retry_reads_it_again(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let error = QueryError {
        kind: QueryErrorKind::Server,
        message: "permission denied for table users".into(),
        position: None,
        statement: 0,
    };
    answer(&fake, Err(error), &mut cx);
    assert!(
        cx.debug_bounds("table-error:permission denied for table users")
            .is_some()
    );
    click(&mut cx, "table-retry");
    assert_eq!(fake.queried(), [PAGE_1, PAGE_1]);
    answer(&fake, Ok(rows(1)), &mut cx);
    assert!(cx.debug_bounds("table-retry").is_none());
    assert_eq!(grid_rows(&shell, &cx), 1);
}

#[gpui_kit::test]
fn retry_after_the_session_dropped_connects_again_and_keeps_it(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(rows(500)), &mut cx);
    let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
    let kept = explorer.update(&mut cx, |e, cx| {
        e.session(crate::model::ConnectionId(1), "app_db", cx)
    });
    let kept = futures::executor::block_on(kept).expect("the explorer kept the tab's session");
    fake::close(&kept);
    cx.simulate_keystrokes("f5");
    cx.run_until_parked();
    assert_eq!(
        fake.pending(),
        ["connect app_db"],
        "a closed session is not reused"
    );
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    answer(&fake, Ok(rows(1)), &mut cx);
    assert_eq!(grid_rows(&shell, &cx), 1);
    cx.simulate_keystrokes("f5");
    cx.run_until_parked();
    assert_eq!(
        fake.pending(),
        ["query app_db"],
        "the new session is kept for the next read"
    );
}
