//! Connecting, and adding, editing and deleting connections.

use super::*;
use crate::db::Reply;
use crate::model::Secret;
use crate::persist::Keys;
use crate::ui::password_prompt::{Entered, PasswordPrompt, keychain_failed};
use futures::FutureExt as _;

impl Explorer {
    /// Connects connection `ci`, then lists its databases. Expanding a connection that is not
    /// connected and Retry on a failed one both come here. `all` expands everything it lists, as `*` does.
    pub(super) fn connect(&mut self, ci: usize, all: bool, cx: &mut Context<Self>) {
        if !self.password_ready(ci, cx) {
            return self.ask_password(ci, all, cx);
        }
        let key = self.workspace.read(cx).connections[ci].id.0.to_string();
        // Opened even when a connect is already running, so an expand while Connecting isn't lost.
        self.expanded.insert(key.clone());
        let started = self.workspace.update(cx, |ws, cx| {
            let conn = &mut ws.connections[ci];
            if matches!(
                conn.status,
                ConnectionStatus::Connected | ConnectionStatus::Connecting
            ) {
                return None;
            }
            conn.status = ConnectionStatus::Connecting;
            cx.notify();
            Some((conn.id, conn.params.clone()))
        });
        let Some((id, params)) = started else {
            return self.refresh(cx);
        };
        let generation = self.next_gen(id);
        // Loads from before carry the old generation and will be dropped, so none stays in flight.
        self.loads.retain(|k, _| !of_connection(k, id));
        let reply = self.driver.connect(&params);
        cx.spawn(async move |this, cx| {
            let result = reply.await;
            this.update(cx, |this, cx| {
                if this.gens.get(&id) != Some(&generation) {
                    return;
                }
                match result {
                    Ok(session) => {
                        this.sessions.insert((id, params.database()), session);
                        this.set_status(id, ConnectionStatus::Connected, cx);
                        this.load(key, all, cx);
                    }
                    Err(reason) => this.set_status(id, ConnectionStatus::Error(reason), cx),
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
        self.refresh(cx);
    }

    /// Whether connection `ci` has what it connects with: no password needed, one in memory,
    /// or one the keychain gives (then kept in memory).
    fn password_ready(&mut self, ci: usize, cx: &mut Context<Self>) -> bool {
        let conn = &self.workspace.read(cx).connections[ci];
        let ConnectionParams::Postgres { password: None, .. } = conn.params else {
            return true;
        };
        if !conn.save_password {
            return false;
        }
        let id = conn.id;
        // shortcut: the keychain is read on the UI thread; move it off if a slow D-Bus shows.
        match Keys::of(cx).get(id) {
            Ok(Some(password)) => {
                self.remember_password(id, password, cx);
                true
            }
            Ok(None) => false,
            Err(error) => {
                keychain_failed(&error, cx);
                false
            }
        }
    }

    /// Asks for connection `ci`'s password, then connects with it; Cancel leaves it as it was.
    fn ask_password(&mut self, ci: usize, all: bool, cx: &mut Context<Self>) {
        let conn = &self.workspace.read(cx).connections[ci];
        let (id, name, window) = (conn.id, conn.name.clone(), self.window);
        let this = cx.entity().downgrade();
        // Deferred: this runs inside the window's event handling, which holds the window.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let prompt = PasswordPrompt::open(&name, window, cx);
                let _ = this.update(cx, |_, cx| {
                    cx.subscribe(&prompt, move |this, _, Entered(password), cx| {
                        this.remember_password(id, password.clone(), cx);
                        this.connect_connection_all(id, all, cx);
                    })
                    .detach()
                });
            });
        });
    }

    /// Keeps `password` on connection `id` until quit, without counting as a params edit.
    fn remember_password(&mut self, id: ConnectionId, secret: Secret, cx: &mut Context<Self>) {
        let params = self.workspace.update(cx, |ws, cx| {
            let conn = ws.connections.iter_mut().find(|c| c.id == id)?;
            if let ConnectionParams::Postgres { password, .. } = &mut conn.params {
                *password = Some(secret);
            }
            cx.notify();
            Some(conn.params.clone())
        });
        if let Some(params) = params {
            self.params.insert(id, params);
        }
    }

    /// Where the table or view row keyed `key` reads its rows: on the session for its database.
    pub fn table(&self, key: &str, cx: &Context<Self>) -> Option<crate::ui::grid::Table> {
        let node = self.nodes.iter().find(|n| n.key == key)?;
        let NodeKind::Object(kind) = node.kind else {
            return None;
        };
        let conn = &self.workspace.read(cx).connections[node.conn];
        let mut parts = segments(key).into_iter().skip(1);
        let (database, schema) = (parts.next()?, parts.next()?);
        Some(crate::ui::grid::Table {
            driver: self.driver.clone(),
            connection: conn.id,
            explorer: cx.entity().downgrade(),
            name: parts.nth(1)?,
            database,
            schema,
            sqlite_table: conn.kind() == DbKind::Sqlite && kind == ObjectKind::Table,
            view: kind != ObjectKind::Table,
        })
    }

    /// Whether connection `id` is marked production now.
    pub fn is_production(&self, id: ConnectionId, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        workspace
            .connections
            .iter()
            .any(|c| c.id == id && c.is_production)
    }

    /// The open session on connection `id`'s database `db`; without one, a new one, kept for the
    /// next caller unless the connection changed meanwhile.
    pub fn session(
        &mut self,
        id: ConnectionId,
        db: &str,
        cx: &mut Context<Self>,
    ) -> Reply<Session> {
        let at = (id, db.to_string());
        if let Some(session) = self.sessions.get(&at).filter(|s| !s.is_closed()) {
            return Box::pin(futures::future::ready(Ok(session.clone())));
        }
        let Some(conn) = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| c.id == id)
        else {
            return Box::pin(futures::future::ready(Err(
                "The connection was deleted".into()
            )));
        };
        let reply = self.driver.connect(&on_database(&conn.params, db)).shared();
        let generation = *self.gens.entry(id).or_default();
        let keep = reply.clone();
        cx.spawn(async move |this, cx| {
            let Ok(session) = keep.await else { return };
            this.update(cx, |this, _| {
                if this.gens.get(&id) == Some(&generation) {
                    let kept = this.sessions.entry(at).or_insert_with(|| session.clone());
                    if kept.is_closed() {
                        *kept = session;
                    }
                }
            })
            .ok();
        })
        .detach();
        reply.boxed()
    }

    /// Connects connection `id` as Retry on its error row does (Connect or Retry in the status bar).
    pub fn connect_connection(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        self.connect_connection_all(id, false, cx);
    }

    fn connect_connection_all(&mut self, id: ConnectionId, all: bool, cx: &mut Context<Self>) {
        if let Some(ci) = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .position(|c| c.id == id)
        {
            self.connect(ci, all, cx);
        }
    }

    /// Closes the sessions of every connection whose params changed or that was deleted, drops
    /// what it was loading and collapses it, so nothing from its old server lands or stays.
    pub(super) fn forget_changed(&mut self, cx: &mut Context<Self>) {
        let now: HashMap<_, _> = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .map(|c| (c.id, c.params.clone()))
            .collect();
        let changed: Vec<ConnectionId> = self
            .params
            .iter()
            .filter(|(id, params)| now.get(id) != Some(params))
            .map(|(id, _)| *id)
            .collect();
        for id in changed {
            self.next_gen(id);
            self.sessions.retain(|(conn, _), _| *conn != id);
            self.expanded.retain(|k| !of_connection(k, id));
            self.loads.retain(|k, _| !of_connection(k, id));
        }
        self.params = now;
    }

    /// Bumps connection `id`'s generation, so every result started before is dropped.
    pub(super) fn next_gen(&mut self, id: ConnectionId) -> u64 {
        let generation = self.gens.entry(id).or_default();
        *generation += 1;
        *generation
    }

    pub(super) fn set_status(
        &mut self,
        id: ConnectionId,
        status: ConnectionStatus,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            if let Some(conn) = ws.connections.iter_mut().find(|c| c.id == id) {
                conn.status = status;
                cx.notify();
            }
        });
    }

    /// Opens the Add connection dialog ("+" in the header, `AddConnection`).
    pub fn add_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_dialog(None, window, cx);
    }

    /// Opens the Edit dialog for the selected connection row (Edit… in its menu, `EditConnection`).
    pub fn edit_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self
            .nodes
            .get(self.selected)
            .filter(|n| matches!(n.kind, NodeKind::Connection(_)))
        else {
            return;
        };
        let id = self.workspace.read(cx).connections[node.conn].id;
        self.open_dialog(Some(id), window, cx);
    }

    /// Asks to confirm deleting the selected connection row (Delete in its menu, `DeleteConnection`).
    pub fn delete_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self
            .nodes
            .get(self.selected)
            .filter(|n| matches!(n.kind, NodeKind::Connection(_)))
        else {
            return;
        };
        let conn = self.workspace.read(cx).connections[node.conn].clone();
        let dialog = DeleteConfirm::open(&conn, window, cx);
        cx.subscribe(&dialog, move |this, _, Confirmed, cx| {
            this.remove_connection(conn.id, cx)
        })
        .detach();
    }

    /// Deletes connection `id`. A connect still running for it finds it gone and is discarded.
    pub(super) fn remove_connection(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        let removed = self.workspace.update(cx, |ws, cx| {
            cx.notify();
            ws.delete(id)
        });
        let Some(ix) = removed else { return };
        // Even when not saved now: an earlier failed delete may have left the entry behind.
        if let Err(error) = Keys::of(cx).delete(id) {
            keychain_failed(&error, cx);
        }
        // A rename holds a position; the delete ends it and the marked rows.
        self.rename = None;
        self.marked.clear();
        cx.emit(ConnectionDeleted { id });
        self.refresh(cx);
        // The connection that took its place, or the previous one when it was last.
        let connections = &self.workspace.read(cx).connections;
        let next = connections
            .get(ix.min(connections.len().saturating_sub(1)))
            .map(|c| c.id.0.to_string());
        if let Some(row) = self
            .nodes
            .iter()
            .position(|n| Some(&n.key) == next.as_ref())
        {
            self.select(row, cx);
        }
    }

    /// Opens Modify for the selected Postgres database, schema or table row (Modify… in its menu, `ModifyObject`).
    pub fn modify_object(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };
        let kind = match node.kind {
            NodeKind::Database => Owned::Database,
            NodeKind::Schema => Owned::Schema,
            NodeKind::Object(ObjectKind::Table) => return self.modify_table(window, cx),
            _ => return,
        };
        let conn = self.workspace.read(cx).connections[node.conn].clone();
        if conn.kind() != DbKind::Postgres {
            return;
        }
        let name = node.label.to_string();
        let db = segments(&node.key)[1].clone();
        let db_key = ancestors(&node.key)
            .get(1)
            .map_or(node.key.clone(), |k| k.to_string());
        let open: crate::db::Reply<Session> = match self.sessions.get(&(conn.id, db.clone())) {
            Some(session) => Box::pin(futures::future::ready(Ok(session.clone()))),
            None => self.driver.connect(&on_database(&conn.params, &db)),
        };
        let dialog = NameOwnerDialog::open(
            self.driver.clone(),
            open,
            kind,
            &name,
            conn.is_production,
            window,
            cx,
        );
        cx.subscribe_in(
            &dialog,
            window,
            move |this, dialog, Apply(statements), window, cx| {
                let (on, atomic, refresh) = match kind {
                    // Dropping the session on `db` keeps it from blocking a rename.
                    Owned::Database => {
                        this.sessions.remove(&(conn.id, db.clone()));
                        ("postgres", false, conn.id.0.to_string())
                    }
                    Owned::Schema => (db.as_str(), true, db_key.clone()),
                };
                let dialog = dialog.downgrade();
                this.apply(
                    conn.id,
                    on,
                    atomic,
                    Some(refresh),
                    move |result, cx| {
                        if let Err(reason) = result {
                            dialog.update(cx, |d, cx| d.failed(reason, cx)).ok();
                        }
                    },
                    statements.clone(),
                    window,
                    cx,
                )
            },
        )
        .detach();
    }

    /// Opens the table editor on the selected Postgres table row. Apply runs atomically on the
    /// table's database and refreshes its schema.
    fn modify_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };
        let conn = self.workspace.read(cx).connections[node.conn].clone();
        if conn.kind() != DbKind::Postgres {
            return;
        }
        let parts = segments(&node.key);
        let (db, schema, table) = (parts[1].clone(), &parts[2], &parts[4]);
        let schema_key = ancestors(&node.key)[2].to_string();
        let open: crate::db::Reply<Session> = match self.sessions.get(&(conn.id, db.clone())) {
            Some(session) => Box::pin(futures::future::ready(Ok(session.clone()))),
            None => self.driver.connect(&on_database(&conn.params, &db)),
        };
        let dialog = TableEditor::open(
            self.driver.clone(),
            open,
            (schema, table),
            conn.is_production,
            window,
            cx,
        );
        cx.subscribe_in(
            &dialog,
            window,
            move |this, dialog, Apply(statements), window, cx| {
                let dialog = dialog.downgrade();
                this.apply(
                    conn.id,
                    &db,
                    true,
                    Some(schema_key.clone()),
                    move |result, cx| {
                        if let Err(reason) = result {
                            dialog.update(cx, |d, cx| d.failed(reason, cx)).ok();
                        }
                    },
                    statements.clone(),
                    window,
                    cx,
                )
            },
        )
        .detach();
    }

    /// Opens New table for the selected Postgres database, schema or table row (+ New in its
    /// menu), in that schema, or `public` from a database. Apply runs atomically on the row's
    /// database and refreshes the schema.
    pub fn new_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };
        let schema_key = match node.kind {
            NodeKind::Database => format!("{}/{}", node.key, seg("public")),
            NodeKind::Schema => node.key.clone(),
            NodeKind::Object(ObjectKind::Table) => ancestors(&node.key)[2].to_string(),
            _ => return,
        };
        let conn = self.workspace.read(cx).connections[node.conn].clone();
        if conn.kind() != DbKind::Postgres {
            return;
        }
        let parts = segments(&schema_key);
        let (db, schema) = (parts[1].clone(), &parts[2]);
        let dialog = TableEditor::create(schema, conn.is_production, window, cx);
        cx.subscribe_in(
            &dialog,
            window,
            move |this, dialog, Apply(statements), window, cx| {
                let dialog = dialog.downgrade();
                this.apply(
                    conn.id,
                    &db,
                    true,
                    Some(schema_key.clone()),
                    move |result, cx| {
                        if let Err(reason) = result {
                            dialog.update(cx, |d, cx| d.failed(reason, cx)).ok();
                        }
                    },
                    statements.clone(),
                    window,
                    cx,
                )
            },
        )
        .detach();
    }

    /// Opens New Database for the selected Postgres database, schema or table row (+ New in its
    /// menu). Apply runs non-atomically on the row's database and refreshes the connection.
    pub fn new_database(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };
        if !matches!(
            node.kind,
            NodeKind::Database | NodeKind::Schema | NodeKind::Object(ObjectKind::Table)
        ) {
            return;
        }
        let conn = self.workspace.read(cx).connections[node.conn].clone();
        let ConnectionParams::Postgres { user, .. } = &conn.params else {
            return;
        };
        let db = segments(&node.key)[1].clone();
        let open: crate::db::Reply<Session> = match self.sessions.get(&(conn.id, db.clone())) {
            Some(session) => Box::pin(futures::future::ready(Ok(session.clone()))),
            None => self.driver.connect(&on_database(&conn.params, &db)),
        };
        let dialog = NameOwnerDialog::create(
            self.driver.clone(),
            open,
            user,
            conn.is_production,
            window,
            cx,
        );
        cx.subscribe_in(
            &dialog,
            window,
            move |this, dialog, Apply(statements), window, cx| {
                let dialog = dialog.downgrade();
                this.apply(
                    conn.id,
                    &db,
                    false,
                    Some(conn.id.0.to_string()),
                    move |result, cx| {
                        if let Err(reason) = result {
                            dialog.update(cx, |d, cx| d.failed(reason, cx)).ok();
                        }
                    },
                    statements.clone(),
                    window,
                    cx,
                )
            },
        )
        .detach();
    }

    /// Opens New Role, or New User when `login`, for the selected Postgres database, schema or
    /// table row (+ New in its menu). Apply runs atomically on the row's database.
    pub fn new_role(&mut self, login: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };
        if !matches!(
            node.kind,
            NodeKind::Database | NodeKind::Schema | NodeKind::Object(ObjectKind::Table)
        ) {
            return;
        }
        let conn = self.workspace.read(cx).connections[node.conn].clone();
        if conn.kind() != DbKind::Postgres {
            return;
        }
        let db = segments(&node.key)[1].clone();
        let open: crate::db::Reply<Session> = match self.sessions.get(&(conn.id, db.clone())) {
            Some(session) => Box::pin(futures::future::ready(Ok(session.clone()))),
            None => self.driver.connect(&on_database(&conn.params, &db)),
        };
        let dialog = RoleDialog::open(
            self.driver.clone(),
            open,
            login,
            conn.is_production,
            window,
            cx,
        );
        cx.subscribe_in(
            &dialog,
            window,
            move |this, dialog, Apply(statements), window, cx| {
                let dialog = dialog.downgrade();
                this.apply(
                    conn.id,
                    &db,
                    true,
                    None,
                    move |result, cx| {
                        if let Err(reason) = result {
                            dialog.update(cx, |d, cx| d.failed(reason, cx)).ok();
                        }
                    },
                    statements.clone(),
                    window,
                    cx,
                )
            },
        )
        .detach();
    }

    /// Runs a Modify dialog's `statements` on connection `id`'s database `on`. Success closes the
    /// dialog and re-reads the node keyed `refresh`, if any; either way `done` gets the result.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply(
        &mut self,
        id: ConnectionId,
        on: &str,
        atomic: bool,
        refresh: Option<String>,
        done: impl FnOnce(Result<(), String>, &mut Context<Self>) + 'static,
        statements: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(params) = self.params.get(&id).cloned() else {
            return;
        };
        let at = (id, on.to_string());
        let open: crate::db::Reply<Session> = match self.sessions.get(&at) {
            Some(session) => Box::pin(futures::future::ready(Ok(session.clone()))),
            None => self.driver.connect(&on_database(&params, on)),
        };
        let generation = *self.gens.entry(id).or_default();
        let driver = self.driver.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = match open.await {
                Ok(session) => {
                    this.update(cx, |this, _| {
                        if this.gens.get(&id) == Some(&generation) {
                            this.sessions.entry(at).or_insert_with(|| session.clone());
                        }
                    })
                    .ok();
                    let done = driver.execute(&session, statements, atomic).await;
                    done.map_err(|err| err.message)
                }
                Err(reason) => Err(reason),
            };
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    window.close_dialog(cx);
                    if let Some(refresh) =
                        refresh.filter(|_| this.gens.get(&id) == Some(&generation))
                    {
                        this.load(refresh, false, cx);
                    }
                    done(Ok(()), cx);
                }
                Err(reason) => done(Err(reason), cx),
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn open_dialog(
        &mut self,
        editing: Option<ConnectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = ConnectionDialog::open(
            self.workspace.clone(),
            self.driver.clone(),
            editing,
            window,
            cx,
        );
        cx.subscribe_in(&dialog, window, |this, _, Saved(id), window, cx| {
            if this
                .workspace
                .read(cx)
                .connections
                .iter()
                .any(|c| c.id == *id)
            {
                this.reveal(&id.0.to_string(), window, cx);
            }
        })
        .detach();
    }
}
