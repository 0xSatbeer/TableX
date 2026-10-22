//! An object row's actions (Open, Copy name, Copy qualified name, Show DDL, Drop…), and the
//! place a new console opens on.

use super::*;
use crate::ddl;
use crate::ui::object_dialog::DropDialog;

impl Explorer {
    /// Open on the selected table, view or materialized view row.
    pub(super) fn open_selected(&mut self, cx: &mut Context<Self>) {
        if self.selected < self.nodes.len() {
            self.open(self.selected, false, cx);
        }
    }

    /// Show DDL: opens the selected table or view and asks for its DDL (Definition) sub-tab.
    pub(super) fn show_ddl(&mut self, cx: &mut Context<Self>) {
        let Some((node, kind)) = self.selected_object() else {
            return;
        };
        if !opens_tab(NodeKind::Object(kind)) {
            return;
        }
        let key = node.key.clone();
        self.open(self.selected, false, cx);
        cx.emit(ShowDdl { key: key.into() });
    }

    pub(super) fn copy_name(&self, cx: &mut Context<Self>) {
        let Some((node, _)) = self.selected_object() else {
            return;
        };
        let name = segments(&node.key).swap_remove(4);
        cx.write_to_clipboard(ClipboardItem::new_string(name));
    }

    /// `"schema"."name"` on Postgres (a function with its argument types), `"name"` on SQLite.
    pub(super) fn copy_qualified_name(&self, cx: &mut Context<Self>) {
        let Some((node, kind)) = self.selected_object() else {
            return;
        };
        let (schema, name) = self.sql_name(node, cx);
        let text = ddl::object_ref(kind, schema.as_deref(), &name);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Drop…: confirms the DROP, runs it atomically on the object's database, then re-reads its
    /// schema and tells the Shell to close the object's tabs.
    pub(super) fn drop_object(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((node, kind)) = self.selected_object() else {
            return;
        };
        let (schema, name) = self.sql_name(node, cx);
        let Some(statement) = ddl::drop_object(kind, schema.as_deref(), &name) else {
            return;
        };
        let conn = &self.workspace.read(cx).connections[node.conn];
        let (id, production) = (conn.id, conn.is_production);
        let key = node.key.clone();
        let db = segments(&key).swap_remove(1);
        let schema_key = ancestors(&key)[2].to_string();
        let dialog = DropDialog::open(&name, statement, production, window, cx);
        cx.subscribe_in(
            &dialog,
            window,
            move |this, dialog, Apply(statements), window, cx| {
                let (dialog, key) = (dialog.downgrade(), key.clone());
                this.apply(
                    id,
                    &db,
                    true,
                    Some(schema_key.clone()),
                    move |result, cx| match result {
                        Ok(()) => cx.emit(ObjectDropped { key }),
                        Err(reason) => {
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

    /// Where New console (Ctrl+Shift+N) opens: the picked row's connection, database and schema.
    pub fn console_place(&self, cx: &App) -> Option<(ConnectionId, String, Option<String>)> {
        let node = self.nodes.get(self.selected).filter(|_| self.picked)?;
        let conn = &self.workspace.read(cx).connections[node.conn];
        let mut parts = segments(&node.key).into_iter().skip(1);
        match node.kind {
            NodeKind::Loading | NodeKind::ConnError | NodeKind::LoadError => None,
            NodeKind::Connection(_) => Some((conn.id, conn.params.database(), None)),
            _ => Some((conn.id, parts.next()?, parts.next())),
        }
    }

    fn selected_object(&self) -> Option<(&FlatNode, ObjectKind)> {
        let node = self.nodes.get(self.selected)?;
        let NodeKind::Object(kind) = node.kind else {
            return None;
        };
        Some((node, kind))
    }

    /// The object row's schema as SQL qualifies it (none on SQLite), and its name.
    fn sql_name(&self, node: &FlatNode, cx: &App) -> (Option<String>, String) {
        let postgres = self.workspace.read(cx).connections[node.conn].kind() == DbKind::Postgres;
        let mut parts = segments(&node.key);
        let name = parts.swap_remove(4);
        (postgres.then(|| parts.swap_remove(2)), name)
    }
}
