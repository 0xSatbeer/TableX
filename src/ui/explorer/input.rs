//! Selection, opening and keyboard handling.

use super::*;

impl Explorer {
    /// Focuses the filter field (Ctrl+Alt+F).
    pub fn focus_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_filter = true;
        window.focus(&self.field.focus_handle(cx), cx);
        cx.notify();
    }

    /// The funnel: shows and focuses the filter row, or clears and hides it.
    pub(super) fn toggle_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_shown() {
            self.show_filter = false;
            self.put_filter(String::new(), window, cx);
            window.focus(&self.focus, cx);
        } else {
            self.focus_field(window, cx);
        }
    }

    pub(super) fn filter_shown(&self) -> bool {
        self.show_filter || !self.filter.is_empty()
    }

    /// Replaces the field's text and applies it as the filter (`set_value` emits no Change).
    pub(super) fn put_filter(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.field
            .update(cx, |f, cx| f.set_value(text.clone(), window, cx));
        self.set_filter(text, cx);
    }

    /// Esc / Down while the field has focus; runs in the capture phase so the input never sees them.
    pub(super) fn on_field_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let route = route_key(
            Focus::Field,
            &ev.keystroke.key,
            None,
            ev.keystroke.modifiers,
            self.filter.is_empty(),
            || first_match(&self.nodes, &self.filter).is_some(),
            false,
        );
        match route {
            KeyRoute::ClearField => self.put_filter(String::new(), window, cx),
            KeyRoute::FocusTree => window.focus(&self.focus, cx),
            KeyRoute::FocusFirstMatch => {
                window.focus(&self.focus, cx);
                self.select(first_match(&self.nodes, &self.filter).unwrap_or(0), cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.nodes = tree_nodes(
            self.workspace.read(cx),
            &self.expanded,
            &self.loads,
            &self.filter,
        );
        self.selected = self.selected.min(self.nodes.len().saturating_sub(1));
        cx.notify();
    }

    pub(super) fn set_filter(&mut self, filter: String, cx: &mut Context<Self>) {
        self.filter = filter;
        self.nodes = tree_nodes(
            self.workspace.read(cx),
            &self.expanded,
            &self.loads,
            &self.filter,
        );
        // First match, or the top when clearing.
        let first = first_match(&self.nodes, &self.filter);
        self.select(first.unwrap_or(0), cx);
    }

    pub(super) fn open(&mut self, ix: usize, preview: bool, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        if opens_tab(node.kind) {
            let conn = &self.workspace.read(cx).connections[node.conn];
            let (connection, color, production) = (conn.id, conn.color, conn.is_production);
            let mut names = vec![node.label.to_string()];
            let mut at = ix;
            while let Some(parent) = parent_of(&self.nodes, at) {
                // Groups ("tables") are UI only, not part of the object path.
                if !matches!(self.nodes[parent].kind, NodeKind::Group(_)) {
                    names.push(self.nodes[parent].label.to_string());
                }
                at = parent;
            }
            names.reverse();
            cx.emit(OpenObject {
                key: node.key.clone().into(),
                name: node.label.clone(),
                origin: {
                    let tail = names[1..names.len() - 1].join(" › ");
                    Origin {
                        connection,
                        color,
                        production,
                        path: crate::ui::tabs::join_path(&names[0], &tail),
                        tail: tail.into(),
                    }
                },
                path: names.join(" › ").into(),
                preview,
            });
        }
    }

    /// Open console on the selected database or schema row.
    pub(super) fn open_console(&mut self, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };
        if !matches!(node.kind, NodeKind::Database | NodeKind::Schema) {
            return;
        }
        let mut parts = segments(&node.key).into_iter().skip(1);
        let database = parts.next().expect("a database row's key names it");
        cx.emit(OpenConsole {
            conn: self.workspace.read(cx).connections[node.conn].id,
            database,
            schema: parts.next(),
        });
    }

    /// Whether `node` is a connection whose connect failed.
    pub(super) fn failed(&self, node: &FlatNode, cx: &App) -> bool {
        matches!(node.kind, NodeKind::Connection(_))
            && matches!(
                self.workspace.read(cx).connections[node.conn].status,
                ConnectionStatus::Error(_)
            )
    }

    /// A plain (unmodified) press: select the row, then do what `click_action` says.
    pub(super) fn press(&mut self, ix: usize, hit: Hit, cx: &mut Context<Self>) {
        self.marked.clear();
        self.select(ix, cx);
        match click_action(self.nodes[ix].kind, hit) {
            Click::Select => {}
            Click::Toggle => self.set_expanded(ix, !self.nodes[ix].expanded, cx),
            Click::Open { preview } => self.open(ix, preview, cx),
        }
    }

    #[cfg(test)]
    pub fn row_labels(&self) -> Vec<SharedString> {
        self.nodes.iter().map(|n| n.label.clone()).collect()
    }

    #[cfg(test)]
    pub fn selected_label(&self) -> Option<SharedString> {
        self.nodes.get(self.selected).map(|n| n.label.clone())
    }

    pub(super) fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        (self.selected, self.picked) = (ix, true);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    pub(super) fn on_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let m = ev.keystroke.modifiers;
        if self.field.focus_handle(cx).is_focused(window) {
            return;
        }
        if let Some(rn) = &mut self.rename {
            let conn = rn.conn;
            let others = other_names(self.workspace.read(cx), conn);
            match rn.key(
                &ev.keystroke.key,
                ev.keystroke
                    .key_char
                    .as_deref()
                    .filter(|_| !m.control && !m.alt && !m.platform),
                &others,
            ) {
                RenameKey::Open => {}
                RenameKey::Commit(name) => {
                    self.workspace.update(cx, |ws, cx| {
                        ws.connections[conn].name = name;
                        cx.notify();
                    });
                    self.rename = None;
                    self.refresh(cx);
                }
                RenameKey::Cancel => self.rename = None,
            }
            cx.notify();
            if rename_swallows(m) {
                cx.stop_propagation();
            }
            return;
        }
        let route = route_key(
            Focus::Tree,
            &ev.keystroke.key,
            ev.keystroke.key_char.as_deref(),
            m,
            self.filter.is_empty(),
            || true,
            false,
        );
        match route {
            KeyRoute::ToField(ch) => {
                self.put_filter(format!("{}{ch}", self.filter), window, cx);
                self.focus_field(window, cx);
                return cx.stop_propagation();
            }
            KeyRoute::PopField => {
                let mut f = self.filter.clone();
                f.pop();
                return self.put_filter(f, window, cx);
            }
            KeyRoute::ClearField => return self.put_filter(String::new(), window, cx),
            _ => {}
        }
        match ev.keystroke.key.as_str() {
            "f2" if self
                .nodes
                .get(self.selected)
                .is_some_and(|n| can_rename(n.kind)) =>
            {
                let conn = self.nodes[self.selected].conn;
                self.rename = Some(Rename {
                    conn,
                    text: self.workspace.read(cx).connections[conn].name.clone(),
                    error: None,
                });
                return cx.notify();
            }
            "escape" if !self.marked.is_empty() => {
                self.marked.clear();
                return cx.notify();
            }
            "c" if m.secondary() => {
                let names: Vec<&str> = self
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(i, n)| *i == self.selected || self.marked.contains(&n.key))
                    .map(|(_, n)| n.label.as_ref())
                    .collect();
                cx.write_to_clipboard(ClipboardItem::new_string(names.join("\n")));
                return cx.stop_propagation();
            }
            "space" | "enter" | "tab" => {}
            _ if ev.keystroke.key_char.as_deref() == Some("*") && self.filter.is_empty() => {
                self.expand_all_under(self.selected, cx);
                return cx.stop_propagation();
            }
            _ => {}
        }
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };
        let ix = self.selected;
        match ev.keystroke.key.as_str() {
            "up" => self.select(ix.saturating_sub(1), cx),
            "down" => self.select((ix + 1).min(self.nodes.len() - 1), cx),
            "right" if node.expandable && (!node.expanded || self.failed(node, cx)) => {
                self.set_expanded(ix, true, cx)
            }
            "right" if node.expanded => self.select(ix + 1, cx),
            "left" if node.expanded => self.set_expanded(ix, false, cx),
            "left" => {
                if let Some(parent) = parent_of(&self.nodes, ix) {
                    self.select(parent, cx);
                }
            }
            "enter" | "space" if matches!(node.kind, NodeKind::ConnError | NodeKind::LoadError) => {
                self.retry(ix, cx)
            }
            "enter" if opens_tab(node.kind) => self.open(ix, false, cx),
            "enter" if matches!(node.kind, NodeKind::Connection(_)) => {
                self.set_expanded(ix, true, cx)
            }
            "space" if opens_tab(node.kind) => self.open(ix, true, cx),
            "enter" | "space" => self.set_expanded(ix, !node.expanded, cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}
