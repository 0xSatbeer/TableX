//! The table page model: totals, filter rollback and failures, without a window.

use super::super::table_page::{Failure, PageState, TABLE_PAGE, TablePage};
use crate::model::{QueryError, QueryErrorKind};

fn error(kind: QueryErrorKind) -> QueryError {
    QueryError {
        kind,
        message: "boom".into(),
        position: None,
        statement: 0,
    }
}

/// A page on page 2 of a table whose total is known.
fn on_page_two() -> TablePage {
    let mut page = TablePage::new();
    page.paged(TABLE_PAGE);
    page.offset = TABLE_PAGE;
    page.paged(7);
    page
}

#[test]
fn a_full_page_leaves_the_total_unknown_and_a_short_one_ends_it() {
    let mut page = TablePage::new();
    page.paged(TABLE_PAGE);
    assert_eq!((page.total, page.full), (None, true));
    let pager = page.pager(TABLE_PAGE);
    assert_eq!(
        (
            pager.range.as_str(),
            pager.count.as_str(),
            pager.next,
            pager.last
        ),
        ("1–500 of", "500+", Some(TABLE_PAGE), None)
    );
    page.offset = TABLE_PAGE;
    page.paged(7);
    assert_eq!(page.total, Some(507));
    let pager = page.pager(7);
    assert_eq!(
        (
            pager.range.as_str(),
            pager.count.as_str(),
            pager.next,
            pager.last,
            pager.prev
        ),
        ("501–507 of", "507", None, Some(TABLE_PAGE), 0)
    );
    assert_eq!(TablePage::new().pager(0).range, "No rows");
}

#[test]
fn a_failed_filter_goes_back_to_the_shown_rows_and_says_why_under_the_toolbar() {
    let mut page = on_page_two();
    page.apply_filter("bad = ".into(), String::new());
    assert_eq!(
        (page.offset, page.total),
        (0, None),
        "page 1, total unknown"
    );
    assert_eq!(
        page.read_failed(error(QueryErrorKind::Server)),
        Failure::Shown
    );
    assert_eq!(
        (page.where_.as_str(), page.offset, page.total),
        ("", TABLE_PAGE, Some(507))
    );
    assert_eq!(page.filter_error.as_deref(), Some("boom"));
    assert_eq!(page.state, PageState::Ready);
}

#[test]
fn an_error_without_a_filter_or_not_from_the_server_replaces_the_rows() {
    // (filter applied, error kind)
    let cases = [
        (false, QueryErrorKind::Server),
        (true, QueryErrorKind::Cancelled),
    ];
    for (filtered, kind) in cases {
        let mut page = on_page_two();
        if filtered {
            page.apply_filter("a = 1".into(), String::new());
        }
        assert_eq!(page.read_failed(error(kind.clone())), Failure::Shown);
        assert_eq!(
            page.state,
            PageState::Failed("boom".into()),
            "{filtered} {kind:?}"
        );
        assert_eq!(page.filter_error, None);
    }
}

#[test]
fn a_disconnected_read_is_lost_and_keeps_the_shown_rows_filter() {
    let mut page = on_page_two();
    page.apply_filter("a = 1".into(), "b".into());
    assert_eq!(
        page.read_failed(error(QueryErrorKind::Disconnected)),
        Failure::Lost
    );
    assert_eq!(page.state, PageState::Lost);
    assert_eq!((page.where_.as_str(), page.order_by.as_str()), ("", ""));
}

#[test]
fn a_second_filter_before_the_first_reads_still_goes_back_to_the_shown_rows() {
    let mut page = on_page_two();
    page.apply_filter("a = 1".into(), String::new());
    page.apply_filter("a = 2".into(), String::new());
    page.read_failed(error(QueryErrorKind::Server));
    assert_eq!((page.where_.as_str(), page.offset), ("", TABLE_PAGE));
}

#[test]
fn a_failed_count_keeps_the_known_total() {
    let mut page = on_page_two();
    page.counted(None);
    assert_eq!(page.total, Some(507));
    page.counted(Some(9));
    assert_eq!(page.total, Some(9));
}
