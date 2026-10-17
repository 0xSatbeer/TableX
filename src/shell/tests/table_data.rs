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
    open_users_with(cx, vec![column("id", true), column("email", false)])
}

/// `open_users`, with `columns` as the table's columns.
fn open_users_with(
    cx: &mut gpui_kit::TestAppContext,
    columns: Vec<Column>,
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
    fake.resolve("columns app_db/public/users", Ok(columns));
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

fn filter_inputs(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cx: &gpui_kit::VisualTestContext,
) -> (
    gpui_kit::Entity<gpui_kit::component::input::InputState>,
    gpui_kit::Entity<gpui_kit::component::input::InputState>,
) {
    shell.read_with(cx, |this, cx| {
        match &this.tabs.active().expect("a tab").content {
            crate::shell::TabContent::Grid(grid) => grid.read(cx).filter_inputs(),
            crate::shell::TabContent::Console(_) => panic!("a grid tab is active"),
        }
    })
}

/// Types `where_` and `order_by` into the filter bar and presses Enter in the WHERE box.
fn apply_filter(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    where_: &str,
    order_by: &str,
    cx: &mut gpui_kit::VisualTestContext,
) {
    let (where_input, order_input) = filter_inputs(shell, cx);
    type_into(cx, order_input, order_by);
    type_into(cx, where_input, where_);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

const FILTERED: &str =
    r#"SELECT * FROM "public"."users" WHERE (id > 5) ORDER BY id DESC LIMIT 500 OFFSET 0"#;

#[gpui_kit::test]
fn enter_applies_where_and_order_by_from_page_one(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(rows(500)), &mut cx);
    click(&mut cx, "grid-next");
    answer(&fake, Ok(rows(500)), &mut cx);
    click(&mut cx, "grid-count:1,000+");
    let mut total = rows(1);
    total.rows = vec![vec![Value::Int(1234)].into()];
    answer(&fake, Ok(total), &mut cx);
    apply_filter(&shell, "id > 5", "id DESC", &mut cx);
    assert_eq!(fake.queried().last().map(String::as_str), Some(FILTERED));
    answer(&fake, Ok(rows(500)), &mut cx);
    assert!(
        cx.debug_bounds("grid-count:500+").is_some(),
        "the count starts over"
    );
    cx.simulate_keystrokes("f5");
    cx.run_until_parked();
    assert_eq!(
        fake.queried().last().map(String::as_str),
        Some(FILTERED),
        "F5 keeps the filter"
    );
    answer(&fake, Ok(rows(500)), &mut cx);
    click(&mut cx, "grid-count:500+");
    assert_eq!(
        fake.queried().last().map(String::as_str),
        Some(r#"SELECT COUNT(*) FROM "public"."users" WHERE (id > 5)"#)
    );
}

#[gpui_kit::test]
fn a_failing_filter_shows_the_error_and_keeps_the_rows(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(rows(500)), &mut cx);
    click(&mut cx, "grid-next");
    answer(&fake, Ok(rows(3)), &mut cx);
    apply_filter(&shell, "nope", "", &mut cx);
    let error = QueryError {
        kind: QueryErrorKind::Server,
        message: r#"column "nope" does not exist"#.into(),
        position: None,
        statement: 0,
    };
    answer(&fake, Err(error), &mut cx);
    assert!(
        cx.debug_bounds(r#"filter-error:column "nope" does not exist"#)
            .is_some()
    );
    assert!(cx.debug_bounds("table-retry").is_none(), "no overlay");
    assert_eq!(grid_rows(&shell, &cx), 3, "the old rows stay");
    assert!(
        cx.debug_bounds("grid-count:503").is_some(),
        "the pager still describes the shown rows"
    );
    cx.simulate_keystrokes("f5");
    cx.run_until_parked();
    assert_eq!(
        fake.queried().last().map(String::as_str),
        Some(r#"SELECT * FROM "public"."users" ORDER BY "id" LIMIT 500 OFFSET 500"#),
        "F5 reads the shown page, not the failed filter"
    );
    answer(&fake, Ok(rows(3)), &mut cx);
    apply_filter(&shell, "id > 1", "", &mut cx);
    answer(&fake, Ok(rows(1)), &mut cx);
    assert!(
        cx.debug_bounds(r#"filter-error:column "nope" does not exist"#)
            .is_none()
    );
    assert_eq!(grid_rows(&shell, &cx), 1);
}

#[gpui_kit::test]
fn header_clicks_cycle_the_sort_through_order_by(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(rows(2)), &mut cx);
    let order_by = |cx: &gpui_kit::VisualTestContext| {
        filter_inputs(&shell, cx)
            .1
            .read_with(cx, |input, _| input.value().to_string())
    };
    let page = |order: &str| {
        format!(r#"SELECT * FROM "public"."users" ORDER BY {order} LIMIT 500 OFFSET 0"#)
    };
    for (order, sql) in [
        (r#""id" ASC"#, page(r#""id" ASC"#)),
        (r#""id" DESC"#, page(r#""id" DESC"#)),
        ("", PAGE_1.to_string()),
    ] {
        click(&mut cx, "sort-0");
        assert_eq!(order_by(&cx), order);
        assert_eq!(fake.queried().last(), Some(&sql));
        answer(&fake, Ok(rows(2)), &mut cx);
    }
}

fn grid(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cx: &gpui_kit::VisualTestContext,
) -> gpui_kit::Entity<crate::ui::grid::Grid> {
    shell.read_with(cx, |this, _| {
        match &this.tabs.active().expect("a tab").content {
            crate::shell::TabContent::Grid(grid) => grid.clone(),
            crate::shell::TabContent::Console(_) => panic!("a grid tab is active"),
        }
    })
}

/// Rows `id`, `email`, `name` with ids 1..=n.
fn people(n: usize) -> ResultSet {
    let meta = |name: &str, ty: &str| ColumnMeta {
        name: name.into(),
        data_type: ty.into(),
        ..Default::default()
    };
    ResultSet {
        columns: vec![
            meta("id", "int4"),
            meta("email", "text"),
            meta("name", "text"),
        ],
        rows: (1..=n)
            .map(|i| {
                vec![
                    Value::Int(i as i64),
                    Value::Text(format!("{i}@x").into()),
                    Value::Null,
                ]
                .into()
            })
            .collect(),
    }
}

fn set(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cells: &[((usize, usize), &str)],
    cx: &mut gpui_kit::VisualTestContext,
) {
    grid(shell, cx).update(cx, |g, cx| {
        for &(cell, text) in cells {
            g.set_cell(cell, Value::Text(text.into()), cx);
        }
    });
    cx.run_until_parked();
}

fn pending_edits(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cx: &gpui_kit::VisualTestContext,
) -> usize {
    grid(shell, cx).read_with(cx, |g, _| g.pending_edits())
}

const UPDATE_ROW_1: &str =
    r#"UPDATE "public"."users" SET "email" = 'ann@x', "name" = 'Ann' WHERE "id" = 1"#;

#[gpui_kit::test]
fn production_submit_shows_the_sql_and_runs_only_on_confirm(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(people(2)), &mut cx);
    set(&shell, &[((0, 1), "ann@x"), ((0, 2), "Ann")], &mut cx);
    click(&mut cx, "grid-submit");
    let preview: &'static str = format!("prod-submit-sql:{UPDATE_ROW_1}").leak();
    assert!(cx.debug_bounds(preview).is_some(), "the exact SQL is shown");
    assert!(fake.executed().is_empty(), "nothing runs until confirmed");
    click(&mut cx, "prod-submit-cancel");
    assert!(cx.debug_bounds(preview).is_none());
    assert!(fake.executed().is_empty(), "Cancel runs nothing");
    assert_eq!(pending_edits(&shell, &cx), 2, "Cancel keeps the edits");

    click(&mut cx, "grid-submit");
    click(&mut cx, "prod-submit-confirm");
    assert_eq!(
        fake.executed(),
        [("app_db".to_string(), vec![UPDATE_ROW_1.to_string()], true)],
        "one UPDATE, in one transaction"
    );
    fake.resolve_query("execute app_db", Ok(()));
    cx.run_until_parked();
    assert_eq!(pending_edits(&shell, &cx), 0, "success clears the edits");
    assert_eq!(fake.queried(), [PAGE_1, PAGE_1], "and reads the page again");
    let logged = shell.read_with(&cx, |this, cx| {
        let log = this.bottom_panel.read(cx).logged();
        log.iter().find(|e| e.sql == UPDATE_ROW_1).map(|e| e.source)
    });
    assert_eq!(logged, Some(QuerySource::TableView));
}

#[gpui_kit::test]
fn a_failing_second_statement_keeps_the_edits_and_marks_its_row(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(people(3)), &mut cx);
    set(&shell, &[((2, 1), "c@x"), ((0, 1), "a@x")], &mut cx);
    click(&mut cx, "grid-submit");
    click(&mut cx, "prod-submit-confirm");
    let statements = &fake.executed()[0].1;
    assert!(
        statements[1].ends_with(r#"WHERE "id" = 3"#),
        "rows go in order"
    );
    let error = QueryError {
        kind: QueryErrorKind::Server,
        message: "duplicate key value".into(),
        position: None,
        statement: 1,
    };
    fake.resolve_query("execute app_db", Err::<(), _>(error));
    cx.run_until_parked();
    assert_eq!(pending_edits(&shell, &cx), 2, "the edits stay");
    assert!(
        cx.debug_bounds("submit-error:duplicate key value")
            .is_some()
    );
    assert!(
        cx.debug_bounds("failed-row:2").is_some(),
        "the failing row is marked"
    );
    assert!(cx.debug_bounds("failed-row:0").is_none());
    assert_eq!(fake.queried(), [PAGE_1], "nothing is read again");

    click(&mut cx, "grid-revert");
    assert_eq!(pending_edits(&shell, &cx), 0, "Revert drops the edits");
    assert!(cx.debug_bounds("failed-row:2").is_none());
}

#[gpui_kit::test]
fn a_postgres_table_without_a_key_is_read_only(cx: &mut gpui_kit::TestAppContext) {
    let (fake, window, shell) = open_users_with(cx, vec![column("id", false)]);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(people(2)), &mut cx);
    assert!(
        cx.debug_bounds("grid-read-only:No primary key: read-only")
            .is_some()
    );
    set(&shell, &[((0, 1), "x")], &mut cx);
    assert_eq!(pending_edits(&shell, &cx), 0, "edits are refused");
}
