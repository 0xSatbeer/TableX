use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::ui::{console, explorer, grid, panel, split, statusbar, structure, tabs};
use crate::{db, mock, model, theme};
use std::rc::Rc;

use super::{Mode, Shell, TabContent, layout::LayoutState};

impl Shell {
    pub(super) fn new(mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (workspace, driver): (_, std::sync::Arc<dyn db::Driver>) = match mode {
            Mode::Normal => (
                model::Workspace {
                    id: 1,
                    name: "Workspace".into(),
                    connections: Vec::new(),
                    id_floor: 0,
                },
                std::sync::Arc::new(db::Live::default()),
            ),
            Mode::Demo => {
                // The trees start unloaded, so the mock driver serves each level as it is expanded.
                let mut workspace = mock::workspaces().remove(0);
                workspace
                    .connections
                    .iter_mut()
                    .for_each(|c| c.databases = None);
                (
                    workspace,
                    std::sync::Arc::new(db::mock::Mock::new(cx.background_executor().clone())),
                )
            }
        };
        Self::with_driver(mode, workspace, driver, window, cx)
    }

    /// The Shell started in `mode` on `workspace`, whose explorer reads databases through `driver`.
    pub(super) fn with_driver(
        mode: Mode,
        workspace: model::Workspace,
        driver: std::sync::Arc<dyn db::Driver>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let demo = mode == Mode::Demo;
        let workspace = cx.new(|_| workspace);
        // The demo console runs against production, so the tint shows up.
        let console_conn = workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| demo && c.is_production)
            .map(|c| (tabs::Origin::of(c), c.id, c.params.database()));
        let explorer =
            cx.new(|cx| explorer::Explorer::new(workspace.clone(), driver.clone(), window, cx));
        cx.observe(&workspace, |this, workspace, cx| {
            this.tabs.refresh_origins(workspace.read(cx));
            let (origin, has_tab) = (
                this.tabs.active().and_then(|t| t.origin.clone()),
                this.tabs.active().is_some(),
            );
            this.status_bar
                .update(cx, |bar, cx| bar.set_origin(origin, has_tab, cx));
            this.update_move_targets(cx);
            cx.notify();
        })
        .detach();
        cx.subscribe(
            &explorer,
            |this, _, ev: &explorer::ConnectionDeleted, cx| this.close_connection_tabs(ev.id, cx),
        )
        .detach();
        cx.subscribe_in(
            &explorer,
            window,
            |this, _, ev: &explorer::MoveConnection, window, cx| {
                this.move_connection(ev.id, ev.to, window, cx)
            },
        )
        .detach();
        cx.subscribe(&explorer, move |this, _, ev: &explorer::OpenObject, cx| {
            let open = this.tabs.tabs.iter().any(|t| t.key == ev.key);
            let table = (!demo && !open)
                .then(|| this.explorer.update(cx, |e, cx| e.table(&ev.key, cx)))
                .flatten();
            let rows = match demo.then(|| mock::run_query(&format!("SELECT * FROM {}", ev.name))) {
                None => model::ResultSet {
                    columns: Vec::new(),
                    rows: Vec::new(),
                },
                Some(model::QueryOutcome::Rows(rs)) => rs,
                Some(_) => mock::edge_cases(),
            };
            let mut tab = match table {
                Some(table) => {
                    let grid = cx.new(|cx| grid::Grid::table(table.clone(), cx));
                    grid.update(cx, |grid, cx| grid.load(cx));
                    table_tab(ev.key.clone(), grid, table, cx)
                }
                None => grid_tab(ev.key.clone(), Rc::new(rows), cx),
            };
            (tab.title, tab.origin, tab.preview) =
                (ev.name.clone(), Some(ev.origin.clone()), ev.preview);
            tab.tooltip = Some(ev.path.clone());
            this.tabs.open(tab);
            this.tab_changed(cx);
            // A preview (single click, Space) leaves focus in the tree for the next arrow key.
            this.focus_content = !ev.preview;
        })
        .detach();
        cx.subscribe_in(
            &explorer,
            window,
            |this, _, ev: &explorer::OpenConsole, window, cx| {
                let key = console_key(ev.conn, &ev.database, ev.schema.as_deref());
                if let Some(ix) = this.tabs.tabs.iter().position(|t| t.key == key) {
                    this.tabs.activate(ix);
                } else {
                    let Some(conn) = this
                        .workspace
                        .read(cx)
                        .connections
                        .iter()
                        .find(|c| c.id == ev.conn)
                        .cloned()
                    else {
                        return;
                    };
                    let tab = this.connection_console(
                        &conn,
                        ev.database.clone(),
                        ev.schema.clone(),
                        "",
                        window,
                        cx,
                    );
                    this.tabs.open(tab);
                }
                this.tab_changed(cx);
            },
        )
        .detach();
        let status_bar =
            cx.new(|cx| statusbar::StatusBar::new(workspace.clone(), demo, window, cx));
        cx.subscribe(&status_bar, |this, _, statusbar::Connect(id), cx| {
            this.explorer
                .update(cx, |e, cx| e.connect_connection(*id, cx))
        })
        .detach();
        let overflow_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search tabs…"));
        cx.subscribe(
            &overflow_search,
            |this, search, ev: &InputEvent, cx| match ev {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => {
                    let query = search.read(cx).value();
                    if let Some(ix) = tabs::first_match(&this.tabs.tabs, &query) {
                        this.overflow_open = false;
                        this.tabs.activate(ix);
                        this.tab_changed(cx);
                    }
                }
                _ => {}
            },
        )
        .detach();
        let (origin, connection, database) = match console_conn {
            Some((origin, id, database)) => (Some(origin), Some(id), database),
            None => (None, None, String::new()),
        };
        let target = console::Target {
            workspace: workspace.clone(),
            driver: driver.clone(),
            connection,
            database,
            schema: None,
        };
        let console = console_tab(
            "console-1",
            "console-1",
            origin,
            target,
            if demo {
                "SELECT id, email\nFROM users\nWHERE created_at > now() - interval '7 days';\n"
            } else {
                ""
            },
            window,
            cx,
        );
        let tabs = match mode {
            Mode::Normal => vec![console],
            Mode::Demo => vec![
                grid_tab("big_events", Rc::new(grid::BigEvents::new()), cx),
                grid_tab("wide_table", Rc::new(mock::wide_table(2_000)), cx),
                grid_tab("edge_cases", Rc::new(mock::edge_cases()), cx),
                console,
            ],
        };
        let log = if demo {
            mock::mock_query_log(panel::LOG_CAP)
        } else {
            Vec::new()
        };
        let mut this = Self {
            overflow_open: false,
            overflow_search,
            workspace: workspace.clone(),
            driver,
            explorer,
            tab_scroll: ScrollHandle::new(),
            tabs: tabs::Tabs::new(tabs),
            layout: LayoutState {
                explorer_visible: true,
                bottom_visible: true,
                maximized_from: None,
                panel_maximized: false,
                explorer: split::EXPLORER,
                bottom: split::BOTTOM,
                dragging: None,
            },
            status_bar,
            bottom_panel: cx.new(|cx| panel::BottomPanel::new(workspace.clone(), log, window, cx)),
            focus_handle: cx.focus_handle(),
            gallery_visible: false,
            focus_content: false,
            theme: theme::ThemeChoice::default(),
            user_themes: Vec::new(),
            user_theme_errors: Vec::new(),
            workspaces_file: None,
            config_root: None,
            sessions_root: None,
            session_saved: None,
            _session_save: None,
            console_ids: std::collections::HashMap::new(),
            next_console: 0,
            console_mono: theme::MONO,
            palette: None,
            cheat_sheet: None,
            _appearance: cx.observe_window_appearance(window, |this, window, cx| {
                this.theme.apply(&this.user_themes, window, cx)
            }),
        };
        this.tab_changed(cx);
        this
    }
}

impl Shell {
    /// Shows the workspaces saved under `root` and saves every later change to them there.
    pub(super) fn persist_workspaces(
        &mut self,
        root: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mut file, notice) = crate::persist::WorkspacesFile::load(root);
        let active = file.active();
        self.workspaces_file = Some(file);
        self.workspace.update(cx, |ws, cx| {
            *ws = active;
            cx.notify();
        });
        cx.observe_in(&self.workspace, window, |this, _, window, cx| {
            this.save_workspaces(window, cx)
        })
        .detach();
        if let Some(notice) = notice {
            // Deferred: this runs while the window is being built, and notifications need its Root.
            cx.defer_in(window, move |_, window, cx| {
                use gpui_kit::component::{WindowExt as _, notification::Notification};
                window.push_notification(Notification::error(notice), cx);
            });
        }
    }

    /// Writes `workspaces.toml` when an add, edit or delete changed what it holds.
    fn save_workspaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        let Some(file) = self.workspaces_file.as_mut() else {
            return;
        };
        if let Err(e) = file.save(self.workspace.read(cx)) {
            let message = format!("Couldn't save {}: {e}", file.path.display());
            window.push_notification(Notification::error(message), cx);
        }
    }
}

impl Shell {
    /// A console tab on `conn`'s `database` (and `schema`), holding `text`.
    pub(super) fn connection_console(
        &mut self,
        conn: &model::Connection,
        database: String,
        schema: Option<String>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> tabs::EditorTab<TabContent> {
        let key = console_key(conn.id, &database, schema.as_deref());
        let tail = tabs::join_path(&database, schema.as_deref().unwrap_or(""));
        let origin = tabs::Origin {
            connection: conn.id,
            color: conn.color,
            production: conn.is_production,
            path: tabs::join_path(&conn.name, &tail),
            tail,
        };
        let title = format!("{database} — console");
        let target = console::Target {
            workspace: self.workspace.clone(),
            driver: self.driver.clone(),
            connection: Some(conn.id),
            database,
            schema,
        };
        console_tab(key, title, Some(origin), target, text, window, cx)
    }
}

/// The key of the console on `conn`'s `database` (and `schema`); opening it again activates it.
fn console_key(conn: model::ConnectionId, database: &str, schema: Option<&str>) -> SharedString {
    match schema {
        Some(schema) => format!("console:{}/{database}/{schema}", conn.0),
        None => format!("console:{}/{database}", conn.0),
    }
    .into()
}

/// A table tab keyed `key` showing `grid`, which reads `table`, with its Structure and DDL sub-tabs.
pub(super) fn table_tab(
    key: SharedString,
    grid: Entity<grid::Grid>,
    table: grid::Table,
    cx: &mut Context<Shell>,
) -> tabs::EditorTab<TabContent> {
    let s = cx.new(|cx| structure::Structure::new(grid.clone(), table, key.clone(), cx));
    // Focus and the status bar's aggregate follow the shown sub-tab.
    cx.observe(&s, |_, _, cx| cx.notify()).detach();
    grid_tab_of(key, grid, Some(s), cx)
}

pub(super) fn grid_tab(
    key: impl Into<SharedString>,
    source: Rc<dyn grid::GridSource>,
    cx: &mut Context<Shell>,
) -> tabs::EditorTab<TabContent> {
    let grid = cx.new(|cx| grid::Grid::new(source, cx));
    grid_tab_of(key, grid, None, cx)
}

/// A tab keyed `key` showing `grid`, whose queries go to the bottom panel's Logs.
fn grid_tab_of(
    key: impl Into<SharedString>,
    grid: Entity<grid::Grid>,
    structure: Option<Entity<structure::Structure>>,
    cx: &mut Context<Shell>,
) -> tabs::EditorTab<TabContent> {
    cx.subscribe(&grid, |this, _, grid::GridEvent::Logged(entry), cx| {
        this.bottom_panel
            .update(cx, |panel, cx| panel.push(entry.clone(), cx));
        // A page read follows every filter apply; the session keeps what it settled on.
        this.session_changed(cx);
    })
    .detach();
    let key = key.into();
    tabs::EditorTab {
        title: key.clone(),
        key,
        origin: None,
        tooltip: None,
        preview: false,
        pinned: false,
        content: TabContent::Grid(grid, structure),
    }
}

/// A console tab keyed `key` on `target`, with its own SQL editor holding `text`. Its runs go
/// to the bottom panel: statements to Logs, and the panel shows Results or Messages when done.
pub(super) fn console_tab(
    key: impl Into<SharedString>,
    title: impl Into<SharedString>,
    origin: Option<tabs::Origin>,
    target: console::Target,
    text: &str,
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> tabs::EditorTab<TabContent> {
    let console = cx.new(|cx| console::Console::new(target, text, window, cx));
    cx.subscribe(&console, |this, console, ev: &console::ConsoleEvent, cx| {
        let shown = console.read(cx).results.is_empty();
        match ev {
            console::ConsoleEvent::Logged(entry) => this
                .bottom_panel
                .update(cx, |panel, cx| panel.push(entry.clone(), cx)),
            console::ConsoleEvent::Finished => this
                .bottom_panel
                .update(cx, |panel, cx| panel.show_run(!shown, cx)),
            console::ConsoleEvent::Moved => this.console_moved(&console, cx),
        }
    })
    .detach();
    // The panel draws the active console's results, so it redraws with them.
    cx.observe(&console, |_, _, cx| cx.notify()).detach();
    tabs::EditorTab {
        key: key.into(),
        title: title.into(),
        origin,
        tooltip: None,
        preview: false,
        pinned: false,
        content: TabContent::Console(console),
    }
}
