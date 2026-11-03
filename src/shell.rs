use gpui_kit::component::input::InputState;
use gpui_kit::*;

use crate::ui::{console, explorer, grid, panel, statusbar, tabs};
use crate::{db, mock, model, theme};
use std::rc::Rc;

mod actions;
mod appearance;
mod layout;
mod render;
mod session;
mod start;
mod tab;
mod tab_bar;
mod titlebar;
mod workspaces;

use layout::LayoutState;

enum TabContent {
    /// A live table or view also has its Structure and DDL sub-tabs.
    Grid(
        Entity<grid::Grid>,
        Option<Entity<crate::ui::structure::Structure>>,
    ),
    Console(Entity<console::Console>),
}

impl TabContent {
    /// Its connection was lost: tables and structures show Disconnected, consoles drop their
    /// sessions and stay editable.
    fn lost(&self, cx: &mut App) {
        match self {
            TabContent::Grid(grid, structure) => {
                grid.update(cx, |grid, cx| grid.set_lost(cx));
                if let Some(s) = structure {
                    s.update(cx, |s, cx| s.lost(cx));
                }
            }
            TabContent::Console(console) => console.update(cx, |c, cx| c.lost(cx)),
        }
    }

    /// Its connection is back: lost tables and structures read again.
    fn restored(&self, window: &mut Window, cx: &mut App) {
        if let TabContent::Grid(grid, structure) = self {
            grid.update(cx, |grid, cx| grid.restored(cx));
            if let Some(s) = structure {
                s.update(cx, |s, cx| s.restored(window, cx));
            }
        }
    }
}

/// How the app starts. `Demo` (`--demo`) loads the sample workspace, tabs and query log and
/// reads every connection through the mock driver; `Normal` starts empty on the real drivers.
/// `Bench` (`--bench-scenario <name>`) runs one perf scenario without reading or writing config.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mode {
    Normal,
    Demo,
    Bench(crate::bench::Scenario),
}

impl Mode {
    /// The mode for the command-line `args` after the program name. Any other argument is ignored;
    /// an unknown or missing bench scenario is an error.
    pub(crate) fn from_args(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let args: Vec<String> = args.into_iter().collect();
        if let Some(ix) = args.iter().position(|a| a == "--bench-scenario") {
            let name = args.get(ix + 1).map(String::as_str);
            return crate::bench::Scenario::parse(name).map(Mode::Bench);
        }
        Ok(match args.iter().any(|a| a == "--demo") {
            true => Mode::Demo,
            false => Mode::Normal,
        })
    }
}

pub(crate) struct Shell {
    /// Demo mode alone offers the component gallery.
    mode: Mode,
    /// The one copy of the workspace's connections; explorer, panel and tabs follow it.
    workspace: Entity<model::Workspace>,
    /// What new consoles run on.
    driver: std::sync::Arc<dyn db::Driver>,
    /// Every connection's sessions and status; tabs and the explorer read through it.
    sessions: Entity<crate::sessions::Sessions>,
    explorer: Entity<explorer::Explorer>,
    tabs: tabs::Tabs<TabContent>,
    tab_scroll: ScrollHandle,
    layout: LayoutState,
    status_bar: Entity<statusbar::StatusBar>,
    bottom_panel: Entity<panel::BottomPanel>,
    /// Keeps focus under the shell's key context when maximize unmounts the focused editor, so Esc reaches it.
    focus_handle: FocusHandle,
    gallery_visible: bool,
    /// Set when a tab is activated; the next render moves keyboard focus into its content.
    focus_content: bool,
    theme: theme::ThemeChoice,
    /// Themes loaded from the user theme dir (`<ConfigDir>/themes`), listed under the built-ins.
    user_themes: Vec<Rc<gpui_kit::component::theme::ThemeConfig>>,
    /// Normal mode's `workspaces.toml`; demo mode never reads or writes config.
    workspaces_file: Option<crate::persist::WorkspacesFile>,
    /// Normal mode's `ConfigDir` root, where `config.toml` is saved; demo mode never writes it.
    config_root: Option<std::path::PathBuf>,
    /// Normal mode's `ConfigDir` root, where `sessions/<workspace id>.toml` are saved.
    sessions_root: Option<std::path::PathBuf>,
    /// The session last saved or waiting to be, so an unchanged one isn't written again.
    session_saved: Option<crate::persist::SessionFile>,
    /// The pending debounced save; replacing it cancels the one before.
    _session_save: Option<Task<()>>,
    /// Each console tab's id (`consoles/<id>.sql`), by tab key; the largest handed out is `next_console`.
    console_ids: std::collections::HashMap<SharedString, u32>,
    next_console: u32,
    /// Problems last notified for the user theme dir (`<ConfigDir>/themes`).
    user_theme_errors: Vec<String>,
    /// Overflow list: open state and its search field (shown above `tabs::SEARCH_MIN_TABS` tabs).
    overflow_open: bool,
    overflow_search: Entity<InputState>,
    /// The console editor's mono font size, before UI scale (Ctrl+= / Ctrl+- / Ctrl+0).
    console_mono: Pixels,
    /// The command palette, while open.
    palette: Option<crate::ui::palette::Open>,
    /// The keyboard cheat sheet, while open.
    cheat_sheet: Option<crate::ui::cheat_sheet::Open>,
    /// Quick open (Ctrl+P), while open.
    quick_open: Option<crate::ui::quick_open::Open>,
    _appearance: Subscription,
}

/// Key context of the shell while the bottom panel is maximized, so Esc restores it.
pub(crate) const PANEL_MAXIMIZED_CONTEXT: &str = "PanelMaximized";

/// Key context of the overflow list's content.
pub(crate) const OVERFLOW_CONTEXT: &str = "OverflowSearch";

#[cfg(test)]
pub(crate) fn bind_overflow_keys(cx: &mut App) {
    crate::keymap::bind_defaults(cx, |_, context| context == Some(OVERFLOW_CONTEXT));
}

#[cfg(test)]
pub(crate) fn bind_zoom_keys(cx: &mut App) {
    crate::keymap::bind_defaults(cx, |action, _| action.starts_with("theme::"));
}

#[cfg(test)]
pub(crate) fn bind_panel_keys(cx: &mut App) {
    crate::keymap::bind_defaults(cx, |_, context| context == Some(PANEL_MAXIMIZED_CONTEXT));
}

/// Opens the main window and starts watching `user_themes` (the user theme dir), if any. Normal
/// mode loads and saves the workspaces and settings in `config` (the `ConfigDir` root), if any.
pub(crate) fn open_shell(
    mode: Mode,
    options: WindowOptions,
    user_themes: Option<std::path::PathBuf>,
    config: Option<std::path::PathBuf>,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<Shell>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let mut shell = Shell::new(mode, window, cx);
            if let (Mode::Normal, Some(root)) = (mode, config) {
                shell.persist_config(&root, window, cx);
                shell.persist_workspaces(&root, window, cx);
                shell.persist_session(&root, window, cx);
                // Logs live only in memory; drop the file older builds kept.
                _ = std::fs::remove_file(root.join("logs").join("queries.jsonl"));
            }
            if let Some(dir) = user_themes {
                shell.watch_user_themes(dir, window, cx);
            }
            shell
        })
    })
}

/// Sets the mock round-trip time for every connect and load from `value`
/// (`TABLEX_MOCK_LATENCY_MS`), to see loading states.
pub(crate) fn apply_mock_latency(mode: Mode, value: Option<String>) {
    if let (Mode::Demo, Some(ms)) = (mode, value) {
        match ms.parse() {
            Ok(ms) => mock::set_latency(ms),
            Err(_) => eprintln!(
                "TABLEX_MOCK_LATENCY_MS={ms:?} is not a whole number of milliseconds; using 0"
            ),
        }
    }
}

#[cfg(test)]
mod tests;
