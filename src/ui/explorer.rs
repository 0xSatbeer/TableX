//! Explorer tree: the visible nodes flattened into a `Vec<FlatNode>` and rendered with
//! `uniform_list`. The flat list is rebuilt only on expand or collapse.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    spinner::Spinner,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use std::sync::Arc;

use crate::db::{Driver, Session};
use crate::model::{
    Column, Connection, ConnectionId, ConnectionParams, ConnectionStatus, Database, DbKind,
    DbObject, Key, ObjectKind, Schema, Workspace,
};
use crate::ui::connection_dialog::{Confirmed, ConnectionDialog, DeleteConfirm, Saved};
use crate::ui::object_dialog::{Apply, NameOwnerDialog, Owned, RoleDialog};
use crate::ui::table_editor::TableEditor;
use crate::ui::tabs::Origin;
use crate::{assets, theme};

mod connections;
mod fetch;
mod filter;
mod flatten;
mod input;
mod keys;
mod load;
mod rename;
mod render;
mod rows;
#[cfg(test)]
mod tests;

pub(crate) use fetch::on_database;
use fetch::*;
use filter::*;
use flatten::*;
use keys::*;
pub(crate) use rename::validate_rename;
use rename::*;
pub(crate) use render::status_style;
use rows::*;

#[derive(Clone, Copy, Debug, PartialEq)]
enum NodeKind {
    Connection(DbKind),
    Database,
    Schema,
    Group(ObjectKind),
    Object(ObjectKind),
    /// A table's or view's Columns, Keys or Indexes group.
    Part(Part),
    /// One column, key or index under a `Part`.
    Item(Part),
    /// Placeholder under a connection that is connecting or a schema whose objects are loading.
    Loading,
    /// A failed connection's message, shown under it whether or not it is expanded.
    ConnError,
    /// A failed load's message, in place of the children it was loading.
    LoadError,
}

/// Where the load of a node's children stands.
#[derive(Clone, Debug, PartialEq)]
enum Load {
    Loading,
    /// Reloading children already shown, which stay until the new ones arrive.
    Refreshing,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Columns,
    Keys,
    Indexes,
}

/// An inline rename in progress on a connection row.
struct Rename {
    conn: usize,
    text: String,
    error: Option<String>,
}

#[derive(Debug, PartialEq)]
enum RenameKey {
    Open,
    Commit(String),
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
struct FlatNode {
    /// Stable path key (`conn/db/schema/group/object`, by index), used for the expanded set.
    key: String,
    kind: NodeKind,
    label: SharedString,
    depth: usize,
    /// Index into `Workspace::connections`, for the color dot.
    conn: usize,
    /// Child count on groups.
    count: Option<usize>,
    expandable: bool,
    expanded: bool,
    /// Muted text after the label: a table's row estimate or a column's type.
    suffix: Option<SharedString>,
    /// A primary-key column.
    pk: bool,
    /// Its children are not loaded yet, so expanding it loads them.
    unloaded: bool,
}

/// A table-like object to show in a tab. `preview` is false on double-click or Enter.
pub struct OpenObject {
    pub key: SharedString,
    pub name: SharedString,
    pub origin: Origin,
    /// `connection › db › schema › object`
    pub path: SharedString,
    pub preview: bool,
}

impl EventEmitter<OpenObject> for Explorer {}

/// Open console on a database or schema row: a console tab bound to that place.
pub struct OpenConsole {
    pub conn: ConnectionId,
    pub database: String,
    pub schema: Option<String>,
}

impl EventEmitter<OpenConsole> for Explorer {}

/// Emitted after connection `id` was deleted, so the Shell can close its tabs.
pub struct ConnectionDeleted {
    pub id: ConnectionId,
}

impl EventEmitter<ConnectionDeleted> for Explorer {}

/// "Move to ▸ <workspace>" on connection `id`: the Shell moves it to workspace `to`.
pub struct MoveConnection {
    pub id: ConnectionId,
    pub to: u32,
}

impl EventEmitter<MoveConnection> for Explorer {}

pub struct Explorer {
    workspace: Entity<Workspace>,
    driver: Arc<dyn Driver>,
    /// The window the password prompt opens over.
    window: AnyWindowHandle,
    expanded: HashSet<String>,
    /// Nodes whose children are loading or failed to.
    loads: HashMap<String, Load>,
    /// Each connection's generation: every connect bumps it, and a result carrying an older one is dropped.
    gens: HashMap<ConnectionId, u64>,
    /// Open sessions by connection and database.
    sessions: HashMap<(ConnectionId, String), Session>,
    /// Each connection's params at the last workspace change, to spot edits and deletes made anywhere.
    params: HashMap<ConnectionId, ConnectionParams>,
    /// Mirror of the field's text; non-empty shows only matches and their ancestors.
    filter: String,
    field: Entity<InputState>,
    nodes: Vec<FlatNode>,
    selected: usize,
    /// False until the user picks a row, so launch doesn't highlight production as "here".
    picked: bool,
    /// Extra rows picked with Ctrl/Shift+click, by key so they survive re-flattening.
    marked: HashSet<String>,
    /// Inline rename of a connection, started with F2.
    rename: Option<Rename>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// The other workspaces, by id and name, for a connection's Move to submenu.
    move_targets: Vec<(u32, SharedString)>,
    /// Set by quick open: a database that finishes loading loads its schemas' objects too.
    load_all: bool,
}

impl Explorer {
    pub fn new(
        workspace: Entity<Workspace>,
        driver: Arc<dyn Driver>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Collapsed until the Shell restores the session's expanded nodes (`set_expanded_paths`).
        let expanded = HashSet::new();
        let nodes = flatten(workspace.read(cx), &expanded, &HashMap::new(), false);
        // Edits made elsewhere (rename, recolor, connect, new params, delete) rebuild the rows.
        cx.observe(&workspace, |this, _, cx| {
            this.forget_changed(cx);
            this.refresh(cx);
        })
        .detach();
        let params = workspace
            .read(cx)
            .connections
            .iter()
            .map(|c| (c.id, c.params.clone()))
            .collect();
        let field = cx.new(|cx| InputState::new(window, cx).placeholder("Filter objects…"));
        cx.subscribe(&field, |this, field, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                let text = field.read(cx).value().to_string();
                this.set_filter(text, cx);
            }
        })
        .detach();
        Self {
            window: window.window_handle(),
            field,
            workspace,
            driver,
            expanded,
            loads: HashMap::new(),
            gens: HashMap::new(),
            sessions: HashMap::new(),
            params,
            filter: String::new(),
            nodes,
            selected: 0,
            picked: false,
            marked: HashSet::new(),
            rename: None,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            move_targets: Vec::new(),
            load_all: false,
        }
    }

    /// The workspaces a connection's Move to submenu lists, by id and name.
    pub fn set_move_targets(&mut self, targets: Vec<(u32, SharedString)>, cx: &mut Context<Self>) {
        if self.move_targets != targets {
            self.move_targets = targets;
            cx.notify();
        }
    }
}

impl Explorer {
    /// The expanded nodes as the session file names them: by name, a group by its label ("Tables").
    pub fn expanded_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .expanded
            .iter()
            .map(|key| {
                with_group(key, |s| {
                    ObjectKind::ALL
                        .into_iter()
                        .find(|k| k.as_str() == s)
                        .map(group_label)
                })
            })
            .collect();
        paths.sort();
        paths
    }

    /// Expands the nodes `expanded_paths` gave; each opens as its parent's children load.
    pub fn set_expanded_paths(&mut self, paths: &[String], cx: &mut Context<Self>) {
        self.expanded = paths
            .iter()
            .map(|path| {
                with_group(path, |s| {
                    ObjectKind::ALL
                        .into_iter()
                        .find(|k| group_label(*k) == s)
                        .map(ObjectKind::as_str)
                })
            })
            .collect();
        self.refresh(cx);
    }

    /// Connects connection `id` without opening its row, as restoring a session's tabs does.
    pub fn connect_collapsed(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        let key = id.0.to_string();
        let open = self.expanded.contains(&key);
        self.connect_connection(id, cx);
        if !open {
            self.expanded.remove(&key);
            self.refresh(cx);
        }
    }
}

/// `key` with its group segment (the 4th) replaced by what `map` gives for it, if anything.
fn with_group(key: &str, map: impl Fn(&str) -> Option<&'static str>) -> String {
    let mut parts = segments(key);
    if let Some(group) = parts.get_mut(3).and_then(|s| Some((map(s)?, s))) {
        *group.1 = group.0.to_string();
    }
    parts.iter().map(|p| seg(p)).collect::<Vec<_>>().join("/")
}

/// The tree key of object `name` of `kind` in `database`.`schema` on connection `id`.
pub(crate) fn object_key(
    id: ConnectionId,
    database: &str,
    schema: &str,
    kind: ObjectKind,
    name: &str,
) -> String {
    format!(
        "{}/{}/{}/{}/{}",
        id.0,
        seg(database),
        seg(schema),
        kind.as_str(),
        seg(name)
    )
}

impl Focusable for Explorer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
