//! Status bar (QX-018). Its own entity so its updates re-render only this bar.

use gpui_kit::component::input::EditorState;
use gpui_kit::*;

use crate::model::{Connection, ConnectionId, ConnectionParams, ConnectionStatus, Workspace};
use crate::ui::console::Console;
use crate::ui::grid::{Grid, SelectionKey};
use crate::ui::tabs::Origin;

mod aggregate;
mod render;

pub use aggregate::aggregate;

pub struct StatusBar {
    /// Where the active origin's connection status comes from.
    workspace: Entity<Workspace>,
    /// Started with `--demo`, where a tab without an origin shows sample data.
    demo: bool,
    /// Where the active tab's data comes from; `None` for no connection (sample data in the demo) or no tab.
    origin: Option<Origin>,
    /// Some tab is open, so a missing origin means sample data rather than nothing.
    has_tab: bool,
    /// 1-based line and column of the focused console editor, and its selection length in chars.
    cursor: Option<(u32, u32, usize)>,
    _editor: Option<Subscription>,
    /// The active console's mode (AUTO-COMMIT, MANUAL or TX); `None` off a console.
    mode: Option<&'static str>,
    _console: Option<Subscription>,
    /// Aggregate of the active grid's selection, and the selection it was computed for.
    selection: Option<String>,
    grid_focused: bool,
    selection_key: Option<SelectionKey>,
    _grid: Option<Subscription>,
    _workspace: Subscription,
}

/// Connect or Retry clicked for connection `.0`: connect it as the tree's Retry does.
pub struct Connect(pub ConnectionId);

impl EventEmitter<Connect> for StatusBar {}

/// What the bar shows for the active tab's connection.
struct Readout {
    /// A solid dot when Connected, else a ring.
    solid: bool,
    tip: String,
    text: Option<&'static str>,
    danger: bool,
    button: Option<&'static str>,
    /// The reason and what to check, on hovering the text.
    hint: Option<String>,
}

fn readout(conn: &Connection) -> Readout {
    let (solid, tip) = crate::ui::explorer::status_style(&conn.status);
    let (text, button, hint) = match &conn.status {
        ConnectionStatus::Connected => (None, None, None),
        ConnectionStatus::Connecting => (Some("Connecting…"), None, None),
        ConnectionStatus::Disconnected => (Some("Not connected"), Some("Connect"), None),
        ConnectionStatus::Error(reason) => {
            let check = match &conn.params {
                ConnectionParams::Postgres { host, port, .. } => {
                    format!("Check that {host}:{port} is reachable (VPN, firewall), then retry.")
                }
                ConnectionParams::Sqlite { .. } => {
                    "Check that the file exists and is readable.".into()
                }
            };
            (
                Some("Could not connect"),
                Some("Retry"),
                Some(format!("{reason}\n{check}")),
            )
        }
    };
    Readout {
        solid,
        tip,
        text,
        danger: matches!(conn.status, ConnectionStatus::Error(_)),
        button,
        hint,
    }
}

impl StatusBar {
    pub fn new(workspace: Entity<Workspace>, demo: bool, cx: &mut Context<Self>) -> Self {
        Self {
            workspace: workspace.clone(),
            demo,
            origin: None,
            has_tab: false,
            cursor: None,
            _editor: None,
            mode: None,
            _console: None,
            selection: None,
            grid_focused: false,
            selection_key: None,
            _grid: None,
            _workspace: cx.observe(&workspace, |_, _, cx| cx.notify()),
        }
    }

    pub fn set_origin(&mut self, origin: Option<Origin>, has_tab: bool, cx: &mut Context<Self>) {
        if (&self.origin, self.has_tab) != (&origin, has_tab) {
            (self.origin, self.has_tab) = (origin, has_tab);
            cx.notify();
        }
    }

    /// Follow `editor`'s cursor, or stop showing one when `None`.
    pub fn set_editor(&mut self, editor: Option<&Entity<EditorState>>, cx: &mut Context<Self>) {
        let position = |editor: &Entity<EditorState>, cx: &App| {
            let editor = editor.read(cx);
            let p = editor.cursor_position();
            (
                p.line + 1,
                p.character + 1,
                editor.selected_text().len_chars(),
            )
        };
        self._editor = editor.map(|editor| {
            // The editor notifies for more than cursor moves; re-render only when the readout changes.
            cx.observe(editor, move |this, editor, cx| {
                let cursor = Some(position(&editor, cx));
                if this.cursor != cursor {
                    this.cursor = cursor;
                    cx.notify();
                }
            })
        });
        self.cursor = editor.map(|editor| position(editor, cx));
        cx.notify();
    }

    /// Follow `console`'s transaction mode, or show none when `None`.
    pub fn set_console(&mut self, console: Option<&Entity<Console>>, cx: &mut Context<Self>) {
        self._console = console.map(|console| {
            cx.observe(console, |this, console, cx| {
                let mode = Some(console.read(cx).mode());
                if this.mode != mode {
                    this.mode = mode;
                    cx.notify();
                }
            })
        });
        self.mode = console.map(|console| console.read(cx).mode());
        cx.notify();
    }

    /// Follow `grid`'s selection aggregate, or hide it when `None`.
    pub fn set_grid(&mut self, grid: Option<&Entity<Grid>>, cx: &mut Context<Self>) {
        self.selection_key = None;
        self._grid = grid.map(|grid| {
            // The grid notifies on scroll too; re-aggregate only when the selection changes.
            cx.observe(grid, |this, grid, cx| this.sync_selection(&grid, cx))
        });
        match grid {
            Some(grid) => self.sync_selection(grid, cx),
            None => self.selection = None,
        }
        cx.notify();
    }

    /// The aggregate as drawn: only while the grid holds focus.
    pub fn aggregate_text(&self) -> Option<String> {
        self.selection.clone().filter(|_| self.grid_focused)
    }

    pub fn set_grid_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.grid_focused != focused {
            self.grid_focused = focused;
            cx.notify();
        }
    }

    fn sync_selection(&mut self, grid: &Entity<Grid>, cx: &mut Context<Self>) {
        let key = grid.read(cx).selection_key();
        if self.selection_key.as_ref() != Some(&key) {
            self.selection = grid.read(cx).selection_summary();
            self.selection_key = Some(key);
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests;
