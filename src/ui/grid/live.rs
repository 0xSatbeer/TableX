//! A table tab's rows, read from the database a page at a time.

use std::time::{Instant, SystemTime};

use gpui_kit::component::spinner::Spinner;

use super::table_page::{Failure, PageState, TABLE_PAGE, TablePage, count_sql, page_sql};
use super::*;
use crate::db::{Driver, Reply, Session};
use crate::ddl::ident;
use crate::model::{
    Column, Connection, ConnectionId, DbKind, ObjectKind, QueryError, QueryLogEntry, QuerySource,
    QueryStatus,
};
use crate::sessions::Sessions;

/// Where a table tab reads its rows.
#[derive(Clone)]
pub struct Table {
    pub driver: Arc<dyn Driver>,
    pub connection: ConnectionId,
    pub database: String,
    /// Owns the session on `database` that pages and counts run on.
    pub sessions: Entity<Sessions>,
    pub schema: String,
    pub name: String,
    /// A SQLite table, which reads its `rowid` when it has no primary key.
    pub sqlite_table: bool,
    /// A view (or materialized view): never editable.
    pub view: bool,
    /// The filter bar completes in this dialect.
    pub dialect: DbKind,
}

impl Table {
    /// Where `kind` object `name` in `database`'s `schema` on `conn` reads its rows.
    pub fn new(
        sessions: &Entity<Sessions>,
        conn: &Connection,
        (database, schema, name): (String, String, String),
        kind: ObjectKind,
        cx: &App,
    ) -> Self {
        Self {
            driver: sessions.read(cx).driver(),
            connection: conn.id,
            database,
            sessions: sessions.clone(),
            schema,
            name,
            sqlite_table: conn.kind() == DbKind::Sqlite && kind == ObjectKind::Table,
            view: kind != ObjectKind::Table,
            dialect: conn.kind(),
        }
    }
}

pub(super) struct Live {
    pub(super) table: Table,
    /// Primary key columns, read before the first page.
    pub(super) key: Option<Vec<String>>,
    /// Columns in a foreign key, read with `key`.
    pub(super) fks: Vec<String>,
    pub(super) page: TablePage,
    /// The last Submit's failing row in `all` and why; its edits stay pending.
    pub(super) submit_error: Option<(Option<usize>, String)>,
    pub(super) _submit: Option<Task<()>>,
    _load: Option<Task<()>>,
    _count: Option<Task<()>>,
}

pub enum GridEvent {
    Logged(QueryLogEntry),
}

impl EventEmitter<GridEvent> for Grid {}

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
            fks: Vec::new(),
            page: TablePage::new(),
            submit_error: None,
            _submit: None,
            _load: None,
            _count: None,
        });
        grid
    }

    /// The table this tab reads, with its applied WHERE and ORDER BY.
    pub fn source(&self) -> Option<(&Table, &str, &str)> {
        let live = self.live.as_ref()?;
        Some((&live.table, &live.page.where_, &live.page.order_by))
    }

    /// Sets the WHERE and ORDER BY the next `load` reads with, as a restored tab had them.
    pub fn set_filter(&mut self, where_: String, order_by: String) {
        if let Some(live) = &mut self.live {
            (live.page.where_, live.page.order_by) = (where_, order_by);
        }
    }

    /// Waits for a Connect click instead of reading, as a restored production tab does.
    pub fn set_disconnected(&mut self) {
        if let Some(live) = &mut self.live {
            live.page.state = PageState::Disconnected;
        }
    }

    /// Shows Disconnected · Reconnect over the rows, as every tab on a lost connection does.
    pub fn set_lost(&mut self, cx: &mut Context<Self>) {
        if let Some(live) = &mut self.live {
            live.page.state = PageState::Lost;
            cx.notify();
        }
    }

    /// Reads the page again if the tab shows its connection lost, now that it is back.
    pub fn restored(&mut self, cx: &mut Context<Self>) {
        if self
            .live
            .as_ref()
            .is_some_and(|l| l.page.state == PageState::Lost)
        {
            self.load(cx);
        }
    }

    /// Connect on a disconnected tab: connects its connection and reads the page.
    fn connect(&mut self, cx: &mut Context<Self>) {
        if let Some(live) = &self.live {
            let id = live.table.connection;
            live.table.sessions.update(cx, |s, cx| s.connect(id, cx));
        }
        self.load(cx);
    }

    /// A table tab whose page read is in flight.
    pub fn loading(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.page.state == PageState::Loading)
    }

    /// Reads the current page again (F5, Refresh, Retry), connecting and reading the key first if needed.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(live) = &mut self.live else { return };
        live.page.state = PageState::Loading;
        let t = &live.table;
        let driver = t.driver.clone();
        let (schema, name, key, offset) = (
            t.schema.clone(),
            t.name.clone(),
            live.key.clone(),
            live.page.offset,
        );
        let (where_, order_by) = (live.page.where_.clone(), live.page.order_by.clone());
        let sqlite_table = t.sqlite_table;
        let session = self.session(cx);
        let Some(live) = &mut self.live else { return };
        live._load = Some(cx.spawn(async move |this, cx| {
            let ready = async {
                let session = session.await?;
                let (key, fks) = match key {
                    Some(key) => (key, None),
                    None => {
                        let key = primary_key(driver.columns(&session, &schema, &name).await?);
                        // Only for the header icon: SQLite and views have no table_def, so none.
                        let fks = driver.table_def(&session, &schema, &name).await.map_or(
                            Vec::new(),
                            |def| {
                                def.foreign_keys
                                    .into_iter()
                                    .flat_map(|f| f.columns)
                                    .collect()
                            },
                        );
                        (key, Some(fks))
                    }
                };
                Ok::<_, String>((session, key, fks))
            };
            let (session, key, fks) = match ready.await {
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
                    if let Some(fks) = fks {
                        live.fks = fks;
                    }
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
            live.page.offset = offset;
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
        live.page.apply_filter(where_, order_by);
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

    /// A failed page read: the tab shows why, or its connection is lost.
    fn query_failed(&mut self, error: QueryError, cx: &mut Context<Self>) {
        let Some(live) = &mut self.live else { return };
        if live.page.read_failed(error) == Failure::Lost {
            let (id, sessions) = (live.table.connection, live.table.sessions.clone());
            sessions.update(cx, |s, cx| s.lost(id, cx));
        }
        cx.notify();
    }

    fn failed(&mut self, reason: String, cx: &mut Context<Self>) {
        if let Some(live) = &mut self.live {
            live.page.state = PageState::Failed(reason);
            cx.notify();
        }
    }

    fn paged(&mut self, rs: ResultSet, rowid: bool, cx: &mut Context<Self>) {
        let Some(live) = &mut self.live else { return };
        live.page.paged(rs.rows.len());
        live.submit_error = None;
        // The server sorted these rows; the header keeps showing which way.
        let sort = self.sort;
        let reset = self.replace(rs, cx);
        self.sort = sort.filter(|_| !reset);
        // SQLite's rowid tells rows apart for editing; it isn't the table's own column.
        if reset && rowid && self.hidden.len() > 1 {
            self.hidden[0] = true;
        }
    }

    fn count(&mut self, cx: &mut Context<Self>) {
        let session = self.session(cx);
        let Some(live) = &mut self.live else { return };
        let sql = count_sql(&live.table.schema, &live.table.name, &live.page.where_);
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
                    live.page.counted(total);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The open session on the table's database, connecting a new one when it has none.
    pub(super) fn session(&self, cx: &mut Context<Self>) -> Reply<Session> {
        let Some(live) = &self.live else {
            return Box::pin(futures::future::ready(Err("Not a table tab".into())));
        };
        let (id, database) = (live.table.connection, live.table.database.clone());
        live.table
            .sessions
            .update(cx, |s, cx| s.session(id, &database, cx))
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
        let len = self.loaded.row_count();
        let pager = live.page.pager(len);
        let offset = pager.offset;
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
        let (range, count) = (pager.range, pager.count);
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
                    Some(pager.prev),
                ))
                .child(div().px(theme::space_4()).child(range))
                .when(len > 0, |d| {
                    let known = pager.known;
                    d.child(
                        div()
                            .debug_selector(|| format!("grid-count:{count}"))
                            // A known total is plain text: a disabled button reads as broken.
                            .when(known, |d| d.child(count.clone()))
                            .when(!known, |d| {
                                d.child(
                                    Button::new("grid-count")
                                        .ghost()
                                        .small()
                                        .label(count.clone())
                                        .tooltip("Count rows")
                                        .on_click(cx.listener(|this, _, _, cx| this.count(cx))),
                                )
                            }),
                    )
                })
                .child(div().debug_selector(|| "grid-next".into()).child(button(
                    "grid-next",
                    IconName::ChevronRight,
                    "Next page",
                    pager.next,
                )))
                .child(button(
                    "grid-last",
                    IconName::ChevronLast,
                    "Last page",
                    pager.last,
                )),
        )
    }

    /// Why the filtered read failed, under the toolbar.
    pub(super) fn render_filter_error(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let reason = self.live.as_ref()?.page.filter_error.clone()?;
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

    /// The loading spinner, the error with Retry, or the empty-table copy over a table tab's rows.
    pub(super) fn render_live_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme();
        let overlay = || {
            v_flex()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .gap(theme::space_8())
                .bg(theme.table)
                .text_size(theme::ui_sm())
                .text_color(theme.muted_foreground)
        };
        let live = self.live.as_ref()?;
        match &live.page.state {
            PageState::Ready if self.all.row_count() > 0 => None,
            PageState::Ready => {
                let text = match live.page.where_.is_empty() {
                    true => "No rows",
                    false => "No rows match this filter",
                };
                Some(
                    overlay()
                        .child(
                            div()
                                .debug_selector(move || format!("table-empty:{text}"))
                                .child(text),
                        )
                        .into_any_element(),
                )
            }
            PageState::Disconnected => Some(
                overlay()
                    .child("Disconnected")
                    .child(
                        div().debug_selector(|| "table-connect".into()).child(
                            Button::new("table-connect")
                                .small()
                                .label("Connect")
                                .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                        ),
                    )
                    .into_any_element(),
            ),
            PageState::Lost => Some(
                overlay()
                    .child("Disconnected")
                    .child(
                        div().debug_selector(|| "table-reconnect".into()).child(
                            Button::new("table-reconnect")
                                .small()
                                .label("Reconnect")
                                .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                        ),
                    )
                    .into_any_element(),
            ),
            PageState::Loading => Some(
                overlay()
                    .debug_selector(|| "table-loading".into())
                    .child(Spinner::new().color(theme.muted_foreground))
                    .child("Loading…")
                    .into_any_element(),
            ),
            PageState::Failed(reason) => Some(
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
