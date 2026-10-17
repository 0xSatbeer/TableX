use gpui_kit::component::input::InputState;
use gpui_kit::*;

use crate::ui::{console, explorer, grid, panel, statusbar, tabs};
use crate::{RestorePanel, db, mock, model, theme};
use std::rc::Rc;

mod actions;
mod appearance;
mod layout;
mod render;
mod start;
mod tab;
mod tab_bar;
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

/// How the app starts. `Demo` (`--demo`) loads the sample workspace, tabs and query log and
/// reads every connection through the mock driver; `Normal` starts empty on the real drivers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mode {
    Normal,
    Demo,
}

impl Mode {
    /// The mode for the command-line `args` after the program name. Any other argument is ignored.
    pub(crate) fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        if args.into_iter().any(|a| a == "--demo") {
            Mode::Demo
        } else {
            Mode::Normal
        }
    }
}

pub(crate) struct Shell {
    /// The one copy of the workspace's connections; explorer, panel and tabs follow it.
    workspace: Entity<model::Workspace>,
    /// What new consoles run on.
    driver: std::sync::Arc<dyn db::Driver>,
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
    /// Problems last notified for the user theme dir (`<ConfigDir>/themes`).
    user_theme_errors: Vec<String>,
    /// Overflow list: open state and its search field (shown above `tabs::SEARCH_MIN_TABS` tabs).
    overflow_open: bool,
    overflow_search: Entity<InputState>,
    /// The console editor's mono font size, before UI scale (Ctrl+= / Ctrl+- / Ctrl+0).
    console_mono: Pixels,
    _appearance: Subscription,
}

/// Key context of the shell while the bottom panel is maximized, so Esc restores it.
const PANEL_MAXIMIZED_CONTEXT: &str = "PanelMaximized";

/// Key context of the overflow list's content.
const OVERFLOW_CONTEXT: &str = "OverflowSearch";

/// `Popover` binds space to Confirm (closing it), which would swallow spaces typed in the search
/// field; a no-op binding in the content's own, deeper context lets the field type them.
pub(crate) fn bind_overflow_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("space", NoAction, Some(OVERFLOW_CONTEXT))]);
}

/// Zoom the focused grid's or editor's mono font; with neither focused nothing handles them.
pub(crate) fn bind_zoom_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-=", theme::ZoomIn, None),
        KeyBinding::new("ctrl--", theme::ZoomOut, None),
        KeyBinding::new("ctrl-0", theme::ZoomReset, None),
    ]);
}

pub(crate) fn bind_panel_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        RestorePanel,
        Some(PANEL_MAXIMIZED_CONTEXT),
    )]);
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
            }
            if let Some(dir) = user_themes {
                shell.watch_user_themes(dir, window, cx);
            }
            shell
        })
    })
}

/// Sets the mock round-trip time for every connect and load from `value`
/// (`QUERYX_MOCK_LATENCY_MS`), to see loading states.
pub(crate) fn apply_mock_latency(mode: Mode, value: Option<String>) {
    if let (Mode::Demo, Some(ms)) = (mode, value) {
        match ms.parse() {
            Ok(ms) => mock::set_latency(ms),
            Err(_) => eprintln!(
                "QUERYX_MOCK_LATENCY_MS={ms:?} is not a whole number of milliseconds; using 0"
            ),
        }
    }
}

#[cfg(test)]
mod tests;
