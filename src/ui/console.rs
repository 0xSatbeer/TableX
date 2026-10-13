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
    ConnectionId, ConnectionParams, QueryError, QueryLogEntry, QuerySource, QueryStatus, ResultSet,
    Workspace,
};
use crate::ui::{explorer, grid};
use crate::{sql, theme};

actions!(console, [RunStatement, RunScript, Rerun]);

const CONTEXT: &str = "Console";
/// Rows a result tab shows; paging past them comes later.
const FETCH: usize = 500;

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
    pub results: Vec<Entity<grid::Grid>>,
    pub active_result: usize,
    pub messages: Vec<String>,
    /// Set by the shell: the editor's zoomed mono size.
    pub text_size: Pixels,
    _run: Option<Task<()>>,
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
            _run: None,
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
        self.run(self.last.clone(), cx);
    }

    /// Runs `statements` one after another, stopping at the first error. Replaces the last run's output.
    fn run(&mut self, statements: Vec<String>, cx: &mut Context<Self>) {
        if statements.is_empty() {
            return;
        }
        self.last = statements.clone();
        (self.results, self.active_result, self.messages) = (Vec::new(), 0, Vec::new());
        cx.notify();
        let Some(params) = self.params(cx) else {
            self.messages.push("This console has no connection".into());
            return;
        };
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
        let (rows, status) = match step.result {
            Ok(Ran::Rows(rs)) => {
                let n = rs.rows.len() as u64;
                self.results
                    .push(cx.new(|cx| grid::Grid::new(Rc::new(rs), cx)));
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
        self.messages.extend(error);
        cx.emit(ConsoleEvent::Finished);
        cx.notify();
    }
}

/// Every statement of `text`.
fn statements(text: &str) -> Vec<String> {
    sql::split(text)
        .into_iter()
        .map(|r| text[r].to_string())
        .collect()
}

/// Opens `sql`; a row-returning statement's first `FETCH` rows are read.
async fn run_one(driver: &dyn Driver, session: &Session, sql: String) -> Result<Ran, QueryError> {
    match driver.open(session, sql, true).await? {
        Opened::Affected(n) => Ok(Ran::Affected(n)),
        Opened::Rows(columns) => {
            let rows = driver.fetch(session, FETCH).await?;
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
