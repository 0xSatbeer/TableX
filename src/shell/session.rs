//! Saving and restoring the shown workspace's tabs, layout and expanded tree (`sessions/<id>.toml`).

use std::time::Duration;

use gpui_kit::*;

use crate::model::{DbKind, ObjectKind};
use crate::persist::{self, SessionFile, SessionTab};
use crate::ui::{explorer, grid, tabs};

use super::{Shell, TabContent, start};

/// How long the session waits after a change before it is written, so a drag writes once.
const SAVE_DELAY: Duration = Duration::from_millis(500);

impl Shell {
    /// Restores the shown workspace's session saved under `root` and saves every later change there.
    pub(super) fn persist_session(
        &mut self,
        root: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sessions_root = Some(root.to_path_buf());
        self.next_console = persist::max_console_id(root);
        self.restore_session(window, cx);
        cx.observe_self(|this, cx| this.session_changed(cx))
            .detach();
        cx.observe(&self.explorer, |this, _, cx| this.session_changed(cx))
            .detach();
        // A change still inside its debounce is written before the app goes.
        cx.on_app_quit(|this, cx| {
            this.save_session(cx);
            async {}
        })
        .detach();
    }

    /// Replaces the tabs, layout and expanded tree with the shown workspace's saved session.
    pub(super) fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.sessions_root.clone() else {
            return;
        };
        let (file, notice) = persist::load_session(&root, self.workspace.read(cx).id);
        if let Some(notice) = notice {
            // Deferred: at startup this runs while the window is being built, and notifications need its Root.
            cx.defer_in(window, move |_, window, cx| {
                crate::ui::toast::error(notice, window, cx);
            });
        }
        let layout = &mut self.layout;
        (layout.explorer_visible, layout.bottom_visible) =
            (file.explorer_visible, file.panel_visible);
        (layout.maximized_from, layout.panel_maximized) = (None, false);
        layout.explorer.size = px(file.explorer_width);
        layout.bottom.size = px(file.panel_height);
        self.explorer
            .update(cx, |e, cx| e.set_expanded_paths(&file.expanded, cx));
        let (mut restored, mut active, mut connect) = (Vec::new(), 0, Vec::new());
        for (ix, saved) in file.tabs.iter().enumerate() {
            let Some(tab) = self.restored_tab(saved, window, cx) else {
                continue;
            };
            if file.active_tab == Some(ix) {
                active = restored.len();
            }
            if let Some(o) = tab.origin.as_ref().filter(|o| !o.production)
                && !connect.contains(&o.connection)
            {
                connect.push(o.connection);
            }
            restored.push(tab);
        }
        let old = std::mem::replace(&mut self.tabs, tabs::Tabs::new(restored));
        for tab in old.tabs {
            if let TabContent::Console(console) = tab.content {
                console.update(cx, |console, cx| {
                    console.save_text(cx);
                    console.close_session(cx);
                });
            }
        }
        self.tabs.activate(active);
        for id in connect {
            self.explorer
                .update(cx, |e, cx| e.connect_collapsed(id, cx));
        }
        self.session_saved = Some(self.session_snapshot(cx));
        self.tab_changed(cx);
    }

    /// The tab `saved` describes; `None` when its connection no longer exists. Table tabs on
    /// production wait for Connect; the others read page 1 now.
    fn restored_tab(
        &mut self,
        saved: &SessionTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<tabs::EditorTab<TabContent>> {
        let (SessionTab::Table { connection, .. } | SessionTab::Console { connection, .. }) = saved;
        let conn = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| c.id.0 == *connection)?
            .clone();
        match saved {
            SessionTab::Table {
                database,
                schema,
                object,
                where_,
                order_by,
                pinned,
                ..
            } => {
                // shortcut: C8 doesn't say table or view, so a view restores as a table (editable,
                // its saves fail on the server); add the kind to the file if that bites.
                let table = grid::Table {
                    driver: self.driver.clone(),
                    connection: conn.id,
                    database: database.clone(),
                    explorer: self.explorer.downgrade(),
                    schema: schema.clone(),
                    name: object.clone(),
                    sqlite_table: conn.kind() == DbKind::Sqlite,
                    view: false,
                };
                let grid = cx.new(|cx| {
                    let mut grid = grid::Grid::table(table.clone(), cx);
                    grid.set_filter(where_.clone(), order_by.clone());
                    if conn.is_production {
                        grid.set_disconnected();
                    } else {
                        grid.load(cx);
                    }
                    grid
                });
                let key: SharedString =
                    explorer::object_key(conn.id, database, schema, ObjectKind::Table, object)
                        .into();
                let tail = tabs::join_path(database, schema);
                let mut tab = start::table_tab(key, grid, table, cx);
                tab.tooltip = Some(format!("{} › {tail} › {object}", conn.name).into());
                tab.title = object.clone().into();
                tab.origin = Some(tabs::Origin {
                    connection: conn.id,
                    color: conn.color,
                    production: conn.is_production,
                    path: tabs::join_path(&conn.name, &tail),
                    tail,
                });
                tab.pinned = *pinned;
                Some(tab)
            }
            SessionTab::Console {
                database,
                schema,
                console,
                autocommit,
                pinned,
                ..
            } => {
                let root = self.sessions_root.as_ref()?;
                let path = root.join("consoles").join(format!("{console}.sql"));
                let text = std::fs::read_to_string(path).unwrap_or_default();
                let schema = Some(schema.clone()).filter(|s| !s.is_empty());
                let mut tab =
                    self.connection_console(&conn, database.clone(), schema, &text, window, cx);
                if let TabContent::Console(c) = &tab.content {
                    c.update(cx, |c, cx| c.set_manual(!autocommit, window, cx));
                }
                // Consoles from New console share a place: each keeps its own key and file.
                if self.console_ids.contains_key(&tab.key) {
                    tab.key = format!("{}#{console}", tab.key).into();
                }
                self.console_ids.insert(tab.key.clone(), *console);
                self.next_console = self.next_console.max(*console);
                tab.pinned = *pinned;
                Some(tab)
            }
        }
    }

    /// What `sessions/<id>.toml` holds for the shown workspace now. Tabs not from a connection
    /// (sample data, a console on nothing) aren't saved.
    fn session_snapshot(&mut self, cx: &App) -> SessionFile {
        let (mut saved, mut active_tab) = (Vec::new(), None);
        for (ix, t) in self.tabs.tabs.iter().enumerate() {
            let tab = match &t.content {
                TabContent::Grid(grid, _) => {
                    let Some((table, where_, order_by)) = grid.read(cx).source() else {
                        continue;
                    };
                    SessionTab::Table {
                        connection: table.connection.0,
                        database: table.database.clone(),
                        schema: table.schema.clone(),
                        object: table.name.clone(),
                        where_: where_.to_string(),
                        order_by: order_by.to_string(),
                        pinned: t.pinned,
                    }
                }
                TabContent::Console(console) => {
                    let console = console.read(cx);
                    let target = console.target();
                    let Some(connection) = target.connection else {
                        continue;
                    };
                    let next = &mut self.next_console;
                    let id = *self.console_ids.entry(t.key.clone()).or_insert_with(|| {
                        *next += 1;
                        *next
                    });
                    SessionTab::Console {
                        connection: connection.0,
                        database: target.database.clone(),
                        schema: target.schema.clone().unwrap_or_default(),
                        console: id,
                        autocommit: console.autocommit(),
                        pinned: t.pinned,
                    }
                }
            };
            if ix == self.tabs.active {
                active_tab = Some(saved.len());
            }
            saved.push(tab);
        }
        let layout = &self.layout;
        // Maximize hides both panels for now; what comes back is how they were before it.
        let (explorer_visible, panel_visible) = layout
            .maximized_from
            .unwrap_or((layout.explorer_visible, layout.bottom_visible));
        SessionFile {
            version: 1,
            active_tab: active_tab.or((!saved.is_empty()).then_some(0)),
            explorer_visible,
            explorer_width: layout.explorer.size.into(),
            panel_visible,
            panel_height: layout.bottom.size.into(),
            expanded: self.explorer.read(cx).expanded_paths(),
            tabs: saved,
        }
    }

    /// Saves the session `SAVE_DELAY` after the last change, when anything it holds changed.
    pub(super) fn session_changed(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.sessions_root.clone() else {
            return;
        };
        let snapshot = self.session_snapshot(cx);
        if self.session_saved.as_ref() == Some(&snapshot) {
            return;
        }
        self.session_saved = Some(snapshot.clone());
        let id = self.workspace.read(cx).id;
        self._session_save = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            if let Err(e) = persist::save_session(&root, id, &snapshot) {
                eprintln!("Couldn't save the session of workspace {id}: {e}");
            }
        }));
    }

    /// Saves the session now, as switching workspace does before the tabs go.
    pub(super) fn save_session(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.sessions_root.clone() else {
            return;
        };
        self._session_save = None;
        let snapshot = self.session_snapshot(cx);
        let id = self.workspace.read(cx).id;
        if let Err(e) = persist::save_session(&root, id, &snapshot) {
            eprintln!("Couldn't save the session of workspace {id}: {e}");
        }
        self.session_saved = Some(snapshot);
    }
}
