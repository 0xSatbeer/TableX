//! Bottom panel (QX-014): Results / Logs / Messages.

mod filter;
mod render;
mod row;
pub(crate) mod sql;

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::model::{ConnectionId, QueryLogEntry, Workspace};
use crate::ui::console::Console;

pub use filter::LogFilter;
use filter::filter_rows;

const TABS: [&str; 3] = ["Results", "Logs", "Messages"];
const RESULTS: usize = 0;
const LOGS: usize = 1;
const MESSAGES: usize = 2;
/// Time column width in `SPACE_8` units; wide enough for the "Time (UTC)" header.
const TIME_W: f32 = 10.;

pub struct BottomPanel {
    /// Set by the shell: the panel covers the editor area, so the header offers restore.
    pub maximized: bool,
    /// Set by the shell: the active console, whose results and messages show here; other tabs hide Results.
    pub console: Option<Entity<Console>>,
    tab: usize,
    log: VecDeque<QueryLogEntry>,
    /// Normal mode's `ConfigDir` root, whose `logs/queries.jsonl` gets every pushed entry.
    log_root: Option<PathBuf>,
    filter: LogFilter,
    search: Entity<InputState>,
    /// Indexes into `log` that pass the filters.
    rows: Vec<usize>,
    list: ListState,
    /// Log indexes of rows shown expanded.
    expanded: BTreeSet<usize>,
    /// The shared workspace, for each log row's connection name, color and production flag.
    workspace: Entity<Workspace>,
    focus: FocusHandle,
}

impl BottomPanel {
    /// The panel over `workspace`, its Logs starting with `log` (oldest first).
    pub fn new(
        workspace: Entity<Workspace>,
        log: Vec<QueryLogEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter SQL…"));
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.refilter(cx);
                cx.notify();
            }
        })
        .detach();
        cx.observe(&workspace, |this, workspace, cx| {
            // A deleted connection takes its log rows with it.
            let ids: Vec<ConnectionId> = workspace
                .read(cx)
                .connections
                .iter()
                .map(|c| c.id)
                .collect();
            let filtered = this.filter.connections.len();
            this.filter
                .connections
                .retain(|id| ids.contains(&ConnectionId(*id)));
            let stale_log = this.log.iter().any(|e| !ids.contains(&e.connection_id));
            if stale_log {
                this.log.retain(|e| ids.contains(&e.connection_id));
                this.expanded.clear();
            }
            if stale_log || this.filter.connections.len() != filtered {
                this.refilter(cx);
            }
            cx.notify();
        })
        .detach();
        let log = log.into_iter().fold(VecDeque::new(), |mut log, entry| {
            push_log(&mut log, entry);
            log
        });
        let filter = LogFilter::default();
        let rows = filter_rows(&log, &filter);
        Self {
            maximized: false,
            console: None,
            tab: RESULTS,
            log,
            log_root: None,
            filter,
            search,
            list: ListState::new(rows.len(), ListAlignment::Top, px(240.)),
            rows,
            expanded: BTreeSet::new(),
            workspace,
            focus: cx.focus_handle(),
        }
    }

    /// The (name, color index, production) a log row shows for `id`.
    pub fn connection(&self, id: ConnectionId, cx: &App) -> (String, u8, bool) {
        let conn = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| c.id == id);
        conn.map(|c| (c.name.clone(), c.color, c.is_production))
            .unwrap_or_default()
    }

    /// Logs `entry` as the newest row.
    pub fn push(&mut self, entry: QueryLogEntry, cx: &mut Context<Self>) {
        if let Some(root) = &self.log_root
            && let Err(e) = crate::persist::append_query_log(root, &entry)
        {
            eprintln!("could not append to the query log: {e}");
        }
        push_log(&mut self.log, entry);
        self.refilter(cx);
        cx.notify();
    }

    /// Loads the query log saved under `root` ahead of the rows logged so far, and appends every
    /// later entry there.
    pub fn persist_log(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let saved = crate::persist::load_query_log(&root);
        let mut log = VecDeque::new();
        saved
            .into_iter()
            .chain(self.log.drain(..))
            .for_each(|e| push_log(&mut log, e));
        self.log = log;
        self.log_root = Some(root);
        self.expanded.clear();
        self.refilter(cx);
        cx.notify();
    }

    /// Shows what a console run left: Results when it returned rows, else Messages.
    pub fn show_run(&mut self, results: bool, cx: &mut Context<Self>) {
        self.tab = if results { RESULTS } else { MESSAGES };
        cx.notify();
    }

    #[cfg(test)]
    pub fn logged(&self) -> Vec<QueryLogEntry> {
        self.log.iter().cloned().collect()
    }

    /// The connections the query log has rows for.
    #[cfg(test)]
    pub fn logged_connections(&self) -> std::collections::HashSet<ConnectionId> {
        self.log.iter().map(|e| e.connection_id).collect()
    }

    #[cfg(test)]
    pub fn show_logs(&mut self) {
        self.tab = LOGS;
    }

    #[cfg(test)]
    pub fn search_field(&self) -> Entity<InputState> {
        self.search.clone()
    }

    #[cfg(test)]
    pub fn set_connection_filter(&mut self, ids: &[ConnectionId], cx: &App) {
        self.filter.connections = ids.iter().map(|id| id.0).collect();
        self.refilter(cx);
    }

    /// The connections of the Logs rows the filters let through.
    #[cfg(test)]
    pub fn shown_connections(&self) -> std::collections::HashSet<ConnectionId> {
        self.rows
            .iter()
            .map(|&ix| self.log[ix].connection_id)
            .collect()
    }

    fn refilter(&mut self, cx: &App) {
        self.filter.text = self.search.read(cx).value().to_string();
        self.rows = filter_rows(&self.log, &self.filter);
        self.list.reset(self.rows.len());
    }

    /// The connection filter's button label: "All connections", the one chosen name, or "<n> connections".
    pub fn connection_filter_label(&self, cx: &App) -> String {
        let connections = &self.workspace.read(cx).connections;
        match self.filter.connections.len() {
            0 => "All connections".to_string(),
            1 => connections
                .iter()
                .find(|c| self.filter.connections.contains(&c.id.0))
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            n => format!("{n} connections"),
        }
    }
}

/// Logs keep the newest this many entries.
pub const LOG_CAP: usize = 10_000;

/// Appends the newest entry (the list shows the last one first); past `LOG_CAP` the oldest drops.
fn push_log(log: &mut VecDeque<QueryLogEntry>, entry: QueryLogEntry) {
    log.push_back(entry);
    if log.len() > LOG_CAP {
        log.pop_front();
    }
}

impl Focusable for BottomPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[cfg(test)]
mod tests;
