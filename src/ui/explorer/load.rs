//! Expanding nodes and loading, retrying and refreshing their children.

use super::*;

impl Explorer {
    pub(super) fn set_expanded(&mut self, ix: usize, open: bool, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        if !node.expandable {
            return;
        }
        // Before the already-open check: a failed connection stays open and still connects again.
        if open
            && matches!(node.kind, NodeKind::Connection(_))
            && self.workspace.read(cx).connections[node.conn].status != ConnectionStatus::Connected
        {
            let ci = node.conn;
            return self.connect(ci, false, cx);
        }
        if node.expanded == open {
            return;
        }
        let key = node.key.clone();
        if open && node.unloaded {
            self.load(key.clone(), false, cx);
        }
        if open {
            self.expanded.insert(key);
        } else {
            self.expanded.remove(&key);
        }
        self.refresh(cx);
    }

    /// Loads the children of the node `key` through its database's session. With `all` (`*`), the
    /// databases, schemas and object groups it brings are expanded and loaded in turn; tables are not.
    pub(super) fn load(&mut self, key: String, all: bool, cx: &mut Context<Self>) {
        let segments = segments(&key);
        let ws = self.workspace.read(cx);
        let Some(conn) = ws
            .connections
            .iter()
            .find(|c| c.id.0.to_string() == segments[0])
        else {
            return;
        };
        let id = conn.id;
        let db = segments
            .get(1)
            .cloned()
            .unwrap_or_else(|| conn.params.database());
        if matches!(self.loads.get(&key), Some(Load::Loading | Load::Refreshing)) {
            return;
        }
        // Another Postgres database needs a session of its own, opened with the same settings.
        let open: crate::db::Reply<Session> = match self.sessions.get(&(id, db.clone())) {
            Some(session) => Box::pin(futures::future::ready(Ok(session.clone()))),
            None => self.driver.connect(&on_database(&conn.params, &db)),
        };
        let refreshing = self.workspace.update(cx, |ws, _| {
            locate(ws, id, &segments).is_some_and(|node| node.loaded())
        });
        self.loads.insert(
            key.clone(),
            if refreshing {
                Load::Refreshing
            } else {
                Load::Loading
            },
        );
        // Entered, so a connection Connected from the start (the demo's) matches its own results.
        let generation = *self.gens.entry(id).or_default();
        let driver = self.driver.clone();
        cx.spawn(async move |this, cx| {
            let result = match open.await {
                Ok(session) => {
                    let current = this.update(cx, |this, _| {
                        let current = this.gens.get(&id) == Some(&generation);
                        if current {
                            this.sessions
                                .entry((id, db.clone()))
                                .or_insert_with(|| session.clone());
                        }
                        current
                    });
                    if !current.unwrap_or(false) {
                        return;
                    }
                    fetch(&*driver, &session, &segments).await
                }
                Err(reason) => Err(reason),
            };
            this.update(cx, |this, cx| {
                if this.gens.get(&id) != Some(&generation) {
                    return;
                }
                match result {
                    Ok(children) => {
                        this.loads.remove(&key);
                        let expanded = this.expanded.clone();
                        let (old, new) = this.workspace.update(cx, |ws, cx| {
                            cx.notify();
                            store(ws, id, &segments, children, |tail| {
                                expanded.contains(&format!("{key}/{tail}"))
                            })
                        });
                        // A child that is gone takes its expansion with it, and its descendants'.
                        for tail in old.iter().filter(|t| !new.contains(t)) {
                            let child = format!("{key}/{tail}");
                            this.expanded
                                .retain(|k| *k != child && !k.starts_with(&format!("{child}/")));
                        }
                        if all {
                            // Databases and schemas open; a schema opens its object groups but not the objects.
                            let open = new.iter().map(|tail| {
                                if segments.len() < 3 {
                                    tail.as_str()
                                } else {
                                    tail.split('/').next().unwrap_or_default()
                                }
                            });
                            this.expanded
                                .extend(open.map(|tail| format!("{key}/{tail}")));
                        }
                        // Expanded children are re-read in turn, and so on down.
                        for tail in new {
                            let child = format!("{key}/{tail}");
                            if this.expanded.contains(&child) {
                                this.load(child, all, cx);
                            }
                        }
                        if this.load_all && segments.len() == 2 {
                            this.load_all_objects(cx);
                        }
                    }
                    Err(reason) => {
                        // A session the server closed is dropped, so Retry opens a new one.
                        let at = (id, db);
                        if this.sessions.get(&at).is_some_and(Session::is_closed) {
                            this.sessions.remove(&at);
                        }
                        // A connection whose databases can no longer be listed has failed as a whole.
                        if refreshing && segments.len() == 1 {
                            this.loads.remove(&key);
                            this.set_status(id, ConnectionStatus::Error(reason), cx);
                            return this.refresh(cx);
                        }
                        this.workspace.update(cx, |ws, cx| {
                            if let Some(node) = locate(ws, id, &segments) {
                                node.unload();
                            }
                            cx.notify();
                        });
                        this.loads.insert(key, Load::Failed(reason));
                    }
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
        self.refresh(cx);
    }

    /// Retry on error row `ix`: connects again, or reloads the node the row sits under.
    pub(super) fn retry(&mut self, ix: usize, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        match node.kind {
            NodeKind::ConnError => self.connect(node.conn, false, cx),
            NodeKind::LoadError => {
                let parent = node.key[..node.key.len() - ERROR.len() - 1].to_string();
                self.load(parent, false, cx);
            }
            _ => {}
        }
    }

    /// Refresh in row `ix`'s menu: re-reads that node's subtree.
    pub(super) fn refresh_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(ix) else {
            return;
        };
        if matches!(node.kind, NodeKind::Connection(_))
            && self.workspace.read(cx).connections[node.conn].status != ConnectionStatus::Connected
        {
            return self.connect(node.conn, false, cx);
        }
        self.load(node.key.clone(), false, cx);
    }

    /// Whether a Refresh is re-reading anything under the connection keyed `conn`.
    pub(super) fn refreshing(&self, conn: &str) -> bool {
        self.loads.iter().any(|(key, load)| {
            *load == Load::Refreshing
                && key
                    .strip_prefix(conn)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    }

    /// Opens every ancestor of `key`, selects it and focuses the tree.
    pub fn reveal(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.expanded
            .extend(ancestors(key).into_iter().map(String::from));
        self.put_filter(String::new(), window, cx);
        if let Some(ix) = self.nodes.iter().position(|n| n.key == key) {
            self.select(ix, cx);
        }
        window.focus(&self.focus, cx);
    }

    /// Quick open: loads every database and schema not loaded yet on the connected connections,
    /// without expanding them; a database's schemas follow once it arrives.
    pub fn load_all_objects(&mut self, cx: &mut Context<Self>) {
        self.load_all = true;
        let ws = self.workspace.read(cx);
        let unloaded: Vec<String> = flatten(ws, &HashSet::new(), &HashMap::new(), true)
            .into_iter()
            .filter(|n| {
                n.unloaded
                    && matches!(n.kind, NodeKind::Database | NodeKind::Schema)
                    && ws.connections[n.conn].status == ConnectionStatus::Connected
            })
            .map(|n| n.key)
            .collect();
        for key in unloaded {
            self.load(key, false, cx);
        }
    }

    /// Quick open closed: databases loaded from now on bring only their schema list again.
    pub fn stop_loading_all(&mut self) {
        self.load_all = false;
    }

    /// Databases and schemas whose children are loading.
    pub fn schemas_pending(&self) -> usize {
        self.loads
            .iter()
            .filter(|(key, load)| {
                matches!(load, Load::Loading | Load::Refreshing)
                    && matches!(segments(key).len(), 2 | 3)
            })
            .count()
    }

    /// Quick open's Enter: reveals object `key` and opens it as a double-click would.
    pub fn open_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal(key, window, cx);
        if self.nodes.get(self.selected).is_some_and(|n| n.key == key) {
            self.open(self.selected, false, cx);
        }
    }

    /// `*`: opens the node and everything under it, loading the databases and schemas not loaded
    /// yet, but leaves tables collapsed so no columns are fetched.
    pub(super) fn expand_all_under(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(ix) else {
            return;
        };
        if matches!(node.kind, NodeKind::Connection(_))
            && self.workspace.read(cx).connections[node.conn].status != ConnectionStatus::Connected
        {
            return self.connect(node.conn, true, cx);
        }
        let root = node.key.clone();
        let prefix = format!("{root}/");
        let all = flatten(
            self.workspace.read(cx),
            &HashSet::new(),
            &HashMap::new(),
            true,
        );
        let mut unloaded = Vec::new();
        for n in all.into_iter().filter(|n| {
            n.expandable
                && !matches!(n.kind, NodeKind::Object(_))
                && (n.key.starts_with(&prefix) || n.key == root)
        }) {
            if n.unloaded {
                unloaded.push(n.key.clone());
            }
            self.expanded.insert(n.key);
        }
        for key in unloaded {
            self.load(key, true, cx);
        }
        self.refresh(cx);
    }
}
