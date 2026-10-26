//! Which page of a table tab is shown and how it got there: offset, total, the applied filter
//! with its rollback, and the read's state. Plain data, so its rules test without a window.

use super::source::grouped;
use crate::ddl::ident;
use crate::model::{QueryError, QueryErrorKind};

pub(super) const TABLE_PAGE: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PageState {
    /// A restored production tab: nothing is read until Connect is clicked.
    Disconnected,
    /// The connection was lost: nothing is read until Reconnect is clicked.
    Lost,
    Loading,
    Failed(String),
    Ready,
}

#[derive(Debug, PartialEq)]
pub(super) struct TablePage {
    pub(super) offset: usize,
    /// The last page came back with `TABLE_PAGE` rows, so more may follow.
    pub(super) full: bool,
    pub(super) total: Option<usize>,
    /// The applied WHERE and ORDER BY fragments, inserted raw.
    pub(super) where_: String,
    pub(super) order_by: String,
    /// Why the last filtered read failed, shown under the toolbar over the old rows.
    pub(super) filter_error: Option<String>,
    /// The where, order by, offset and total the shown rows came from, while an applied filter is unread.
    before: Option<(String, String, usize, Option<usize>)>,
    pub(super) state: PageState,
}

/// What a failed page read leaves the tab showing.
#[derive(Debug, PartialEq)]
pub(super) enum Failure {
    /// The session is gone: the connection is lost.
    Lost,
    /// The error over the rows, or under the toolbar for a filter's fault.
    Shown,
}

/// The table pager: where each button goes (None: disabled) and its labels.
#[derive(Debug, PartialEq)]
pub(super) struct Pager {
    pub(super) offset: usize,
    pub(super) prev: usize,
    pub(super) next: Option<usize>,
    pub(super) last: Option<usize>,
    /// "1–500 of", or "No rows".
    pub(super) range: String,
    /// The total, or the rows so far with "+" while it is unknown.
    pub(super) count: String,
    pub(super) known: bool,
}

impl TablePage {
    pub(super) fn new() -> Self {
        Self {
            offset: 0,
            full: false,
            total: None,
            where_: String::new(),
            order_by: String::new(),
            filter_error: None,
            before: None,
            state: PageState::Loading,
        }
    }

    /// Page 1 through `where_` and `order_by` (Enter, a header sort click). The shown rows'
    /// filter is kept, so a failing filter goes back to it.
    pub(super) fn apply_filter(&mut self, where_: String, order_by: String) {
        let old_where = std::mem::replace(&mut self.where_, where_);
        let old_order = std::mem::replace(&mut self.order_by, order_by);
        let total = self.total.take();
        self.before
            .get_or_insert((old_where, old_order, self.offset, total));
        self.offset = 0;
    }

    /// A page read came back with `rows` rows.
    pub(super) fn paged(&mut self, rows: usize) {
        self.full = rows == TABLE_PAGE;
        if !self.full {
            self.total = Some(self.offset + rows);
        }
        self.state = PageState::Ready;
        self.filter_error = None;
        self.before = None;
    }

    /// A failed page read: with a filter applied it is the filter's fault, so the old rows stay.
    /// The page goes back to what the shown rows came from.
    pub(super) fn read_failed(&mut self, error: QueryError) -> Failure {
        let filtered =
            self.before.is_some() || !self.where_.is_empty() || !self.order_by.is_empty();
        if let Some(before) = self.before.take() {
            (self.where_, self.order_by, self.offset, self.total) = before;
        }
        if error.kind == QueryErrorKind::Disconnected {
            self.state = PageState::Lost;
            return Failure::Lost;
        }
        if !filtered || error.kind != QueryErrorKind::Server {
            self.state = PageState::Failed(error.message);
        } else {
            self.filter_error = Some(error.message);
            self.state = PageState::Ready;
        }
        Failure::Shown
    }

    /// A count came back: `total`, or the one known before when it failed.
    pub(super) fn counted(&mut self, total: Option<usize>) {
        self.total = total.or(self.total);
    }

    /// The pager over a page of `len` rows.
    pub(super) fn pager(&self, len: usize) -> Pager {
        let offset = self.offset;
        Pager {
            offset,
            prev: offset.saturating_sub(TABLE_PAGE),
            next: self.full.then_some(offset + TABLE_PAGE),
            last: self
                .total
                .map(|n| n.saturating_sub(1) / TABLE_PAGE * TABLE_PAGE),
            range: if len == 0 {
                "No rows".to_string()
            } else {
                format!("{}–{} of", grouped(offset + 1), grouped(offset + len))
            },
            count: match self.total {
                Some(n) => grouped(n),
                None => format!("{}+", grouped(offset + len)),
            },
            known: self.total.is_some(),
        }
    }
}

pub(crate) fn page_sql(
    schema: &str,
    name: &str,
    key: &[String],
    rowid: bool,
    filter: (&str, &str),
    offset: usize,
) -> String {
    let select = select_sql(schema, name, key, rowid, filter);
    format!("{select} LIMIT {TABLE_PAGE} OFFSET {offset}")
}

/// Every row of the table through the filter, ordered as its pages are.
pub(crate) fn select_sql(
    schema: &str,
    name: &str,
    key: &[String],
    rowid: bool,
    (where_, order_by): (&str, &str),
) -> String {
    let select = if rowid { "rowid, *" } else { "*" };
    let order = if !order_by.is_empty() {
        format!(" ORDER BY {order_by}")
    } else if key.is_empty() {
        String::new()
    } else {
        // Newest first by default: the key descending.
        let cols: Vec<_> = key.iter().map(|c| format!("{} DESC", ident(c))).collect();
        format!(" ORDER BY {}", cols.join(", "))
    };
    format!(
        "SELECT {select} FROM {}.{}{}{order}",
        ident(schema),
        ident(name),
        where_clause(where_)
    )
}

pub(crate) fn count_sql(schema: &str, name: &str, where_: &str) -> String {
    format!(
        "SELECT COUNT(*) FROM {}.{}{}",
        ident(schema),
        ident(name),
        where_clause(where_)
    )
}

fn where_clause(where_: &str) -> String {
    if where_.is_empty() {
        String::new()
    } else {
        format!(" WHERE ({where_})")
    }
}
