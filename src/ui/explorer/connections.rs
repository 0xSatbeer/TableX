//! Connecting, and adding, editing and deleting connections.

use super::*;

impl Explorer {
    /// Connects connection `ci`, then lists its databases. Expanding a connection that is not
    /// connected and Retry on a failed one both come here. `all` expands everything it lists, as `*` does.
    pub(super) fn connect(&mut self, ci: usize, all: bool, cx: &mut Context<Self>) {
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

    /// Connects connection `id` as Retry on its error row does (Connect or Retry in the status bar).
    pub fn connect_connection(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        if let Some(ci) = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .position(|c| c.id == id)
        {
            self.connect(ci, false, cx);
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
