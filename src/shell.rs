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
    /// Demo mode alone offers the component gallery.
    mode: Mode,
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
                shell
                    .bottom_panel
                    .update(cx, |p, cx| p.persist_log(root, cx));
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
