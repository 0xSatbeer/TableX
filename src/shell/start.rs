use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::sessions::{self, Sessions};
use crate::ui::{console, explorer, grid, panel, split, statusbar, structure, tabs};
use crate::{db, mock, model, theme};
use std::rc::Rc;

use super::{Mode, Shell, TabContent, layout::LayoutState};
use crate::bench::Scenario;

impl Shell {
    pub(super) fn new(mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (workspace, driver): (_, std::sync::Arc<dyn db::Driver>) = match mode {
            Mode::Bench(Scenario::BigResult) => {
                let env = |k| std::env::var(k).ok();
                match crate::bench::workspace(env("QUERYX_BENCH_PG"), env("QUERYX_BENCH_SQLITE")) {
                    Ok(workspace) => (workspace, std::sync::Arc::new(db::Live::default())),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
            }
            Mode::Normal | Mode::Bench(_) => (
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
        let handle = window.window_handle();
        let sessions = cx.new(|cx| Sessions::new(workspace.clone(), driver.clone(), handle, cx));
        let explorer =
            cx.new(|cx| explorer::Explorer::new(workspace.clone(), sessions.clone(), window, cx));
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
        cx.subscribe(&sessions, |this, _, ev: &sessions::ConnectionLost, cx| {
            this.connection_lost(ev.id, cx)
        })
        .detach();
        cx.subscribe_in(
            &sessions,
            window,
            |this, _, ev: &sessions::ConnectionRestored, window, cx| {
                this.connection_restored(ev.id, window, cx)
            },
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
        cx.subscribe(&explorer, |this, _, ev: &explorer::OpenObject, cx| {
            let open = this.tabs.tabs.iter().any(|t| t.key == ev.key);
            let table = (!open)
                .then(|| this.explorer.update(cx, |e, cx| e.table(&ev.key, cx)))
                .flatten();
            let mut tab = match table {
                Some(table) => {
                    let grid = cx.new(|cx| grid::Grid::table(table.clone(), cx));
                    grid.update(cx, |grid, cx| grid.load(cx));
                    table_tab(ev.key.clone(), grid, table, this.explorer.downgrade(), cx)
                }
                None => grid_tab(
                    ev.key.clone(),
                    Rc::new(model::ResultSet {
                        columns: Vec::new(),
                        rows: Vec::new(),
                    }),
                    cx,
                ),
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
        cx.subscribe_in(
            &explorer,
            window,
            |this, _, ev: &explorer::ShowDdl, window, cx| {
                let tab = this.tabs.tabs.iter().find(|t| t.key == ev.key);
                if let Some(TabContent::Grid(_, Some(s))) = tab.map(|t| &t.content) {
                    s.update(cx, |s, cx| s.show(structure::Sub::Ddl, window, cx));
                }
            },
        )
        .detach();
        cx.subscribe(&explorer, |this, _, ev: &explorer::ObjectDropped, cx| {
            if let Some(ix) = this.tabs.tabs.iter().position(|t| t.key == ev.key) {
                this.tabs.close(ix);
                this.tab_changed(cx);
            }
        })
        .detach();
        let status_bar = cx.new(|cx| statusbar::StatusBar::new(workspace.clone(), demo, cx));
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
            Mode::Bench(Scenario::GridScroll) => vec![
                grid_tab("big_events", Rc::new(grid::BigEvents::new()), cx),
                console,
            ],
            Mode::Normal | Mode::Bench(_) => vec![console],
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
            mode,
            overflow_open: false,
            overflow_search,
            workspace: workspace.clone(),
            driver,
            sessions,
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
            quick_open: None,
            _appearance: cx.observe_window_appearance(window, |this, window, cx| {
                this.theme.apply(&this.user_themes, window, cx)
            }),
        };
        if mode == Mode::Bench(Scenario::BigResult) {
            this.open_bench_tabs(window, cx);
        }
        this.tab_changed(cx);
        this
    }
}

impl Shell {
    /// `big_result`'s tabs on its Postgres and SQLite connections, beside the empty console: a
    /// console reading every row of `bench_rows`, a table tab on it on each, and a small query.
    fn open_bench_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let conns = self.workspace.read(cx).connections.clone();
        let [pg, lite] = &conns[..] else { return };
        for (conn, sql) in [
            (pg, "select * from bench_rows"),
            (lite, "select count(*) from bench_rows"),
        ] {
            let tab = self.connection_console(conn, conn.params.database(), None, sql, window, cx);
            if let TabContent::Console(c) = &tab.content {
                c.update(cx, |c, cx| c.run_all_pages(cx));
            }
            self.tabs.open(tab);
        }
        for (conn, schema) in [(pg, "public"), (lite, "main")] {
            let database = conn.params.database();
            let place = (database.clone(), schema.into(), "bench_rows".into());
            let table = grid::Table::new(&self.sessions, conn, place, model::ObjectKind::Table, cx);
            let grid = cx.new(|cx| grid::Grid::table(table.clone(), cx));
            grid.update(cx, |grid, cx| grid.load(cx));
            let key = explorer::object_key(
                conn.id,
                &database,
                schema,
                model::ObjectKind::Table,
                "bench_rows",
            );
            let explorer = self.explorer.downgrade();
            self.tabs
                .open(table_tab(key.into(), grid, table, explorer, cx));
        }
    }

    /// `grid_scroll`'s big_events grid.
    pub(crate) fn bench_grid(&self) -> Option<Entity<grid::Grid>> {
        self.tabs.tabs.iter().find_map(|t| match &t.content {
            TabContent::Grid(g, _) => Some(g.clone()),
            _ => None,
        })
    }

    /// Rows the first console holds once no tab is running or loading; `None` before.
    pub(crate) fn bench_rows(&self, cx: &App) -> Option<usize> {
        let mut rows = None;
        for tab in &self.tabs.tabs {
            match &tab.content {
                TabContent::Console(c) if !c.read(cx).settled() => return None,
                TabContent::Console(c) => rows = rows.or(c.read(cx).rows_held()),
                TabContent::Grid(g, _) if g.read(cx).loading() => return None,
                TabContent::Grid(..) => {}
            }
        }
        Some(rows.unwrap_or(0))
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
        let origin = tabs::Origin::at(conn, tail);
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

impl Shell {
    /// New console (Ctrl+Shift+N): always a new console tab, on the picked explorer row's
    /// connection, database and schema, else the active tab's; nothing without either.
    pub(super) fn new_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let place = self.explorer.read(cx).console_place(cx);
        let Some((id, database, schema)) = place.or_else(|| self.active_place(cx)) else {
            return;
        };
        let workspace = self.workspace.read(cx);
        let Some(conn) = workspace.connections.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        let mut tab = self.connection_console(&conn, database, schema, "", window, cx);
        self.next_console += 1;
        tab.key = format!("{}#{}", tab.key, self.next_console).into();
        self.console_ids.insert(tab.key.clone(), self.next_console);
        self.tabs.open(tab);
        self.tab_changed(cx);
    }

    /// The active tab's connection, database and schema, if it is on one.
    fn active_place(&self, cx: &App) -> Option<(model::ConnectionId, String, Option<String>)> {
        match &self.tabs.active()?.content {
            TabContent::Console(console) => {
                let target = console.read(cx).target();
                Some((
                    target.connection?,
                    target.database.clone(),
                    target.schema.clone(),
                ))
            }
            TabContent::Grid(grid, _) => {
                let (table, ..) = grid.read(cx).source()?;
                Some((
                    table.connection,
                    table.database.clone(),
                    Some(table.schema.clone()),
                ))
            }
        }
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
    explorer: WeakEntity<explorer::Explorer>,
    cx: &mut Context<Shell>,
) -> tabs::EditorTab<TabContent> {
    let s = cx.new(|cx| structure::Structure::new(grid.clone(), table, key.clone(), explorer, cx));
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
/// to the bottom panel: statements to Logs, and the panel shows Results or Logs when done.
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
            console::ConsoleEvent::Finished => {
                if !this.layout.bottom_visible {
                    this.layout.toggle_bottom();
                }
                this.bottom_panel
                    .update(cx, |panel, cx| panel.show_run(!shown, cx));
                cx.notify();
            }
            console::ConsoleEvent::Moved => this.console_moved(&console, cx),
            console::ConsoleEvent::Lost => {
                let tab = this
                    .tabs
                    .tabs
                    .iter()
                    .find(|t| matches!(&t.content, TabContent::Console(c) if *c == console));
                if let Some(id) = tab.and_then(|t| t.origin.as_ref()).map(|o| o.connection) {
                    this.sessions.update(cx, |s, cx| s.lost(id, cx));
                }
            }
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
