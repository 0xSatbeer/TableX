//! The SQL console: an editor on the tab's own session that runs statements into result grids
//! and messages, which the bottom panel shows while the console is the active tab.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    v_flex,
};
use gpui_kit::*;

use crate::db::{Driver, Opened, Session};
use crate::model::{
    ColumnMeta, ConnectionId, ConnectionParams, QueryError, QueryLogEntry, QuerySource,
    QueryStatus, ResultSet, Row, Workspace,
};
use crate::ui::{explorer, grid};
use crate::{sql, theme};

actions!(console, [RunStatement, RunScript, Rerun]);

const CONTEXT: &str = "Console";
/// Rows a result page shows and each fetch reads.
const PAGE: usize = 500;
/// Rows a result keeps in memory; past it the oldest pages are dropped.
#[cfg(not(test))]
const CAP: usize = 100_000;
#[cfg(test)]
const CAP: usize = 3 * PAGE;

pub fn bind_keys(cx: &mut App) {
    // Matched at the editor's own depth, or its ctrl-enter (Enter { secondary }) wins.
    let editor = Some("Console > Input");
    cx.bind_keys([
        KeyBinding::new("ctrl-enter", RunStatement, editor),
        KeyBinding::new("ctrl-shift-enter", RunScript, editor),
        KeyBinding::new("f5", Rerun, Some(CONTEXT)),
    ]);
}

/// Where a console runs: the connection (looked up in `workspace` at connect) and its database.
pub struct Target {
    pub workspace: Entity<Workspace>,
    pub driver: Arc<dyn Driver>,
    pub connection: Option<ConnectionId>,
    pub database: String,
}

pub enum ConsoleEvent {
    /// One statement ran, for the query log.
    Logged(QueryLogEntry),
    /// A run ended, by finishing or failing.
    Finished,
}

pub struct Console {
    pub editor: Entity<EditorState>,
    target: Target,
    /// Opened on the first run and kept, so each console has its own.
    session: Option<Session>,
    last: Vec<String>,
    /// The last run's row-returning statements, in order.
    pub results: Vec<Paged>,
    pub active_result: usize,
    pub messages: Vec<String>,
    /// Set by the shell: the editor's zoomed mono size.
    pub text_size: Pixels,
    running: bool,
    _run: Option<Task<()>>,
    _fetch: Option<Task<()>>,
}

/// One result tab: the rows read so far, shown a page at a time.
pub struct Paged {
    /// The current page's rows.
    pub grid: Entity<grid::Grid>,
    columns: Vec<ColumnMeta>,
    rows: Vec<Row>,
    /// Rows dropped from the front to stay under `CAP`.
    dropped: usize,
    page: usize,
    /// Still open on the session, so Next can fetch more.
    open: bool,
    /// Every row has been read.
    done: bool,
    fetching: bool,
}

impl Paged {
    fn new(rs: ResultSet, cx: &mut App) -> Self {
        let done = rs.rows.len() < PAGE;
        Self {
            grid: cx.new(|cx| grid::Grid::new(Rc::new(rs.clone()), cx)),
            columns: rs.columns,
            rows: rs.rows,
            dropped: 0,
            page: 0,
            open: !done,
            done,
            fetching: false,
        }
    }

    fn total(&self) -> usize {
        self.dropped + self.rows.len()
    }

    /// e.g. "501–1000 of 500+" while more rows may follow, "1001–1234 of 1234" once all are read.
    pub fn label(&self) -> String {
        let total = self.total();
        if total == 0 {
            return "No rows".into();
        }
        let start = self.page * PAGE;
        let end = (start + PAGE).min(total);
        match self.done {
            true => format!("{}–{end} of {total}", start + 1),
            false => format!("{}–{end} of {PAGE}+", start + 1),
        }
    }

    /// Set when the page shown was dropped to save memory.
    pub fn notice(&self) -> Option<String> {
        (self.page * PAGE < self.dropped).then(|| {
            format!(
                "Rows before {} were dropped to save memory · Run again",
                self.dropped + 1
            )
        })
    }

    pub fn can_prev(&self) -> bool {
        self.page > 0
    }

    pub fn can_next(&self) -> bool {
        (self.page + 1) * PAGE < self.total() || (self.open && !self.fetching)
    }

    /// Why Next is disabled on a result a later statement closed.
    pub fn next_tip(&self) -> Option<&'static str> {
        (!self.done && !self.open).then_some("Run this statement alone to page further")
    }

    /// Points the grid at the current page, unless it was dropped.
    fn show(&mut self, cx: &mut App) {
        let Some(start) = (self.page * PAGE).checked_sub(self.dropped) else {
            return;
        };
        let end = (start + PAGE).min(self.rows.len());
        let rs = ResultSet {
            columns: self.columns.clone(),
            rows: self.rows[start..end].to_vec(),
        };
        self.grid = cx.new(|cx| grid::Grid::new(Rc::new(rs), cx));
    }

    /// Appends a fetch's rows, moving to them, and drops whole pages from the front past `CAP`.
    fn append(&mut self, rows: Vec<Row>, cx: &mut App) {
        if rows.len() < PAGE {
            (self.open, self.done) = (false, true);
        }
        if rows.is_empty() {
            return;
        }
        self.rows.extend(rows);
        let over = self.rows.len().saturating_sub(CAP).next_multiple_of(PAGE);
        self.rows.drain(..over);
        self.dropped += over;
        self.page += 1;
        self.show(cx);
    }
}

impl EventEmitter<ConsoleEvent> for Console {}

struct Step {
    sql: String,
    at: SystemTime,
    duration: Duration,
    result: Result<Ran, QueryError>,
}

enum Ran {
    Rows(ResultSet),
    Affected(u64),
}

impl Console {
    pub fn new(target: Target, text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = text.to_string();
        Self {
            editor: cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("sql")
                    .default_value(text)
            }),
            target,
            session: None,
            last: Vec::new(),
            results: Vec::new(),
            active_result: 0,
            messages: Vec::new(),
            text_size: theme::scaled(theme::MONO),
            running: false,
            _run: None,
            _fetch: None,
        }
    }

    /// The selection's statements, else the one under the cursor.
    fn run_statement(&mut self, _: &RunStatement, _: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        let text = editor.value();
        let selected = editor.selected_range();
        let statements = if selected.is_empty() {
            sql::at_cursor(&text, editor.cursor())
                .map(|r| text[r].to_string())
                .into_iter()
                .collect()
        } else {
            statements(&text[selected])
        };
        self.run(statements, cx);
    }

    fn run_script(&mut self, _: &RunScript, _: &mut Window, cx: &mut Context<Self>) {
        let statements = statements(&self.editor.read(cx).value());
        self.run(statements, cx);
    }

    fn rerun(&mut self, _: &Rerun, _: &mut Window, cx: &mut Context<Self>) {
        self.run_again(cx);
    }

    /// Runs `statements` one after another, stopping at the first error. Replaces the last run's output.
    fn run(&mut self, statements: Vec<String>, cx: &mut Context<Self>) {
        if statements.is_empty() {
            return;
        }
        self.last = statements.clone();
        (self.results, self.active_result, self.messages) = (Vec::new(), 0, Vec::new());
        self._fetch = None;
        cx.notify();
        let Some(params) = self.params(cx) else {
            self.messages.push("This console has no connection".into());
            return;
        };
        self.running = true;
        let driver = self.target.driver.clone();
        let session = self.session.clone().filter(|s| !s.is_closed());
        self._run = Some(cx.spawn(async move |this, cx| {
            let session = match session {
                Some(session) => session,
                None => match driver.connect(&params).await {
                    Ok(session) => session,
                    Err(reason) => {
                        this.update(cx, |this, cx| this.finish(Some(reason), cx))
                            .ok();
                        return;
                    }
                },
            };
            this.update(cx, |this, _| this.session = Some(session.clone()))
                .ok();
            for sql in statements {
                let (at, started) = (SystemTime::now(), Instant::now());
                let result = run_one(&*driver, &session, sql.clone()).await;
                let duration = started.elapsed();
                let step = Step {
                    sql,
                    at,
                    duration,
                    result,
                };
                let ok = this.update(cx, |this, cx| this.show(step, cx));
                if !ok.unwrap_or(false) {
                    break;
                }
            }
            this.update(cx, |this, cx| this.finish(None, cx)).ok();
        }));
    }

    fn params(&self, cx: &App) -> Option<ConnectionParams> {
        let id = self.target.connection?;
        let workspace = self.target.workspace.read(cx);
        let conn = workspace.connections.iter().find(|c| c.id == id)?;
        Some(explorer::on_database(&conn.params, &self.target.database))
    }

    /// Adds `step`'s result tab or message and logs it; false when it failed.
    fn show(&mut self, step: Step, cx: &mut Context<Self>) -> bool {
        let Some(connection_id) = self.target.connection else {
            return false;
        };
        // Each `open` closes the session's previous result.
        for result in &mut self.results {
            result.open = false;
        }
        let (rows, status) = match step.result {
            Ok(Ran::Rows(rs)) => {
                let n = rs.rows.len() as u64;
                self.results.push(Paged::new(rs, cx));
                (Some(n), QueryStatus::Ok)
            }
            Ok(Ran::Affected(n)) => {
                self.messages.push(format!("{n} rows affected"));
                (Some(n), QueryStatus::Ok)
            }
            Err(error) => {
                self.messages.push(error.message.clone());
                (None, QueryStatus::Err(error.message))
            }
        };
        let ok = rows.is_some();
        cx.emit(ConsoleEvent::Logged(QueryLogEntry {
            at: step.at,
            connection_id,
            database: self.target.database.clone(),
            sql: step.sql,
            duration: step.duration,
            rows,
            status,
            source: QuerySource::Console,
        }));
        cx.notify();
        ok
    }

    fn finish(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.running = false;
        self.messages.extend(error);
        cx.emit(ConsoleEvent::Finished);
        cx.notify();
    }
}

impl Console {
    /// Shows result `ix`'s next page, fetching it when it isn't in memory yet.
    pub fn next_page(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(result) = self.results.get_mut(ix) else {
            return;
        };
        if !result.can_next() {
            return;
        }
        if (result.page + 1) * PAGE < result.total() {
            result.page += 1;
            result.show(cx);
            cx.notify();
            return;
        }
        let Some(session) = self.session.clone().filter(|_| !self.running) else {
            return;
        };
        result.fetching = true;
        cx.notify();
        let driver = self.target.driver.clone();
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let rows = driver.fetch(&session, PAGE).await;
            this.update(cx, |this, cx| this.fetched(ix, rows, cx)).ok();
        }));
    }

    fn fetched(&mut self, ix: usize, rows: Result<Vec<Row>, QueryError>, cx: &mut Context<Self>) {
        let Some(result) = self.results.get_mut(ix) else {
            return;
        };
        result.fetching = false;
        match rows {
            Ok(rows) => result.append(rows, cx),
            Err(error) => {
                result.open = false;
                self.messages.push(error.message);
            }
        }
        cx.notify();
    }

    /// Shows result `ix`'s previous page, from memory.
    pub fn prev_page(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(result) = self.results.get_mut(ix).filter(|r| r.can_prev()) else {
            return;
        };
        result.page -= 1;
        result.show(cx);
        cx.notify();
    }

    /// Re-runs the last run, as the dropped-rows notice offers.
    pub fn run_again(&mut self, cx: &mut Context<Self>) {
        self.run(self.last.clone(), cx);
    }

    /// Closes the session's open result, as when the console tab closes; a later run reconnects.
    pub fn close_session(&mut self, cx: &mut Context<Self>) {
        (self._run, self._fetch, self.running) = (None, None, false);
        for result in &mut self.results {
            (result.open, result.fetching) = (false, false);
        }
        if let Some(session) = self.session.take() {
            cx.background_spawn(self.target.driver.close(&session))
                .detach();
        }
    }
}

/// Every statement of `text`.
fn statements(text: &str) -> Vec<String> {
    sql::split(text)
        .into_iter()
        .map(|r| text[r].to_string())
        .collect()
}

/// Opens `sql`; a row-returning statement's first `PAGE` rows are read.
async fn run_one(driver: &dyn Driver, session: &Session, sql: String) -> Result<Ran, QueryError> {
    match driver.open(session, sql, true).await? {
        Opened::Affected(n) => Ok(Ran::Affected(n)),
        Opened::Rows(columns) => {
            let rows = driver.fetch(session, PAGE).await?;
            Ok(Ran::Rows(ResultSet { columns, rows }))
        }
    }
}

impl Render for Console {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toolbar = h_flex()
            .flex_none()
            .h(theme::tab())
            .px(theme::space_8())
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("console-run")
                    .ghost()
                    .xsmall()
                    .label("Run")
                    .tooltip_with_action("Run statement", &RunStatement, None)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_statement(&RunStatement, window, cx)
                    })),
            );
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::run_statement))
            .on_action(cx.listener(Self::run_script))
            .on_action(cx.listener(Self::rerun))
            .child(toolbar)
            .child(
                div().flex_1().min_h_0().child(
                    Editor::new(&self.editor)
                        .bordered(false)
                        .h_full()
                        .text_size(self.text_size),
                ),
            )
    }
}

#[cfg(test)]
mod tests;
