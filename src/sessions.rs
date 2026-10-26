//! The open sessions of every connection, and its connection status: one place decides when a
//! session is reused, opened, kept or dropped. The explorer, table tabs, structure and console
//! completion all get their sessions here.

use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui_kit::*;

use crate::db::{Driver, Reply, Session};
use crate::model::{ConnectionId, ConnectionParams, ConnectionStatus, Secret, Workspace};
use crate::persist::Keys;
use crate::ui::password_prompt::keychain_failed;
use crate::ui::toast;

/// Emitted when connection `id` went Disconnected (lost or Disconnect), so tabs and the tree mark it.
pub struct ConnectionLost {
    pub id: ConnectionId,
}

impl EventEmitter<ConnectionLost> for Sessions {}

/// Emitted when connection `id` connected, so lost tabs re-read and the tree lists its databases.
pub struct ConnectionRestored {
    pub id: ConnectionId,
}

impl EventEmitter<ConnectionRestored> for Sessions {}

/// Emitted when connection `id`'s params changed or it was deleted: its sessions are closed.
pub struct ConnectionChanged {
    pub id: ConnectionId,
}

impl EventEmitter<ConnectionChanged> for Sessions {}

/// Emitted when a connect needs a password nobody has saved, so the explorer can ask for it.
pub struct PasswordNeeded {
    pub id: ConnectionId,
}

impl EventEmitter<PasswordNeeded> for Sessions {}

/// What `Sessions::connect` did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Connect {
    Started,
    /// Already connected or connecting.
    Busy,
    NeedsPassword,
}

pub struct Sessions {
    workspace: Entity<Workspace>,
    driver: Arc<dyn Driver>,
    /// The window the lost-connection toast shows in.
    window: AnyWindowHandle,
    /// Open sessions by connection and database.
    open: HashMap<(ConnectionId, String), Session>,
    /// Each connection's generation: every connect, close and params change bumps it, and a
    /// result carrying an older one is dropped.
    gens: HashMap<ConnectionId, u64>,
    /// Each connection's params at the last workspace change, to spot edits and deletes made anywhere.
    params: HashMap<ConnectionId, ConnectionParams>,
}

impl Sessions {
    pub fn new(
        workspace: Entity<Workspace>,
        driver: Arc<dyn Driver>,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&workspace, |this, _, cx| this.forget_changed(cx))
            .detach();
        let params = workspace
            .read(cx)
            .connections
            .iter()
            .map(|c| (c.id, c.params.clone()))
            .collect();
        Self {
            workspace,
            driver,
            window,
            open: HashMap::new(),
            gens: HashMap::new(),
            params,
        }
    }

    pub fn driver(&self) -> Arc<dyn Driver> {
        self.driver.clone()
    }

    /// Whether connection `id` is marked production now.
    pub fn is_production(&self, id: ConnectionId, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        workspace
            .connections
            .iter()
            .any(|c| c.id == id && c.is_production)
    }

    /// Connection `id`'s generation; a result started under an older one is stale.
    pub fn generation(&self, id: ConnectionId) -> u64 {
        self.gens.get(&id).copied().unwrap_or_default()
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
        if let Some(session) = self.open.get(&at).filter(|s| !s.is_closed()) {
            return Box::pin(futures::future::ready(Ok(session.clone())));
        }
        // A restored tab can get here before Connect has read the saved password from the keychain.
        self.password_ready(id, cx);
        let params = match self.params_on(id, db, cx) {
            Ok(params) => params,
            Err(reason) => return Box::pin(futures::future::ready(Err(reason))),
        };
        let reply = self.driver.connect(&params).shared();
        let generation = self.generation(id);
        let keep = reply.clone();
        cx.spawn(async move |this, cx| {
            let Ok(session) = keep.await else { return };
            this.update(cx, |this, _| {
                if this.generation(id) == generation {
                    let kept = this.open.entry(at).or_insert_with(|| session.clone());
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

    /// A new session on connection `id`'s database `db`, kept by nobody: the caller owns it.
    pub fn fresh(&self, id: ConnectionId, db: &str, cx: &App) -> Reply<Session> {
        match self.params_on(id, db, cx) {
            Ok(params) => self.driver.connect(&params),
            Err(reason) => Box::pin(futures::future::ready(Err(reason))),
        }
    }

    /// The open session on connection `id`'s database `db`, if one is open; never connects.
    pub fn open(&self, id: ConnectionId, db: &str) -> Option<Session> {
        let session = self.open.get(&(id, db.to_string()))?;
        (!session.is_closed()).then(|| session.clone())
    }

    /// Closes the kept session on connection `id`'s database `db`, as renaming that database needs.
    pub fn release(&mut self, id: ConnectionId, db: &str) {
        self.open.remove(&(id, db.to_string()));
    }

    /// A read on connection `id`'s database `db` failed: a session the server closed is dropped,
    /// so a retry opens a new one, and its connection is lost unless it was already. Returns
    /// whether the connection was lost now.
    pub fn read_failed(&mut self, id: ConnectionId, db: &str, cx: &mut Context<Self>) -> bool {
        let at = (id, db.to_string());
        if !self.open.get(&at).is_some_and(Session::is_closed) {
            return false;
        }
        self.open.remove(&at);
        let generation = self.generation(id);
        self.lost(id, cx);
        self.generation(id) != generation
    }

    fn params_on(&self, id: ConnectionId, db: &str, cx: &App) -> Result<ConnectionParams, String> {
        let workspace = self.workspace.read(cx);
        let conn = workspace.connections.iter().find(|c| c.id == id);
        let conn = conn.ok_or("The connection was deleted")?;
        Ok(on_database(&conn.params, db))
    }

    /// Connects connection `id` on its own database: Connecting, then Connected (`ConnectionRestored`)
    /// or Error. Without a password it emits `PasswordNeeded` and starts nothing.
    pub fn connect(&mut self, id: ConnectionId, cx: &mut Context<Self>) -> Connect {
        if !self.password_ready(id, cx) {
            cx.emit(PasswordNeeded { id });
            return Connect::NeedsPassword;
        }
        let started = self.workspace.update(cx, |ws, cx| {
            let conn = ws.connections.iter_mut().find(|c| c.id == id)?;
            if matches!(
                conn.status,
                ConnectionStatus::Connected | ConnectionStatus::Connecting
            ) {
                return None;
            }
            conn.status = ConnectionStatus::Connecting;
            cx.notify();
            Some(conn.params.clone())
        });
        let Some(params) = started else {
            return Connect::Busy;
        };
        let generation = self.next_gen(id);
        let reply = self.driver.connect(&params);
        cx.spawn(async move |this, cx| {
            let result = reply.await;
            this.update(cx, |this, cx| {
                if this.generation(id) != generation {
                    return;
                }
                match result {
                    Ok(session) => {
                        this.open.insert((id, params.database()), session);
                        this.set_status(id, ConnectionStatus::Connected, cx);
                        cx.emit(ConnectionRestored { id });
                    }
                    Err(reason) => this.set_status(id, ConnectionStatus::Error(reason), cx),
                }
            })
            .ok();
        })
        .detach();
        Connect::Started
    }

    /// Whether connection `id` has what it connects with: no password needed, one in memory,
    /// or one the keychain gives (then kept in memory).
    fn password_ready(&mut self, id: ConnectionId, cx: &mut Context<Self>) -> bool {
        let workspace = self.workspace.read(cx);
        let Some(conn) = workspace.connections.iter().find(|c| c.id == id) else {
            return true;
        };
        let ConnectionParams::Postgres { password: None, .. } = conn.params else {
            return true;
        };
        if !conn.save_password {
            return false;
        }
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

    /// Keeps `password` on connection `id` until quit, without counting as a params edit.
    pub fn remember_password(&mut self, id: ConnectionId, secret: Secret, cx: &mut Context<Self>) {
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

    /// Connection `id`'s session is gone: closes it (`disconnect`) and shows one toast. Does
    /// nothing while it isn't connected.
    pub fn lost(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        let Some(name) = self.disconnect(id, cx) else {
            return;
        };
        let notice = format!("Lost connection to {name}");
        let window = self.window;
        // Deferred: callers run inside entity or window updates.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| toast::error(notice, window, cx));
        });
    }

    /// Drops connection `id`'s sessions, marks it Disconnected and emits `ConnectionLost`.
    /// Returns its name, or None when it wasn't connected.
    pub fn disconnect(&mut self, id: ConnectionId, cx: &mut Context<Self>) -> Option<String> {
        let workspace = self.workspace.read(cx);
        let conn = workspace.connections.iter().find(|c| c.id == id)?;
        if conn.status != ConnectionStatus::Connected {
            return None;
        }
        let name = conn.name.clone();
        self.next_gen(id);
        self.open.retain(|(conn, _), _| *conn != id);
        self.set_status(id, ConnectionStatus::Disconnected, cx);
        cx.emit(ConnectionLost { id });
        Some(name)
    }

    /// Connection `id` is reachable but can no longer list its databases: it failed as a whole.
    pub fn failed(&mut self, id: ConnectionId, reason: String, cx: &mut Context<Self>) {
        self.set_status(id, ConnectionStatus::Error(reason), cx);
    }

    /// Closes the sessions of every connection whose params changed or that was deleted, so
    /// nothing from its old server lands or stays.
    fn forget_changed(&mut self, cx: &mut Context<Self>) {
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
            self.open.retain(|(conn, _), _| *conn != id);
            cx.emit(ConnectionChanged { id });
        }
        self.params = now;
    }

    fn next_gen(&mut self, id: ConnectionId) -> u64 {
        let generation = self.gens.entry(id).or_default();
        *generation += 1;
        *generation
    }

    fn set_status(&mut self, id: ConnectionId, status: ConnectionStatus, cx: &mut Context<Self>) {
        self.workspace.update(cx, |ws, cx| {
            if let Some(conn) = ws.connections.iter_mut().find(|c| c.id == id) {
                conn.status = status;
                cx.notify();
            }
        });
    }
}

/// `params` pointed at database `db`: a Postgres server's other database, or the SQLite file itself.
pub(crate) fn on_database(params: &ConnectionParams, db: &str) -> ConnectionParams {
    match params {
        ConnectionParams::Postgres { .. } if params.database() != db => {
            let mut params = params.clone();
            if let ConnectionParams::Postgres { database, .. } = &mut params {
                *database = db.to_string();
            }
            params
        }
        _ => params.clone(),
    }
}

#[cfg(test)]
mod tests;
