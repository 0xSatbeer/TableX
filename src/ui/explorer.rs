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
    menu::ContextMenuExt as _,
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
use crate::ui::object_dialog::{Apply, NameOwnerDialog, Owned};
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

/// Emitted after connection `id` was deleted, so the Shell can close its tabs.
pub struct ConnectionDeleted {
    pub id: ConnectionId,
}

impl EventEmitter<ConnectionDeleted> for Explorer {}

pub struct Explorer {
    workspace: Entity<Workspace>,
    driver: Arc<dyn Driver>,
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
}

impl Explorer {
    pub fn new(
        workspace: Entity<Workspace>,
        driver: Arc<dyn Driver>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // ponytail: starts collapsed; QX-020 restores the last session's expanded nodes
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
        }
    }
}

impl Focusable for Explorer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
