//! The SQL console: an editor on the tab's own session that runs statements into result grids
//! and messages, which the bottom panel shows while the console is the active tab.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::input::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, InputEvent, RopeExt as _},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::{Driver, Opened, Reply, Session};
use crate::model::{
    ColumnMeta, ConnectionId, ConnectionParams, QueryError, QueryErrorKind, QueryLogEntry,
    QuerySource, QueryStatus, ResultSet, Row, Workspace,
};
use crate::ui::{explorer, grid, tabs};
use crate::{ddl, sql, theme};

actions!(
    console,
    [RunStatement, RunScript, Rerun, Cancel, AcceptCompletion]
);

pub mod completion;

pub(crate) const CONTEXT: &str = "Console";
/// How long after the last edit the text is saved.
pub const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Rows a result page shows and each fetch reads.
const PAGE: usize = 500;
/// Rows a result keeps in memory; past it the oldest pages are dropped.
#[cfg(not(test))]
const CAP: usize = 100_000;
#[cfg(test)]
const CAP: usize = 3 * PAGE;

#[cfg(test)]
pub fn bind_keys(cx: &mut App) {
    crate::keymap::bind_defaults(cx, |action, _| action.starts_with("console::"));
}

/// Where a console runs: the connection (looked up in `workspace` at connect), its database
/// and, when picked, the schema its `search_path` is set to.
#[derive(Clone)]
pub struct Target {
    pub workspace: Entity<Workspace>,
    pub driver: Arc<dyn Driver>,
    pub connection: Option<ConnectionId>,
    pub database: String,
    pub schema: Option<String>,
}

pub enum ConsoleEvent {
    /// One statement ran, for the query log.
    Logged(QueryLogEntry),
    /// A run ended, by finishing or failing.
    Finished,
    /// The database or schema changed, so the tab's origin and title follow `Console::tail`.
    Moved,
}

pub struct Console {
    pub editor: Entity<EditorState>,
    target: Target,
    /// Opened on the first run and kept, so each console has its own.
    session: Option<Session>,
    /// The last run's statements with their byte offsets in the editor text.
    last: Vec<(usize, String)>,
    /// The last run's row-returning statements, in order.
    pub results: Vec<Paged>,
    pub active_result: usize,
    pub messages: Vec<String>,
    /// Set by the shell: the editor's zoomed mono size.
    pub text_size: Pixels,
    running: bool,
    /// Manual mode: runs open a transaction that only Commit or Rollback ends.
    manual: bool,
    pub in_transaction: bool,
    /// What the toolbar's database and schema dropdowns offer, listed on the tab's session.
    pub databases: Vec<String>,
    pub schemas: Vec<String>,
    _run: Option<Task<()>>,
    _fetch: Option<Task<()>>,
    _context: Option<Task<()>>,
    /// `consoles/<id>.sql`, set by the shell; the text autosaves there.
    file: Option<std::path::PathBuf>,
    _save: Option<Task<()>>,
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
    /// The statement, for the grid's Export All rows.
    rerun: Option<grid::Rerun>,
}

impl Paged {
    fn new(rs: ResultSet, rerun: Option<grid::Rerun>, cx: &mut App) -> Self {
        let done = rs.rows.len() < PAGE;
        Self {
            grid: grid(rs.clone(), rerun.clone(), cx),
            columns: rs.columns,
            rows: rs.rows,
            dropped: 0,
            page: 0,
            open: !done,
            done,
            fetching: false,
            rerun,
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
        self.grid = grid(rs, self.rerun.clone(), cx);
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
    offset: usize,
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
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .default_value(text)
        });
        let provider = Rc::new(completion::Provider {
            console: cx.weak_entity(),
        });
        editor.update(cx, |e, _| e.lsp_mut().completion_provider = Some(provider));
        cx.subscribe(&editor, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                this.text_changed(cx);
            }
        })
        .detach();
        // A change still inside its debounce is written before the app goes.
        cx.on_app_quit(|this, cx| {
            this.save_text(cx);
            async {}
        })
        .detach();
        Self {
            editor,
            target,
            session: None,
            last: Vec::new(),
            results: Vec::new(),
            active_result: 0,
            messages: Vec::new(),
            text_size: theme::scaled(theme::MONO),
            running: false,
            manual: false,
            in_transaction: false,
            databases: Vec::new(),
            schemas: Vec::new(),
            _run: None,
            _fetch: None,
            _context: None,
            file: None,
            _save: None,
        }
    }

    /// Autosaves the text to `file` from now on.
    pub fn set_file(&mut self, file: std::path::PathBuf) {
        self.file = Some(file);
    }

    pub fn file(&self) -> Option<&std::path::Path> {
        self.file.as_deref()
    }

    pub fn text(&self, cx: &App) -> String {
        self.editor.read(cx).value().to_string()
    }

    /// Saves the text `SAVE_DELAY` after the last edit.
    fn text_changed(&mut self, cx: &mut Context<Self>) {
        if self.file.is_none() {
            return;
        }
        self._save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            this.update(cx, |this, cx| this.save_text(cx)).ok();
        }));
    }

    /// Writes the text to `file` now, ending any pending debounce.
    pub fn save_text(&mut self, cx: &App) {
        let Some(file) = &self.file else {
            return;
        };
        self._save = None;
        if let Err(e) = crate::persist::write_atomic(file, self.text(cx).as_bytes()) {
            eprintln!("Couldn't save {}: {e}", file.display());
        }
    }

    /// Deletes `file` and stops autosaving, as closing the console does.
    pub fn discard_file(&mut self) {
        self._save = None;
        if let Some(file) = self.file.take() {
            _ = std::fs::remove_file(file);
        }
    }

    /// Where this console runs.
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// Whether each statement commits on its own (not manual mode).
    pub fn autocommit(&self) -> bool {
        !self.manual
    }

    /// What the status bar shows for this console.
    pub fn mode(&self) -> &'static str {
        match (self.manual, self.in_transaction) {
            (_, true) => "TX",
            (true, false) => "MANUAL",
            (false, false) => "AUTO-COMMIT",
        }
    }

    /// Switches mode; leaving manual with a transaction open asks whether to commit or roll it back.
    pub fn set_manual(&mut self, manual: bool, window: &mut Window, cx: &mut Context<Self>) {
        if manual || !self.in_transaction {
            self.manual = manual;
            cx.notify();
            return;
        }
        let console = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let end = |id: &'static str, label: &'static str, sql: &'static str| {
                let console = console.clone();
                Button::new(id)
                    .debug_selector(move || id.into())
                    .small()
                    .label(label)
                    .on_click(move |_, window, cx| {
                        console.update(cx, |c, cx| {
                            c.manual = false;
                            c.end_transaction(sql, cx);
                        });
                        window.close_dialog(cx);
                    })
            };
            dialog
                .title(
                    div()
                        .debug_selector(|| "tx-switch-title".into())
                        .child("Commit or roll back the open transaction?"),
                )
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1())
                        .child(
                            Button::new("tx-switch-cancel")
                                .debug_selector(|| "tx-switch-cancel".into())
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(end("tx-switch-rollback", "Roll back", "ROLLBACK"))
                        .child(end("tx-switch-commit", "Commit", "COMMIT").primary()),
                )
        });
    }

    /// Ends the open transaction with `sql`, `COMMIT` or `ROLLBACK`.
    pub fn end_transaction(&mut self, sql: &'static str, cx: &mut Context<Self>) {
        let Some(session) = self
            .session
            .clone()
            .filter(|_| self.in_transaction && !self.running)
        else {
            return;
        };
        self.running = true;
        cx.notify();
        let open = self.target.driver.open(&session, sql.into(), false);
        self._run = Some(cx.spawn(async move |this, cx| {
            let result = open.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.in_transaction = false,
                    Err(error) => _ = this.failed(error, 0, Duration::ZERO, cx),
                }
                this.finish(None, cx);
            })
            .ok();
        }));
    }

    /// The selection's statements, else the one under the cursor.
    fn run_statement(&mut self, _: &RunStatement, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        let text = editor.value();
        let selected = editor.selected_range();
        let statements = if selected.is_empty() {
            sql::at_cursor(&text, editor.cursor())
                .map(|r| (r.start, text[r].to_string()))
                .into_iter()
                .collect()
        } else {
            let start = selected.start;
            statements(&text[selected])
                .into_iter()
                .map(|(offset, sql)| (start + offset, sql))
                .collect()
        };
        self.run(statements, window, cx);
    }

    fn run_script(&mut self, _: &RunScript, window: &mut Window, cx: &mut Context<Self>) {
        let statements = statements(&self.editor.read(cx).value());
        self.run(statements, window, cx);
    }

    fn rerun(&mut self, _: &Rerun, window: &mut Window, cx: &mut Context<Self>) {
        self.run_again(window, cx);
    }

    /// Stops the run: its running statement fails as cancelled, which ends the script.
    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if !self.running {
            cx.propagate();
            return;
        }
        let Some(session) = self.session.clone().filter(|s| !s.is_closed()) else {
            self._run = None;
            self.finish(Some("Cancelled".into()), cx);
            return;
        };
        cx.background_spawn(self.target.driver.cancel(&session))
            .detach();
    }

    /// Tab with the completion menu open accepts its item, as Enter does; else Tab indents.
    fn accept_completion(
        &mut self,
        _: &AcceptCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor.read(cx).completion_menu_state().open {
            cx.propagate();
            return;
        }
        let enter = gpui_kit::base::input::Enter {
            secondary: false,
            shift: false,
        };
        window.dispatch_action(Box::new(enter), cx);
    }

    /// Runs `statements`, first asking when any of them writes to a production connection.
    fn run(
        &mut self,
        statements: Vec<(usize, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let writes: Vec<&String> = statements
            .iter()
            .map(|(_, sql)| sql)
            .filter(|sql| !sql::is_read_only(sql))
            .collect();
        match (self.production(cx), writes.first()) {
            (Some(name), Some(first)) => {
                let first = (*first).clone();
                self.confirm_production(name, writes.len(), first, statements, window, cx)
            }
            _ => self.execute(statements, cx),
        }
    }

    /// The connection's name when it is a production one.
    fn production(&self, cx: &App) -> Option<String> {
        let id = self.target.connection?;
        let workspace = self.target.workspace.read(cx);
        let conn = workspace.connections.iter().find(|c| c.id == id)?;
        conn.is_production.then(|| conn.name.clone())
    }

    /// Asks before running `n` writing statements on production; only "Run on production" runs them.
    fn confirm_production(
        &self,
        name: String,
        n: usize,
        first: String,
        statements: Vec<(usize, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let console = cx.entity();
        let noun = if n == 1 { "statement" } else { "statements" };
        let title = format!("Run {n} writing {noun} on {name}?");
        window.open_dialog(cx, move |dialog, _, cx| {
            let title = title.clone();
            let (console, statements) = (console.clone(), statements.clone());
            dialog
                .title(
                    h_flex()
                        .gap(theme::space_8())
                        .debug_selector({
                            let title = title.clone();
                            move || format!("prod-run-title {title}")
                        })
                        .child(theme::prod_badge(cx.theme()))
                        .child(title),
                )
                .child(
                    div()
                        .debug_selector(|| "prod-run-preview".into())
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(theme::ui_sm())
                        .text_color(theme::text_secondary(cx.theme()))
                        .child(first.clone()),
                )
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1())
                        .child(
                            Button::new("prod-run-cancel")
                                .debug_selector(|| "prod-run-cancel".into())
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("prod-run-confirm")
                                .debug_selector(|| "prod-run-confirm".into())
                                .danger()
                                .small()
                                .label("Run on production")
                                .on_click(move |_, window, cx| {
                                    let statements = statements.clone();
                                    console.update(cx, |c, cx| c.execute(statements, cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    /// Runs `statements` one after another, stopping at the first error. Replaces the last run's output.
    fn execute(&mut self, statements: Vec<(usize, String)>, cx: &mut Context<Self>) {
        if statements.is_empty() {
            return;
        }
        self.last = statements.clone();
        (self.results, self.active_result, self.messages) = (Vec::new(), 0, Vec::new());
        self.mark_error(None, "", cx);
        self._fetch = None;
        cx.notify();
        let Some(params) = self.params(cx) else {
            self.messages.push("This console has no connection".into());
            return;
        };
        self.running = true;
        let session = self.session.clone().filter(|s| !s.is_closed());
        // A dropped session took its transaction with it; a new one must BEGIN again.
        if session.is_none() && std::mem::take(&mut self.in_transaction) {
            self.messages
                .push("Connection lost · the open transaction was rolled back".into());
        }
        let (begin, autocommit) = (self.manual && !self.in_transaction, !self.manual);
        let driver = self.target.driver.clone();
        let schema = self.target.schema.clone();
        self._run = Some(cx.spawn(async move |this, cx| {
            let session = match session {
                Some(session) => session,
                None => match connect(&*driver, &params, schema).await {
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
            if begin {
                let began = driver.open(&session, "BEGIN".into(), false).await;
                if !this
                    .update(cx, |this, cx| this.began(began, cx))
                    .unwrap_or(false)
                {
                    return;
                }
            }
            for (offset, sql) in statements {
                let (at, started) = (SystemTime::now(), Instant::now());
                let result = run_one(&*driver, &session, sql.clone(), autocommit).await;
                let duration = started.elapsed();
                let step = Step {
                    sql,
                    offset,
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

    /// Runs `sql` again on a new session set up like this console's, for Export All rows.
    fn export_source(&self, sql: String, cx: &App) -> Option<grid::Rerun> {
        let params = self.params(cx)?;
        let (driver, schema) = (self.target.driver.clone(), self.target.schema.clone());
        let on = driver.clone();
        let connect = move || -> Reply<Session> {
            let (driver, params, schema) = (on.clone(), params.clone(), schema.clone());
            Box::pin(async move { connect(&*driver, &params, schema).await })
        };
        Some(grid::Rerun {
            driver,
            sql,
            connect: Rc::new(connect),
        })
    }

    fn params(&self, cx: &App) -> Option<ConnectionParams> {
        let id = self.target.connection?;
        let workspace = self.target.workspace.read(cx);
        let conn = workspace.connections.iter().find(|c| c.id == id)?;
        Some(explorer::on_database(&conn.params, &self.target.database))
    }

    /// Marks the transaction open, or reports why BEGIN failed and ends the run; whether it began.
    fn began(&mut self, began: Result<Opened, QueryError>, cx: &mut Context<Self>) -> bool {
        if let Err(error) = began {
            self.failed(error, 0, Duration::ZERO, cx);
            self.finish(None, cx);
            return false;
        }
        self.in_transaction = true;
        cx.notify();
        true
    }

    /// Adds `step`'s result tab or message and logs it; false when it failed.
    fn show(&mut self, step: Step, cx: &mut Context<Self>) -> bool {
        let Some(connection_id) = self.target.connection else {
            return false;
        };
        if completion::is_ddl(&step.sql) {
            let schema = self.target.schema.clone();
            let path = [connection_id.0.to_string(), self.target.database.clone()];
            let path: Vec<String> = path.into_iter().chain(schema).collect();
            completion::invalidate(&path, cx);
        }
        // Each `open` closes the session's previous result.
        for result in &mut self.results {
            result.open = false;
        }
        let (rows, status) = match step.result {
            Ok(Ran::Rows(rs)) => {
                let n = rs.rows.len() as u64;
                let rerun = self.export_source(step.sql.clone(), cx);
                self.results.push(Paged::new(rs, rerun, cx));
                (Some(n), QueryStatus::Ok)
            }
            Ok(Ran::Affected(n)) => {
                self.messages.push(format!("{n} rows affected"));
                (Some(n), QueryStatus::Ok)
            }
            Err(error) => (None, self.failed(error, step.offset, step.duration, cx)),
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

    /// Reports a failed statement starting at `offset` in the editor text; the log status.
    fn failed(
        &mut self,
        error: QueryError,
        offset: usize,
        duration: Duration,
        cx: &mut Context<Self>,
    ) -> QueryStatus {
        match error.kind {
            QueryErrorKind::Cancelled => {
                let ms = duration.as_millis();
                self.messages.push(format!("Cancelled after {ms} ms"));
                return QueryStatus::Cancelled;
            }
            QueryErrorKind::Disconnected => {
                (self.session, self.in_transaction) = (None, false);
                self.messages
                    .push("Connection lost · Run again to reconnect".into());
            }
            QueryErrorKind::Server => {
                let at = error.position.map(|p| offset + p);
                let message = self.mark_error(at, &error.message, cx);
                self.messages.push(message);
            }
        }
        QueryStatus::Err(error.message)
    }

    /// Replaces the editor's diagnostics with `message` over the token at byte `at`, if given.
    /// Returns the message to show, with the line and column when `at` is in the text.
    fn mark_error(&self, at: Option<usize>, message: &str, cx: &mut Context<Self>) -> String {
        self.editor.update(cx, |editor, cx| {
            let text = editor.text().clone();
            let diagnostics = editor
                .diagnostics_mut()
                .expect("the console is a code editor");
            diagnostics.clear();
            cx.notify();
            let full = text.to_string();
            let Some(at) = at.filter(|&at| full.get(..at).is_some()) else {
                return message.to_string();
            };
            let (line, col) = sql::line_col(&full, at);
            let token = text.word_range(at).unwrap_or(at..(at + 1).min(text.len()));
            let range = text.offset_to_position(token.start)..text.offset_to_position(token.end);
            diagnostics.push(
                Diagnostic::new(range, message.to_string())
                    .with_severity(DiagnosticSeverity::Error),
            );
            format!("Error at line {line}, col {col}: {message}")
        })
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
    pub fn run_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(self.last.clone(), window, cx);
    }

    /// Rolls back an open transaction and closes the session's open result, as when the console
    /// tab closes; a later run reconnects.
    pub fn close_session(&mut self, cx: &mut Context<Self>) {
        (self._run, self._fetch, self.running) = (None, None, false);
        for result in &mut self.results {
            (result.open, result.fetching) = (false, false);
        }
        let in_transaction = std::mem::take(&mut self.in_transaction);
        let Some(session) = self.session.take() else {
            return;
        };
        let driver = &self.target.driver;
        let rollback = in_transaction.then(|| driver.open(&session, "ROLLBACK".into(), false));
        let close = driver.close(&session);
        cx.background_spawn(async move {
            if let Some(rollback) = rollback {
                rollback.await.ok();
            }
            close.await.ok();
        })
        .detach();
    }
}

fn grid(rs: ResultSet, rerun: Option<grid::Rerun>, cx: &mut App) -> Entity<grid::Grid> {
    cx.new(|cx| {
        let mut grid = grid::Grid::new(Rc::new(rs), cx);
        grid.rerun = rerun;
        grid
    })
}

/// A session on `params`, its `search_path` set to `schema` when there is one.
async fn connect(
    driver: &dyn Driver,
    params: &ConnectionParams,
    schema: Option<String>,
) -> Result<Session, String> {
    let session = driver.connect(params).await?;
    if let Some(schema) = schema {
        let set = driver.open(&session, search_path(&schema), false).await;
        set.map_err(|error| error.message)?;
    }
    Ok(session)
}

fn search_path(schema: &str) -> String {
    format!("SET search_path TO {}", ddl::ident(schema))
}

/// Every statement of `text`, with its byte offset.
fn statements(text: &str) -> Vec<(usize, String)> {
    sql::split(text)
        .into_iter()
        .map(|r| (r.start, text[r].to_string()))
        .collect()
}

/// Opens `sql`; a row-returning statement's first `PAGE` rows are read.
async fn run_one(
    driver: &dyn Driver,
    session: &Session,
    sql: String,
    autocommit: bool,
) -> Result<Ran, QueryError> {
    match driver.open(session, sql, autocommit).await? {
        Opened::Affected(n) => Ok(Ran::Affected(n)),
        Opened::Rows(columns) => {
            let rows = driver.fetch(session, PAGE).await?;
            Ok(Ran::Rows(ResultSet { columns, rows }))
        }
    }
}

impl Console {
    /// `database › schema`, as the tab's origin shows it.
    pub fn tail(&self) -> String {
        let schema = self.target.schema.as_deref().unwrap_or("");
        tabs::join_path(&self.target.database, schema).to_string()
    }

    /// The Postgres connection's params on the console's database; SQLite has no context to pick.
    fn pg_params(&self, cx: &App) -> Option<ConnectionParams> {
        self.params(cx)
            .filter(|p| matches!(p, ConnectionParams::Postgres { .. }))
    }

    /// Lists the databases and schemas the toolbar offers, connecting first when there is no session.
    pub fn load_context(&mut self, cx: &mut Context<Self>) {
        let Some(params) = self.pg_params(cx).filter(|_| !self.running) else {
            return;
        };
        let session = self.session.clone().filter(|s| !s.is_closed());
        let (driver, schema) = (self.target.driver.clone(), self.target.schema.clone());
        self._context = Some(cx.spawn(async move |this, cx| {
            let session = match session {
                Some(session) => session,
                None => match connect(&*driver, &params, schema).await {
                    Ok(session) => session,
                    Err(reason) => {
                        this.update(cx, |this, cx| this.finish(Some(reason), cx))
                            .ok();
                        return;
                    }
                },
            };
            this.update(cx, |this, _| {
                this.session.get_or_insert_with(|| session.clone());
            })
            .ok();
            let (databases, schemas) = (driver.databases(&session), driver.schemas(&session));
            let (databases, schemas) = (databases.await, schemas.await);
            this.update(cx, |this, cx| {
                (this.databases, this.schemas) =
                    (databases.unwrap_or_default(), schemas.unwrap_or_default());
                cx.notify();
            })
            .ok();
        }));
    }

    /// Points the session's `search_path` at `schema`; without a session the next connect does.
    pub fn pick_schema(&mut self, schema: String, cx: &mut Context<Self>) {
        // A SET inside a transaction is undone by its rollback, leaving the toolbar wrong.
        if self.running || self.in_transaction {
            return;
        }
        let Some(session) = self.session.clone().filter(|s| !s.is_closed()) else {
            self.target.schema = Some(schema);
            cx.emit(ConsoleEvent::Moved);
            cx.notify();
            return;
        };
        self.running = true;
        cx.notify();
        let set = self
            .target
            .driver
            .open(&session, search_path(&schema), false);
        self._run = Some(cx.spawn(async move |this, cx| {
            let result = set.await;
            this.update(cx, |this, cx| {
                this.running = false;
                match result {
                    Ok(_) => {
                        this.target.schema = Some(schema);
                        cx.emit(ConsoleEvent::Moved);
                    }
                    Err(error) => _ = this.failed(error, 0, Duration::ZERO, cx),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Moves the console to `database` on a new session, first asking when that would roll back
    /// an open transaction.
    pub fn pick_database(&mut self, database: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.running || database == self.target.database {
            return;
        }
        if !self.in_transaction {
            self.switch_database(database, cx);
            return;
        }
        let console = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let (console, database) = (console.clone(), database.clone());
            dialog
                .title(
                    div()
                        .debug_selector(|| "db-switch-title".into())
                        .child("Roll back and switch database?"),
                )
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1())
                        .child(
                            Button::new("db-switch-cancel")
                                .debug_selector(|| "db-switch-cancel".into())
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("db-switch-confirm")
                                .debug_selector(|| "db-switch-confirm".into())
                                .danger()
                                .small()
                                .label("Roll back and switch")
                                .on_click(move |_, window, cx| {
                                    let database = database.clone();
                                    console.update(cx, |c, cx| c.switch_database(database, cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    fn switch_database(&mut self, database: String, cx: &mut Context<Self>) {
        self.close_session(cx);
        (self.target.database, self.target.schema) = (database, None);
        self.schemas.clear();
        cx.emit(ConsoleEvent::Moved);
        cx.notify();
        self.load_context(cx);
    }

    /// `connection ▸ database ▸ schema`; only the connection on SQLite.
    fn context(&self, cx: &mut Context<Self>) -> Option<Div> {
        let id = self.target.connection?;
        let name = self
            .target
            .workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| c.id == id)?
            .name
            .clone();
        let crumbs = h_flex()
            .gap(theme::space_4())
            .text_size(theme::ui_sm())
            .text_color(theme::text_secondary(cx.theme()))
            .child(name);
        if self.pg_params(cx).is_none() {
            return Some(crumbs);
        }
        let console = cx.entity();
        let pick = |id: &'static str, current: String, items: &[String], on_database: bool| {
            let (console, items) = (console.clone(), items.to_vec());
            let label = if current.is_empty() {
                "Schema".to_string()
            } else {
                current.clone()
            };
            Button::new(id)
                .ghost()
                .xsmall()
                .label(label)
                .dropdown_caret(true)
                .disabled(self.running || (!on_database && self.in_transaction))
                .when(!on_database && self.in_transaction, |b| {
                    b.tooltip("Commit or roll back to change the schema")
                })
                .dropdown_menu(move |menu, _, cx| {
                    if items.is_empty() {
                        console.update(cx, |c, cx| c.load_context(cx));
                        return menu.item(PopupMenuItem::new("Loading…").disabled(true));
                    }
                    items.iter().fold(menu, |menu, item| {
                        let (console, item) = (console.clone(), item.clone());
                        menu.item(
                            PopupMenuItem::new(item.clone())
                                .checked(item == current)
                                .on_click(move |_, window, cx| {
                                    let item = item.clone();
                                    console.update(cx, |c, cx| match on_database {
                                        true => c.pick_database(item, window, cx),
                                        false => c.pick_schema(item, cx),
                                    });
                                }),
                        )
                    })
                })
        };
        let schema = self.target.schema.clone().unwrap_or_default();
        Some(
            crumbs
                .child("▸")
                .child(pick(
                    "console-database",
                    self.target.database.clone(),
                    &self.databases,
                    true,
                ))
                .child("▸")
                .child(pick("console-schema", schema, &self.schemas, false)),
        )
    }

    fn mode_toggle(&self, cx: &mut Context<Self>) -> Button {
        let manual = self.manual;
        Button::new("console-mode")
            .ghost()
            .xsmall()
            .label(if manual { "Manual" } else { "Auto-commit" })
            .disabled(self.running)
            .tooltip("Switch between auto-commit and manual transactions")
            .on_click(cx.listener(move |this, _, window, cx| this.set_manual(!manual, window, cx)))
    }

    fn end_button(
        &self,
        label: &'static str,
        sql: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(label)
            .ghost()
            .xsmall()
            .label(label)
            .disabled(!enabled)
            .on_click(cx.listener(move |this, _, _, cx| this.end_transaction(sql, cx)))
    }
}

impl Render for Console {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let run =
            match self.running {
                true => Button::new("console-cancel")
                    .icon(Lucide::Square)
                    .label("Cancel")
                    .tooltip_with_action("Cancel", &Cancel, None)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx))),
                false => Button::new("console-run")
                    .label("Run")
                    .tooltip_with_action("Run statement", &RunStatement, None)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_statement(&RunStatement, window, cx)
                    })),
            };
        let toolbar = h_flex()
            .flex_none()
            .h(theme::tab())
            .px(theme::space_8())
            .border_b_1()
            .border_color(cx.theme().border)
            .gap(theme::space_4())
            .children(self.context(cx))
            .child(run.ghost().xsmall())
            .child(self.mode_toggle(cx))
            .when(self.manual, |d| {
                let enabled = self.in_transaction && !self.running;
                d.child(self.end_button("Commit", "COMMIT", enabled, cx))
                    .child(self.end_button("Rollback", "ROLLBACK", enabled, cx))
            });
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::run_statement))
            .on_action(cx.listener(Self::run_script))
            .on_action(cx.listener(Self::rerun))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::accept_completion))
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
