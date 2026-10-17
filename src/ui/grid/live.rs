//! A table tab's rows, read from the database a page at a time.

use std::time::{Instant, SystemTime};

use gpui_kit::component::spinner::Spinner;

use super::*;
use crate::db::{Driver, Reply, Session};
use crate::ddl::ident;
use crate::model::{
    Column, ConnectionId, QueryError, QueryErrorKind, QueryLogEntry, QuerySource, QueryStatus,
};
use crate::ui::explorer::Explorer;

pub(super) const TABLE_PAGE: usize = 500;

/// Where a table tab reads its rows.
pub struct Table {
    pub driver: Arc<dyn Driver>,
    pub connection: ConnectionId,
    pub database: String,
    /// Owns the session on `database` that pages and counts run on.
    pub explorer: WeakEntity<Explorer>,
    pub schema: String,
    pub name: String,
    /// A SQLite table, which reads its `rowid` when it has no primary key.
    pub sqlite_table: bool,
    /// A view (or materialized view): never editable.
    pub view: bool,
}

pub(super) struct Live {
    pub(super) table: Table,
    /// Primary key columns, read before the first page.
    pub(super) key: Option<Vec<String>>,
    offset: usize,
    /// The last page came back with `TABLE_PAGE` rows, so more may follow.
    full: bool,
    total: Option<usize>,
    /// The applied WHERE and ORDER BY fragments, inserted raw.
    where_: String,
    order_by: String,
    /// Why the last filtered read failed, shown under the toolbar over the old rows.
    filter_error: Option<String>,
    /// The where, order by, offset and total the shown rows came from, while an applied filter is unread.
    before: Option<(String, String, usize, Option<usize>)>,
    state: LiveState,
    /// The last Submit's failing row in `all` and why; its edits stay pending.
    pub(super) submit_error: Option<(Option<usize>, String)>,
    pub(super) _submit: Option<Task<()>>,
    _load: Option<Task<()>>,
    _count: Option<Task<()>>,
}

enum LiveState {
    Loading,
    Failed(String),
    Ready,
}

pub enum GridEvent {
    Logged(QueryLogEntry),
}

impl EventEmitter<GridEvent> for Grid {}

pub(crate) fn page_sql(
    schema: &str,
    name: &str,
    key: &[String],
    rowid: bool,
    (where_, order_by): (&str, &str),
    offset: usize,
) -> String {
    let select = if rowid { "rowid, *" } else { "*" };
    let order = if !order_by.is_empty() {
        format!(" ORDER BY {order_by}")
    } else if key.is_empty() {
        String::new()
    } else {
        let cols: Vec<_> = key.iter().map(|c| ident(c)).collect();
        format!(" ORDER BY {}", cols.join(", "))
    };
    format!(
        "SELECT {select} FROM {}.{}{}{order} LIMIT {TABLE_PAGE} OFFSET {offset}",
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

fn primary_key(columns: Vec<Column>) -> Vec<String> {
    columns
        .into_iter()
        .filter(|c| c.is_pk)
        .map(|c| c.name)
        .collect()
}

impl Grid {
    /// An empty grid for `table`; `load` reads its first page.
    pub fn table(table: Table, cx: &mut Context<Self>) -> Self {
        let empty = ResultSet {
            columns: Vec::new(),
            rows: Vec::new(),
        };
        let mut grid = Self::new(Rc::new(empty), cx);
        grid.page_size = TABLE_PAGE;
        grid.live = Some(Live {
            table,
            key: None,
            offset: 0,
            full: false,
            total: None,
            where_: String::new(),
            order_by: String::new(),
            filter_error: None,
            before: None,
            state: LiveState::Loading,
            submit_error: None,
            _submit: None,
            _load: None,
            _count: None,
        });
        grid
    }

    /// Reads the current page again (F5, Refresh, Retry), connecting and reading the key first if needed.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(live) = &mut self.live else { return };
        live.state = LiveState::Loading;
        let t = &live.table;
        let driver = t.driver.clone();
        let (schema, name, key, offset) = (
            t.schema.clone(),
            t.name.clone(),
            live.key.clone(),
            live.offset,
        );
        let (where_, order_by) = (live.where_.clone(), live.order_by.clone());
        let sqlite_table = t.sqlite_table;
        let session = self.session(cx);
        let Some(live) = &mut self.live else { return };
        live._load = Some(cx.spawn(async move |this, cx| {
            let ready = async {
                let session = session.await?;
                let key = match key {
                    Some(key) => key,
                    None => primary_key(driver.columns(&session, &schema, &name).await?),
                };
                Ok::<_, String>((session, key))
            };
            let (session, key) = match ready.await {
                Ok(ready) => ready,
                Err(reason) => {
                    this.update(cx, |this, cx| this.failed(reason, cx)).ok();
                    return;
                }
            };
            let rowid = sqlite_table && key.is_empty();
            let sql = page_sql(&schema, &name, &key, rowid, (&where_, &order_by), offset);
            this.update(cx, |this, _| {
                if let Some(live) = &mut this.live {
                    live.key = Some(key);
                }
            })
            .ok();
            let (at, started) = (SystemTime::now(), Instant::now());
            let result = driver.query(&session, sql.clone()).await;
            this.update(cx, |this, cx| {
                this.log(sql, at, started, &result, cx);
                match result {
                    Ok(rs) => this.paged(rs, rowid, cx),
                    Err(error) => this.query_failed(error, cx),
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn goto(&mut self, offset: usize, cx: &mut Context<Self>) {
        if let Some(live) = &mut self.live {
            live.offset = offset;
            self.load(cx);
        }
    }

    /// Reads page 1 with the filter bar's WHERE and ORDER BY (Enter, a header sort click).
    pub(super) fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let Some((where_input, order_input)) = &self.filter else {
            return;
        };
        let (where_, order_by) = (
            where_input.read(cx).value().trim().to_string(),
            order_input.read(cx).value().trim().to_string(),
        );
        let Some(live) = &mut self.live else { return };
        let (old_where, old_order) = (
            std::mem::replace(&mut live.where_, where_),
            std::mem::replace(&mut live.order_by, order_by),
        );
        let (offset, total) = (live.offset, live.total.take());
        live.before
            .get_or_insert((old_where, old_order, offset, total));
        live.offset = 0;
        self.load(cx);
    }

    /// The ORDER BY a header sort writes: `"col" ASC|DESC`, or nothing.
    pub(super) fn sort_order_by(&self) -> String {
        self.sort.map_or(String::new(), |(c, asc)| {
            let dir = if asc { "ASC" } else { "DESC" };
            format!("{} {dir}", ident(&self.all.columns()[c].name))
        })
    }

    /// A header sort click on a table tab: the server sorts, through the ORDER BY box.
    pub(super) fn sort_live(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let order_by = self.sort_order_by();
        if let Some((_, input)) = &self.filter {
            input.update(cx, |input, cx| input.set_value(order_by, window, cx));
        }
        self.apply_filter(cx);
    }

    /// A failed page read: with a filter applied it is the filter's fault, so the old rows stay.
    /// The tab goes back to what the shown rows came from.
    fn query_failed(&mut self, error: QueryError, cx: &mut Context<Self>) {
        let Some(live) = &mut self.live else { return };
        let filtered =
            live.before.is_some() || !live.where_.is_empty() || !live.order_by.is_empty();
        if let Some(before) = live.before.take() {
            (live.where_, live.order_by, live.offset, live.total) = before;
        }
        if !filtered || error.kind != QueryErrorKind::Server {
            return self.failed(error.message, cx);
        }
        live.filter_error = Some(error.message);
        live.state = LiveState::Ready;
        cx.notify();
    }

    fn failed(&mut self, reason: String, cx: &mut Context<Self>) {
        if let Some(live) = &mut self.live {
            live.state = LiveState::Failed(reason);
            cx.notify();
        }
    }

    fn paged(&mut self, rs: ResultSet, rowid: bool, cx: &mut Context<Self>) {
        let Some(live) = &mut self.live else { return };
        live.full = rs.rows.len() == TABLE_PAGE;
        if !live.full {
            live.total = Some(live.offset + rs.rows.len());
        }
        live.state = LiveState::Ready;
        live.filter_error = None;
        live.before = None;
        live.submit_error = None;
        let reset = rs.columns != self.all.columns();
        self.loaded = Rc::new(rs);
        self.all = self.loaded.clone();
        (self.added, self.deleted) = (0, BTreeSet::new());
        self.edits.clear();
        self.editing = None;
        // The server sorted these rows; the header keeps showing which way.
        let sort = self.sort;
        self.set_page(0, TABLE_PAGE, cx);
        self.sort = sort.filter(|_| !reset);
        if reset {
            let n = self.all.columns().len();
            (self.widths, self.hidden) = (vec![DEFAULT_WIDTH; n], vec![false; n]);
            self.col_order = (0..n).collect();
            // SQLite's rowid tells rows apart for editing; it isn't the table's own column.
            if rowid && n > 1 {
                self.hidden[0] = true;
            }
            for c in 0..n {
                self.autofit(c);
                self.widths[c] = self.widths[c].min(MAX_OPEN_WIDTH);
            }
        }
    }

    fn count(&mut self, cx: &mut Context<Self>) {
        let session = self.session(cx);
        let Some(live) = &mut self.live else { return };
        let sql = count_sql(&live.table.schema, &live.table.name, &live.where_);
        let driver = live.table.driver.clone();
        live._count = Some(cx.spawn(async move |this, cx| {
            let Ok(session) = session.await else { return };
            let (at, started) = (SystemTime::now(), Instant::now());
            let result = driver.query(&session, sql.clone()).await;
            this.update(cx, |this, cx| {
                this.log(sql, at, started, &result, cx);
                let total = match result
                    .as_ref()
                    .ok()
                    .and_then(|rs| rs.rows.first()?.first().cloned())
                {
                    Some(Value::Int(n)) => usize::try_from(n).ok(),
                    _ => None,
                };
                if let Some(live) = &mut this.live {
                    live.total = total.or(live.total);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The explorer's open session on the table's database, connecting a new one when it has none.
    pub(super) fn session(&self, cx: &mut Context<Self>) -> Reply<Session> {
        let gone = || -> Reply<Session> {
            Box::pin(futures::future::ready(Err("The explorer is closed".into())))
        };
        let Some(live) = &self.live else {
            return gone();
        };
        let (id, database) = (live.table.connection, live.table.database.clone());
        live.table
            .explorer
            .update(cx, |e, cx| e.session(id, &database, cx))
            .unwrap_or_else(|_| gone())
    }

    fn log(
        &self,
        sql: String,
        at: SystemTime,
        started: Instant,
        result: &Result<ResultSet, QueryError>,
        cx: &mut Context<Self>,
    ) {
        let outcome = match result {
            Ok(rs) => (Some(rs.rows.len() as u64), QueryStatus::Ok),
            Err(error) => (None, QueryStatus::Err(error.message.clone())),
        };
        self.log_status(sql, (at, started), outcome, cx);
    }

    pub(super) fn log_status(
        &self,
        sql: String,
        (at, started): (SystemTime, Instant),
        (rows, status): (Option<u64>, QueryStatus),
        cx: &mut Context<Self>,
    ) {
        let Some(live) = &self.live else { return };
        cx.emit(GridEvent::Logged(QueryLogEntry {
            at,
            connection_id: live.table.connection,
            database: live.table.database.clone(),
            sql,
            duration: started.elapsed(),
            rows,
            status,
            source: QuerySource::TableView,
        }));
    }

    pub(super) fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// First, Previous, the row range with its count, Next and Last for a table tab.
    pub(super) fn render_live_pager(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let live = self.live.as_ref()?;
        let (offset, len) = (live.offset, self.loaded.row_count());
        let last = live
            .total
            .map(|n| n.saturating_sub(1) / TABLE_PAGE * TABLE_PAGE);
        let button = |id: &'static str, icon: IconName, tip: &'static str, to: Option<usize>| {
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .tooltip(tip)
                .accessibility_label(tip)
                .disabled(to.is_none_or(|to| to == offset))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(to) = to {
                        this.goto(to, cx)
                    }
                }))
        };
        let count = match live.total {
            Some(n) => grouped(n),
            None => format!("{}+", grouped(offset + len)),
        };
        let range = if len == 0 {
            "No rows".to_string()
        } else {
            format!("{}–{} of", grouped(offset + 1), grouped(offset + len))
        };
        Some(
            h_flex()
                .gap(theme::space_2())
                .child(button(
                    "grid-first",
                    IconName::ChevronFirst,
                    "First page",
                    Some(0),
                ))
                .child(button(
                    "grid-prev",
                    IconName::ChevronLeft,
                    "Previous page",
                    Some(offset.saturating_sub(TABLE_PAGE)),
                ))
                .child(div().px(theme::space_4()).child(range))
                .when(len > 0, |d| {
                    d.child(
                        div()
                            .debug_selector(|| format!("grid-count:{count}"))
                            .child(
                                Button::new("grid-count")
                                    .ghost()
                                    .small()
                                    .label(count.clone())
                                    .tooltip("Count rows")
                                    .disabled(live.total.is_some())
                                    .on_click(cx.listener(|this, _, _, cx| this.count(cx))),
                            ),
                    )
                })
                .child(div().debug_selector(|| "grid-next".into()).child(button(
                    "grid-next",
                    IconName::ChevronRight,
                    "Next page",
                    live.full.then_some(offset + TABLE_PAGE),
                )))
                .child(button(
                    "grid-last",
                    IconName::ChevronLast,
                    "Last page",
                    last,
                )),
        )
    }

    /// Why the filtered read failed, under the toolbar.
    pub(super) fn render_filter_error(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let reason = self.live.as_ref()?.filter_error.clone()?;
        let theme = cx.theme();
        Some(
            div()
                .debug_selector(|| format!("filter-error:{reason}"))
                .flex_none()
                .px(theme::space_8())
                .py(theme::space_2())
                .border_b_1()
                .border_color(theme.border)
                .text_size(theme::ui_sm())
                .text_color(theme.danger)
                .child(format!(
                    "The filter failed: {reason}. Fix WHERE or ORDER BY and press Enter."
                )),
        )
    }

    /// The loading spinner, or the error with Retry, over a table tab's rows.
    pub(super) fn render_live_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme();
        let overlay = || {
            v_flex()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .gap(theme::space_8())
                .bg(theme.background)
                .text_size(theme::ui_sm())
                .text_color(theme.muted_foreground)
        };
        match &self.live.as_ref()?.state {
            LiveState::Ready => None,
            LiveState::Loading => Some(
                overlay()
                    .debug_selector(|| "table-loading".into())
                    .child(Spinner::new().color(theme.muted_foreground))
                    .child("Loading…")
                    .into_any_element(),
            ),
            LiveState::Failed(reason) => Some(
                overlay()
                    .child(
                        div()
                            .debug_selector(|| format!("table-error:{reason}"))
                            .px(theme::space_16())
                            .text_color(theme.danger)
                            .child(reason.clone()),
                    )
                    .child(
                        div().debug_selector(|| "table-retry".into()).child(
                            Button::new("table-retry")
                                .small()
                                .label("Retry")
                                .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                        ),
                    )
                    .into_any_element(),
            ),
        }
    }
}
