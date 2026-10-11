//! Explorer tree: the visible nodes flattened into a `Vec<FlatNode>` and rendered with
//! `uniform_list`. The flat list is rebuilt only on expand or collapse.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    menu::ContextMenuExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{Connection, ConnectionId, ConnectionParams, ConnectionStatus, DbKind, DbObject, Key, ObjectKind, Workspace};
use crate::connection_dialog::{ConnectionDialog, Saved};
use crate::tabs::Origin;
use crate::{assets, mock, theme};

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
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Columns,
    Keys,
    Indexes,
}

/// The trimmed name to commit, or the message to show inline. `others` are the other connections'
/// names; uniqueness is case-sensitive.
pub(crate) fn validate_rename(input: &str, others: &[&str]) -> Result<String, String> {
    let name = input.trim();
    if name.is_empty() {
        Err("Name can't be empty".into())
    } else if others.contains(&name) {
        Err(format!("Another connection is already named {name}"))
    } else {
        Ok(name.into())
    }
}

/// Whether the status dot is solid (else a ring), and the tooltip naming the status.
fn status_style(status: &ConnectionStatus) -> (bool, String) {
    match status {
        ConnectionStatus::Connected => (true, "Connected".into()),
        ConnectionStatus::Disconnected => (false, "Disconnected".into()),
        ConnectionStatus::Connecting => (false, "Connecting…".into()),
        ConnectionStatus::Error(msg) => (false, format!("Error: {msg}")),
    }
}

/// The nodes to show: the expanded tree, or the filtered full tree while a filter is typed.
fn tree_nodes(ws: &Workspace, expanded: &HashSet<String>, loading: &HashSet<String>, filter: &str) -> Vec<FlatNode> {
    if filter.is_empty() {
        flatten(ws, expanded, loading, false)
    } else {
        filter_nodes(flatten(ws, expanded, &HashSet::new(), true), filter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Focus {
    Tree,
    Field,
}

/// What a keystroke does to the filter field.
#[derive(Clone, Debug, PartialEq)]
enum KeyRoute {
    /// Not ours: normal tree / input handling.
    Pass,
    /// Tree-focused printable key: focus the field and append this text.
    ToField(String),
    /// Tree-focused Backspace with text in the field: drop its last character.
    PopField,
    ClearField,
    FocusTree,
    FocusFirstMatch,
}

fn route_key(focus: Focus, key: &str, key_char: Option<&str>, m: Modifiers, field_empty: bool, has_match: impl FnOnce() -> bool, renaming: bool) -> KeyRoute {
    if focus == Focus::Field {
        return match key {
            "escape" if field_empty => KeyRoute::FocusTree,
            "escape" => KeyRoute::ClearField,
            "down" if !field_empty && !m.shift && !m.control && !m.alt && !m.platform && has_match() => KeyRoute::FocusFirstMatch,
            _ => KeyRoute::Pass,
        };
    }
    if renaming {
        return KeyRoute::Pass;
    }
    let printable = key_char.filter(|c| !c.chars().any(char::is_control) && *c != " ");
    match key {
        "escape" if !field_empty => KeyRoute::ClearField,
        "backspace" if !field_empty => KeyRoute::PopField,
        // "*" stays the expand-all key while there is no filter.
        _ => match printable {
            Some(c) if !m.control && !m.alt && !m.platform && !(c == "*" && field_empty) => KeyRoute::ToField(c.into()),
            _ => KeyRoute::Pass,
        },
    }
}

/// The single muted row shown when a filter matches nothing.
fn no_match_text(filter: &str) -> String {
    format!("No objects match \"{filter}\"")
}

/// Index of the first node matching a non-empty filter.
fn first_match(nodes: &[FlatNode], filter: &str) -> Option<usize> {
    nodes.iter().position(|n| !filter.is_empty() && fuzzy_match(&n.label, filter).is_some())
}

/// Byte ranges of the characters in `label` that the filter matched, for highlighting.
fn highlight_ranges(label: &str, filter: &str) -> Vec<Range<usize>> {
    fuzzy_match(label, filter)
        .unwrap_or_default()
        .into_iter()
        .map(|at| at..at + label[at..].chars().next().map_or(1, char::len_utf8))
        .collect()
}

/// Where a mouse press landed on a row.
#[derive(Clone, Copy, Debug)]
enum Hit {
    Chevron,
    Row { clicks: usize },
}

/// What a press does after selecting the row.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Click {
    Select,
    Toggle,
    Open { preview: bool },
}

fn click_action(kind: NodeKind, hit: Hit) -> Click {
    match hit {
        Hit::Chevron => Click::Toggle,
        Hit::Row { clicks } if opens_tab(kind) => Click::Open { preview: clicks < 2 },
        Hit::Row { clicks: 2 } => Click::Toggle,
        Hit::Row { .. } => Click::Select,
    }
}

/// Table-like objects open in a tab and expand into their structure.
fn opens_tab(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Object(ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView))
}

/// A row estimate rounded for the tree: `999`, `1.2k`, `12k`, `1.2M`.
fn row_estimate_label(rows: Option<u64>) -> Option<String> {
    let rows = rows?;
    if rows < 1000 {
        return Some(rows.to_string());
    }
    let mut scaled = rows as f64;
    for unit in ["k", "M", "B"] {
        scaled /= 1000.0;
        let rounded = if scaled < 10.0 { (scaled * 10.0).round() / 10.0 } else { scaled.round() };
        // 999_999 rounds to 1000k; carry it to the next unit instead.
        if rounded < 1000.0 || unit == "B" {
            return Some(format!("{rounded}{unit}"));
        }
    }
    unreachable!("the last unit always returns")
}

/// The rows under a structure part: label, muted suffix, primary key.
fn part_rows(obj: &DbObject, part: Part) -> Vec<(&str, Option<&str>, bool)> {
    match part {
        Part::Columns => obj.columns.iter().map(|c| (c.name.as_str(), Some(c.data_type.as_str()), c.is_pk)).collect(),
        Part::Keys => obj
            .keys
            .iter()
            .map(|k| match k {
                Key::Primary { name, .. } | Key::Foreign { name, .. } | Key::Unique { name, .. } => (name.as_str(), None, false),
            })
            .collect(),
        Part::Indexes => obj.indexes.iter().map(|i| (i.name.as_str(), None, false)).collect(),
    }
}

fn part_label(part: Part) -> &'static str {
    match part {
        Part::Columns => "Columns",
        Part::Keys => "Keys",
        Part::Indexes => "Indexes",
    }
}

/// Names of every connection except `conn`, which a rename may keep.
fn other_names(ws: &Workspace, conn: usize) -> Vec<&str> {
    ws.connections.iter().enumerate().filter(|(i, _)| *i != conn).map(|(_, c)| c.name.as_str()).collect()
}

/// Whether a key during a rename is consumed; chords pass through to the Shell (Ctrl+1..9).
fn rename_swallows(m: Modifiers) -> bool {
    !m.control && !m.alt && !m.platform
}

/// F2 renames connections only.
fn can_rename(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Connection(_))
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

impl Rename {
    fn key(&mut self, key: &str, key_char: Option<&str>, others: &[&str]) -> RenameKey {
        match key {
            "escape" => return RenameKey::Cancel,
            "enter" => match validate_rename(&self.text, others) {
                Ok(name) => return RenameKey::Commit(name),
                Err(msg) => self.error = Some(msg),
            },
            "backspace" => {
                self.text.pop();
                self.error = None;
            }
            _ => {
                if let Some(ch) = key_char.filter(|c| !c.chars().any(char::is_control)) {
                    self.text.push_str(ch);
                    self.error = None;
                }
            }
        }
        RenameKey::Open
    }
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
}

fn group_label(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Table => "Tables",
        ObjectKind::View => "Views",
        ObjectKind::MaterializedView => "Materialized views",
        ObjectKind::Sequence => "Sequences",
        ObjectKind::Function => "Functions",
        ObjectKind::Procedure => "Procedures",
        ObjectKind::Type => "Types",
        ObjectKind::Index => "Indexes",
        ObjectKind::Trigger => "Triggers",
    }
}

/// Test fixture: production open down to its first schema's tables. The app itself starts
/// collapsed, so every connection is visible and production is never the default context.
#[cfg(test)]
fn default_expanded(ws: &Workspace) -> HashSet<String> {
    let mut set = HashSet::new();
    for (ci, conn) in ws.connections.iter().enumerate().filter(|(_, c)| c.is_production) {
        set.insert(format!("{ci}"));
        if let Some(schema) = conn.databases.first().and_then(|db| db.schemas.first()) {
            set.insert(format!("{ci}/0"));
            set.insert(format!("{ci}/0/0"));
            if schema.objects.iter().any(|o| o.kind == ObjectKind::Table) {
                set.insert(format!("{ci}/0/0/{}", ObjectKind::Table.as_str()));
            }
        }
    }
    set
}

/// Where a connection points, in the user's terms.
fn target(conn: &Connection) -> String {
    match &conn.params {
        ConnectionParams::Postgres { host, port, .. } => format!("{host}:{port}"),
        ConnectionParams::Sqlite { path } => path.display().to_string(),
    }
}

/// Visible nodes in preorder. `all` opens every node (used while filtering). Expanded schemas
/// in `loading` show a single loading row instead of their objects.
fn flatten(ws: &Workspace, expanded: &HashSet<String>, loading: &HashSet<String>, all: bool) -> Vec<FlatNode> {
    let mut out = Vec::new();
    let mut push = |mut node: FlatNode| {
        // Filtering opens everything but structure, so a query doesn't list every table's columns.
        let structure = matches!(node.kind, NodeKind::Object(_) | NodeKind::Part(_));
        node.expanded = node.expandable && (expanded.contains(&node.key) || all && !structure);
        let open = node.expanded;
        out.push(node);
        open
    };
    for (ci, conn) in ws.connections.iter().enumerate() {
        let node = |key: String, kind, label: &str, depth| FlatNode {
            key,
            kind,
            label: label.to_string().into(),
            depth,
            conn: ci,
            count: None,
            expandable: false,
            expanded: false,
            suffix: None,
            pk: false,
        };
        let key = format!("{ci}");
        // Expanding a connection that isn't connected connects it, even with no databases yet.
        let expandable = !conn.databases.is_empty() || conn.status != ConnectionStatus::Connected;
        let open = push(FlatNode { expandable, ..node(key.clone(), NodeKind::Connection(conn.kind()), &conn.name, 0) });
        match &conn.status {
            ConnectionStatus::Connected => {}
            ConnectionStatus::Error(msg) => {
                push(node(format!("{key}/error"), NodeKind::ConnError, msg, 1));
                continue;
            }
            ConnectionStatus::Connecting if open => {
                push(node(format!("{key}/loading"), NodeKind::Loading, &format!("Connecting to {}…", target(conn)), 1));
                continue;
            }
            ConnectionStatus::Connecting | ConnectionStatus::Disconnected => continue,
        }
        if !open {
            continue;
        }
        for (di, db) in conn.databases.iter().enumerate() {
            let key = format!("{key}/{di}");
            if !push(FlatNode { expandable: !db.schemas.is_empty(), ..node(key.clone(), NodeKind::Database, &db.name, 1) }) {
                continue;
            }
            for (si, schema) in db.schemas.iter().enumerate() {
                let key = format!("{key}/{si}");
                if !push(FlatNode { expandable: !schema.objects.is_empty(), ..node(key.clone(), NodeKind::Schema, &schema.name, 2) }) {
                    continue;
                }
                if loading.contains(&key) {
                    push(node(format!("{key}/loading"), NodeKind::Loading, "Loading objects…", 3));
                    continue;
                }
                let mut groups: BTreeMap<ObjectKind, Vec<&DbObject>> = BTreeMap::new();
                for obj in &schema.objects {
                    groups.entry(obj.kind).or_default().push(obj);
                }
                for (kind, objects) in groups {
                    let key = format!("{key}/{}", kind.as_str());
                    let group = node(key.clone(), NodeKind::Group(kind), group_label(kind), 3);
                    if !push(FlatNode { count: Some(objects.len()), expandable: true, ..group }) {
                        continue;
                    }
                    for (oi, obj) in objects.into_iter().enumerate() {
                        let key = format!("{key}/{oi}");
                        let kind = NodeKind::Object(kind);
                        let suffix = row_estimate_label(obj.row_estimate).map(Into::into);
                        let parts = [(Part::Columns, obj.columns.len()), (Part::Keys, obj.keys.len()), (Part::Indexes, obj.indexes.len())];
                        let parts: Vec<_> = parts.into_iter().filter(|(_, n)| *n > 0).collect();
                        let expandable = opens_tab(kind) && !parts.is_empty();
                        if !push(FlatNode { expandable, suffix, ..node(key.clone(), kind, &obj.name, 4) }) {
                            continue;
                        }
                        for (part, n) in parts {
                            let label = part_label(part);
                            let key = format!("{key}/{label}");
                            if !push(FlatNode { count: Some(n), expandable: true, ..node(key.clone(), NodeKind::Part(part), label, 5) }) {
                                continue;
                            }
                            for (i, (label, suffix, pk)) in part_rows(obj, part).into_iter().enumerate() {
                                push(FlatNode { suffix: suffix.map(Into::into), pk, ..node(format!("{key}/{i}"), NodeKind::Item(part), label, 6) });
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// Byte offsets of a case-insensitive subsequence match of `query` in `label`.
fn fuzzy_match(label: &str, query: &str) -> Option<Vec<usize>> {
    let mut hits = Vec::new();
    let mut chars = label.char_indices();
    for q in query.chars().flat_map(char::to_lowercase) {
        let (at, _) = chars.find(|(_, c)| c.to_lowercase().eq(std::iter::once(q)))?;
        hits.push(at);
    }
    Some(hits)
}

/// Keeps matching nodes and their ancestors, dropping the rest. Relies on preorder: walking
/// backwards, the first node shallower than a kept one is its parent.
fn filter_nodes(nodes: Vec<FlatNode>, query: &str) -> Vec<FlatNode> {
    let mut want: Option<usize> = None;
    let mut keep: Vec<FlatNode> = nodes
        .into_iter()
        .rev()
        .filter(|n| {
            if fuzzy_match(&n.label, query).is_some() {
                want = Some(n.depth);
                true
            } else if want.is_some_and(|w| n.depth < w) {
                want = Some(n.depth);
                true
            } else {
                false
            }
        })
        .collect();
    keep.reverse();
    keep
}

/// Index of the nearest node above `ix` with a smaller depth.
fn parent_of(nodes: &[FlatNode], ix: usize) -> Option<usize> {
    let depth = nodes[ix].depth;
    nodes[..ix].iter().rposition(|n| n.depth < depth)
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

pub struct Explorer {
    workspace: Entity<Workspace>,
    expanded: HashSet<String>,
    /// Schema keys whose objects have "arrived", and those still waiting on mock latency.
    loaded: HashSet<String>,
    loading: HashSet<String>,
    /// The latest connect started on each connection; only that one may land.
    connects: HashMap<ConnectionId, usize>,
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
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // ponytail: starts collapsed; QX-020 restores the last session's expanded nodes
        let expanded = HashSet::new();
        let nodes = flatten(workspace.read(cx), &expanded, &HashSet::new(), false);
        let loaded = HashSet::new();
        // Edits made elsewhere (rename, recolor, connect) rebuild the rows.
        cx.observe(&workspace, |this, _, cx| this.refresh(cx)).detach();
        let field = cx.new(|cx| InputState::new(window, cx).placeholder("Filter objects…"));
        cx.subscribe(&field, |this, field, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                let text = field.read(cx).value().to_string();
                this.set_filter(text, cx);
            }
        })
        .detach();
        Self { field, workspace, expanded, loaded, loading: HashSet::new(), connects: HashMap::new(), filter: String::new(), nodes, selected: 0, picked: false, marked: HashSet::new(), rename: None, scroll: UniformListScrollHandle::new(), focus: cx.focus_handle() }
    }

    fn set_expanded(&mut self, ix: usize, open: bool, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        if !node.expandable || node.expanded == open {
            return;
        }
        let key = node.key.clone();
        if open && matches!(node.kind, NodeKind::Connection(_)) && self.workspace.read(cx).connections[node.conn].status != ConnectionStatus::Connected {
            let ci = node.conn;
            return self.connect(ci, cx);
        }
        if open && node.kind == NodeKind::Schema && !self.loaded.contains(&key) && self.loading.insert(key.clone()) {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(mock::latency()).await;
                this.update(cx, |this, cx| {
                    this.loading.remove(&key);
                    this.loaded.insert(key);
                    this.refresh(cx);
                })
                .ok();
            })
            .detach();
        }
        if open {
            self.expanded.insert(node.key.clone());
        } else {
            self.expanded.remove(&node.key);
        }
        self.refresh(cx);
    }

    /// Starts a mock connect that lands after `mock::latency()`. Expanding a connection that is
    /// not connected and Retry on a failed one both come here.
    fn connect(&mut self, ci: usize, cx: &mut Context<Self>) {
        let started = self.workspace.update(cx, |ws, cx| {
            let conn = &mut ws.connections[ci];
            if matches!(conn.status, ConnectionStatus::Connected | ConnectionStatus::Connecting) {
                return None;
            }
            conn.status = ConnectionStatus::Connecting;
            cx.notify();
            Some(conn.id)
        });
        let Some(id) = started else { return };
        let token = self.connects.entry(id).or_default();
        *token += 1;
        let token = *token;
        self.expanded.insert(format!("{ci}"));
        // ponytail: every mock connect succeeds, including retries; drivers report real failures (QX-023)
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(mock::latency()).await;
            this.update(cx, |this, cx| {
                if this.connects.get(&id) != Some(&token) {
                    return; // a later connect owns the result
                }
                this.workspace.update(cx, |ws, cx| {
                    // A params edit disconnects it, so Connecting means none happened since this connect began.
                    let Some(conn) = ws.connections.iter_mut().find(|c| c.id == id) else { return };
                    if conn.status == ConnectionStatus::Connecting {
                        conn.status = ConnectionStatus::Connected;
                        cx.notify();
                    }
                });
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
        self.refresh(cx);
    }

    /// Opens every ancestor of `key`, selects it and focuses the tree.
    pub fn reveal(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let parts: Vec<&str> = key.split('/').collect();
        for depth in 1..parts.len() {
            let ancestor = parts[..depth].join("/");
            if depth == 3 {
                self.loaded.insert(ancestor.clone());
            }
            self.expanded.insert(ancestor);
        }
        self.put_filter(String::new(), window, cx);
        if let Some(ix) = self.nodes.iter().position(|n| n.key == key) {
            self.select(ix, cx);
        }
        window.focus(&self.focus, cx);
    }

    /// `*`: opens the node and everything under it, skipping the loading step.
    fn expand_all_under(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(root) = self.nodes.get(ix).map(|n| n.key.clone()) else { return };
        let prefix = format!("{root}/");
        let all = flatten(self.workspace.read(cx), &HashSet::new(), &HashSet::new(), true);
        for n in all.into_iter().filter(|n| n.expandable && (n.key.starts_with(&prefix) || n.key == root)) {
            if n.kind == NodeKind::Schema {
                self.loaded.insert(n.key.clone());
            }
            self.expanded.insert(n.key);
        }
        self.refresh(cx);
    }

    /// Opens the Add connection dialog ("+" in the header, `AddConnection`).
    pub fn add_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_dialog(None, window, cx);
    }

    /// Opens the Edit dialog for the selected connection row (Edit… in its menu, `EditConnection`).
    pub fn edit_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected).filter(|n| matches!(n.kind, NodeKind::Connection(_))) else { return };
        let id = self.workspace.read(cx).connections[node.conn].id;
        self.open_dialog(Some(id), window, cx);
    }

    fn open_dialog(&mut self, editing: Option<ConnectionId>, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = ConnectionDialog::open(self.workspace.clone(), editing, window, cx);
        cx.subscribe_in(&dialog, window, |this, _, Saved(id), window, cx| {
            let connections = &this.workspace.read(cx).connections;
            if let Some(ix) = connections.iter().position(|c| c.id == *id) {
                // Collapse a connection the edit disconnected, so expanding it connects again.
                if connections[ix].status == ConnectionStatus::Disconnected {
                    this.expanded.remove(&ix.to_string());
                }
                this.reveal(&ix.to_string(), window, cx);
            }
        })
        .detach();
    }

    /// Focuses the filter field (Ctrl+Alt+F).
    pub fn focus_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.field.focus_handle(cx), cx);
    }

    /// Replaces the field's text and applies it as the filter (`set_value` emits no Change).
    fn put_filter(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.field.update(cx, |f, cx| f.set_value(text.clone(), window, cx));
        self.set_filter(text, cx);
    }

    /// Esc / Down while the field has focus; runs in the capture phase so the input never sees them.
    fn on_field_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let route = route_key(Focus::Field, &ev.keystroke.key, None, ev.keystroke.modifiers, self.filter.is_empty(), || first_match(&self.nodes, &self.filter).is_some(), false);
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

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.nodes = tree_nodes(self.workspace.read(cx), &self.expanded, &self.loading, &self.filter);
        self.selected = self.selected.min(self.nodes.len().saturating_sub(1));
        cx.notify();
    }

    fn set_filter(&mut self, filter: String, cx: &mut Context<Self>) {
        self.filter = filter;
        self.nodes = tree_nodes(self.workspace.read(cx), &self.expanded, &self.loading, &self.filter);
        // First match, or the top when clearing.
        let first = first_match(&self.nodes, &self.filter);
        self.select(first.unwrap_or(0), cx);
    }

    fn open(&mut self, ix: usize, preview: bool, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        if opens_tab(node.kind) {
            let conn = &self.workspace.read(cx).connections[node.conn];
            let (connection, color, production) = (conn.id, conn.color, conn.is_production);
            let mut names = vec![node.label.to_string()];
            let mut at = ix;
            while let Some(parent) = parent_of(&self.nodes, at) {
                // Groups ("Tables") are UI only, not part of the object path.
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
                    Origin { connection, color, production, path: crate::tabs::join_path(&names[0], &tail), tail: tail.into() }
                },
                path: names.join(" › ").into(),
                preview,
            });
        }
    }

    /// A plain (unmodified) press: select the row, then do what `click_action` says.
    fn press(&mut self, ix: usize, hit: Hit, cx: &mut Context<Self>) {
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

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        (self.selected, self.picked) = (ix, true);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = ev.keystroke.modifiers;
        if self.field.focus_handle(cx).is_focused(window) {
            return;
        }
        if let Some(rn) = &mut self.rename {
            let conn = rn.conn;
            let others = other_names(self.workspace.read(cx), conn);
            match rn.key(&ev.keystroke.key, ev.keystroke.key_char.as_deref().filter(|_| !m.control && !m.alt && !m.platform), &others) {
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
        let route = route_key(Focus::Tree, &ev.keystroke.key, ev.keystroke.key_char.as_deref(), m, self.filter.is_empty(), || true, false);
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
            "f2" if self.nodes.get(self.selected).is_some_and(|n| can_rename(n.kind)) => {
                let conn = self.nodes[self.selected].conn;
                self.rename = Some(Rename { conn, text: self.workspace.read(cx).connections[conn].name.clone(), error: None });
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
        let Some(node) = self.nodes.get(self.selected) else { return };
        let ix = self.selected;
        match ev.keystroke.key.as_str() {
            "up" => self.select(ix.saturating_sub(1), cx),
            "down" => self.select((ix + 1).min(self.nodes.len() - 1), cx),
            "right" if node.expandable && !node.expanded => self.set_expanded(ix, true, cx),
            "right" if node.expanded => self.select(ix + 1, cx),
            "left" if node.expanded => self.set_expanded(ix, false, cx),
            "left" => {
                if let Some(parent) = parent_of(&self.nodes, ix) {
                    self.select(parent, cx);
                }
            }
            "enter" | "space" if node.kind == NodeKind::ConnError => self.connect(node.conn, cx),
            "enter" if opens_tab(node.kind) => self.open(ix, false, cx),
            "enter" if matches!(node.kind, NodeKind::Connection(_)) => self.set_expanded(ix, true, cx),
            "space" if opens_tab(node.kind) => self.open(ix, true, cx),
            "enter" | "space" => self.set_expanded(ix, !node.expanded, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let focused = self.focus.contains_focused(window, cx);
        let ws = self.workspace.read(cx);
        let range_kinds: Vec<bool> = range.clone().map(|ix| matches!(self.nodes[ix].kind, NodeKind::Connection(_))).collect();
        range
            .map(|ix| {
                let node = &self.nodes[ix];
                let (icon, color) = match node.kind {
                    NodeKind::Connection(DbKind::Postgres) => {
                        (Icon::empty().path(assets::POSTGRES_ICON), theme::brand_color(DbKind::Postgres, theme))
                    }
                    NodeKind::Connection(DbKind::Sqlite) => (Icon::empty().path(assets::SQLITE_ICON), theme::brand_color(DbKind::Sqlite, theme)),
                    NodeKind::Database => (Icon::new(Lucide::Database), theme.yellow),
                    NodeKind::Schema => (Icon::new(Lucide::Layers), theme.magenta),
                    NodeKind::Group(kind) => (
                        Icon::empty().path(assets::object_icon(kind.as_str(), true)),
                        theme::object_color(kind, theme).opacity(0.7),
                    ),
                    NodeKind::Object(kind) => {
                        (Icon::empty().path(assets::object_icon(kind.as_str(), false)), theme::object_color(kind, theme))
                    }
                    NodeKind::Part(part) | NodeKind::Item(part) => (
                        match part {
                            Part::Columns => Icon::new(Lucide::Columns3),
                            Part::Keys => Icon::new(Lucide::KeyRound),
                            Part::Indexes => Icon::empty().path(assets::object_icon(ObjectKind::Index.as_str(), false)),
                        },
                        theme.muted_foreground,
                    ),
                    NodeKind::Loading => (Icon::new(Lucide::LoaderCircle), theme.muted_foreground),
                    NodeKind::ConnError => (Icon::new(Lucide::CircleAlert), theme.danger),
                };
                let conn = &ws.connections[node.conn];
                let is_conn = matches!(node.kind, NodeKind::Connection(_));
                let chevron = if node.expanded { Lucide::ChevronDown } else { Lucide::ChevronRight };
                let selected = ix == self.selected && (self.picked || focused);
                h_flex()
                    .id(ix)
                    .relative()
                    .h(theme::tree_row())
                    .w_full()
                    .gap(theme::space_6())
                    .pl(theme::space_4() + theme::space_12() * node.depth as f32)
                    .pr(theme::space_8())
                    .when(!selected && self.marked.contains(&node.key), |d| d.bg(theme::selected_inactive(theme)))
                    .when(selected, |d| d.bg(if focused { theme.list_active } else { theme::selected_inactive(theme) }))
                    .when(!selected, |d| d.hover(|d| d.bg(theme.list_hover)))
                    // An overlay, so selection and hover don't wipe out production.
                    .when(conn.is_production, |d| d.child(div().absolute().inset_0().bg(theme::production_tint(theme))))
                    // Indent guides, one hairline per ancestor level.
                    .children((1..node.depth).map(|level| {
                        div()
                            .absolute()
                            .left(theme::space_4() + theme::space_12() * level as f32 + theme::space_6())
                            .top_0()
                            .h_full()
                            .w(px(1.))
                            .bg(theme.border)
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.rename = None;
                            let m = ev.modifiers;
                            if m.secondary() || m.shift {
                                let range = if m.shift { this.selected.min(ix)..=this.selected.max(ix) } else { ix..=ix };
                                for key in range.map(|i| this.nodes[i].key.clone()) {
                                    if !this.marked.insert(key.clone()) && m.secondary() {
                                        this.marked.remove(&key);
                                    }
                                }
                                return cx.notify();
                            }
                            this.press(ix, Hit::Row { clicks: ev.click_count }, cx);
                        }),
                    )
                    // The row's menu acts on the selection, so a right-click selects first.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, window, cx| {
                            window.focus(&this.focus, cx);
                            this.rename = None;
                            this.marked.clear();
                            this.select(ix, cx);
                        }),
                    )
                    .child(
                        div().flex_none().w(theme::space_12()).when(node.expandable, |d| {
                            d.child(Icon::new(chevron).xsmall().text_color(theme.muted_foreground)).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.focus, cx);
                                    this.rename = None;
                                    this.press(ix, Hit::Chevron, cx);
                                    cx.stop_propagation();
                                }),
                            )
                        }),
                    )
                    .child(icon.small().text_color(color))
                    .when(is_conn, |d| {
                        // Solid = connected, 2px ring otherwise; an error also has its own row below.
                        let color = theme::connection_color(conn.color, theme);
                        let dot = div().flex_none().size(theme::space_8()).rounded_full();
                        d.child(if status_style(&conn.status).0 { dot.bg(color) } else { dot.border_2().border_color(color) })
                    })
                    .when(is_conn, |d| {
                        let tip: SharedString = status_style(&conn.status).1.into();
                        d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })
                    .child(div().debug_selector(|| format!("tree-label-{ix}")).flex_1().min_w_0().truncate().child({
                        // shortcut: the rename input is the typed text plus a caret glyph, no cursor movement; AltGr-as-alt characters and IME input aren't supported
                        if let Some(rn) = self.rename.as_ref().filter(|rn| is_conn && rn.conn == node.conn) {
                            StyledText::new(format!("{}▏", rn.text)).into_any_element()
                        } else {
                        let style = HighlightStyle { color: Some(theme.primary), ..Default::default() };
                        let ranges = highlight_ranges(&node.label, &self.filter).into_iter().map(|r| (r, style));
                        let text = StyledText::new(node.label.clone()).with_highlights(ranges.collect::<Vec<_>>());
                        theme::truncating(("tree-label", ix), text, node.label.clone()).into_any_element()
                        }
                    }).when(matches!(node.kind, NodeKind::Loading | NodeKind::ConnError), |d| d.text_size(theme::ui_sm()).text_color(theme.muted_foreground)))
                    .when(node.kind == NodeKind::ConnError, |d| {
                        let help: SharedString = format!(
                            "{}\nCheck that {} is reachable (VPN, firewall), then retry. Ctrl+C copies this message.",
                            node.label,
                            target(conn)
                        )
                        .into();
                        let ci = node.conn;
                        d.tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx)).child(
                            Button::new(("retry", ix))
                                .ghost()
                                .small()
                                .label("Retry")
                                .on_click(cx.listener(move |this, _, _, cx| this.connect(ci, cx))),
                        )
                    })
                    .when_some(node.suffix.clone(), |d, suffix| {
                        d.child(div().flex_none().text_size(theme::ui_sm()).text_color(theme.muted_foreground).child(suffix))
                    })
                    .when(node.pk, |d| d.child(div().flex_none().text_size(theme::ui_sm()).text_color(theme.muted_foreground).child("PK")))
                    .when_some(node.count, |d, n| {
                        d.child(div().text_size(theme::ui_sm()).text_color(theme.muted_foreground).child(n.to_string()))
                    })
                    .when(is_conn && conn.is_production, |d| {
                        d.child(theme::prod_badge(theme))
                    })
            })
            .zip(range_kinds)
            .map(|(row, is_conn)| if is_conn { row.context_menu(|menu, _, _| menu.menu("Edit…", Box::new(crate::EditConnection))).into_any_element() } else { row.into_any_element() })
            .collect()
    }
}

impl Focusable for Explorer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Explorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                h_flex()
                    .flex_none()
                    .gap(theme::space_4())
                    .p(theme::space_6())
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .capture_key_down(cx.listener(Self::on_field_key))
                            .child(Input::new(&self.field).small().prefix(Icon::new(Lucide::Search).xsmall().text_color(theme.muted_foreground))),
                    )
                    .child(
                        div().debug_selector(|| "add-connection".into()).child(
                            Button::new("add-connection")
                                .ghost()
                                .small()
                                .icon(Lucide::Plus)
                                .tooltip("Add connection")
                                .on_click(cx.listener(|this, _, window, cx| this.add_connection(window, cx))),
                        ),
                    ),
            )
            .when_some(self.rename.as_ref().and_then(|rn| rn.error.clone()), |d, msg| {
                d.child(
                    h_flex()
                        .flex_none()
                        .h(theme::tree_row())
                        .px(theme::space_8())
                        .border_b_1()
                        .border_color(theme.border)
                        .text_size(theme::ui_sm())
                        .text_color(theme.danger)
                        .child(msg),
                )
            })
            .map(|d| if self.nodes.is_empty() && !self.filter.is_empty() {
                d.child(
                    div()
                        .flex_none()
                        .px(theme::space_8())
                        .h(theme::tree_row())
                        .flex()
                        .items_center()
                        .text_size(theme::ui_sm())
                        .text_color(theme.muted_foreground)
                        .child(no_match_text(&self.filter)),
                )
            } else {
                d.child(
                    uniform_list("explorer-rows", self.nodes.len(), cx.processor(Self::render_rows))
                        .track_scroll(&self.scroll)
                        .flex_1(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{row_estimate_label, Part, Click, Hit, click_action, NodeKind, default_expanded, filter_nodes, flatten, fuzzy_match, parent_of, validate_rename, Rename, RenameKey, can_rename, status_style, tree_nodes, rename_swallows, highlight_ranges, first_match, route_key, Focus, KeyRoute, no_match_text};
    use crate::{mock, model::{ConnectionStatus, DbKind, ObjectKind}};

    #[test]
    fn flatten_follows_expanded_set() {
        let ws = &mock::workspaces()[0];
        let collapsed = flatten(ws, &Default::default(), &Default::default(), false);
        let failed = ws.connections.iter().filter(|c| matches!(c.status, ConnectionStatus::Error(_))).count();
        assert!(failed > 0, "mock has a failed connection");
        assert_eq!(collapsed.len(), ws.connections.len() + failed, "connections, plus the message under each failed one");
        assert_eq!(collapsed.iter().filter(|n| n.kind == NodeKind::ConnError).count(), failed);

        let mut open = default_expanded(ws);
        let nodes = flatten(ws, &open, &Default::default(), false);
        let tables = nodes.iter().position(|n| n.kind == NodeKind::Group(ObjectKind::Table)).expect("tables group");
        let count = nodes[tables].count.unwrap();
        assert!(nodes[tables + 1..=tables + count].iter().all(|n| n.kind == NodeKind::Object(ObjectKind::Table)));
        assert_eq!(parent_of(&nodes, tables + 1), Some(tables));

        let schema = &nodes[parent_of(&nodes, tables).unwrap()].key;
        let loading = flatten(ws, &open, &[schema.clone()].into(), false);
        assert_eq!(loading.iter().filter(|n| n.kind == NodeKind::Loading).count(), 1, "loading schema shows one placeholder");
        assert!(loading.iter().all(|n| n.kind != NodeKind::Group(ObjectKind::Table) || !n.key.starts_with(schema.as_str())));

        open.remove(&nodes[tables].key);
        assert_eq!(flatten(ws, &open, &Default::default(), false).len(), nodes.len() - count, "collapsing hides exactly the children");
    }

    #[test]
    fn only_connected_connections_show_children() {
        let mut ws = mock::workspaces()[0].clone();
        let all = |ws: &crate::model::Workspace| flatten(ws, &Default::default(), &Default::default(), true);
        let ci = ws.connections.iter().position(|c| c.status == ConnectionStatus::Disconnected).unwrap();
        let under = |nodes: &[super::FlatNode]| nodes.iter().filter(|n| n.key.starts_with(&format!("{ci}/"))).cloned().collect::<Vec<_>>();
        assert!(under(&all(&ws)).is_empty(), "disconnected: no children even when open");

        ws.connections[ci].status = ConnectionStatus::Connecting;
        let connecting = under(&all(&ws));
        assert_eq!(connecting.len(), 1);
        assert_eq!(connecting[0].kind, NodeKind::Loading);
        assert!(connecting[0].label.starts_with("Connecting to "));

        ws.connections[ci].status = ConnectionStatus::Connected;
        assert!(under(&all(&ws)).iter().any(|n| n.kind == NodeKind::Database));

        ws.connections[ci].status = ConnectionStatus::Error("boom".into());
        let failed = under(&all(&ws));
        assert_eq!((failed.len(), failed[0].kind, failed[0].label.as_ref()), (1, NodeKind::ConnError, "boom"));
    }

    #[test]
    fn filter_keeps_matches_and_ancestors() {
        assert_eq!(fuzzy_match("order_items", "oit"), Some(vec![0, 6, 7]));
        assert_eq!(fuzzy_match("Users", "us"), Some(vec![0, 1]));
        assert_eq!(fuzzy_match("users", "x"), None);

        let ws = &mock::workspaces()[0];
        let all = flatten(ws, &Default::default(), &Default::default(), true);
        let target = all.iter().find(|n| matches!(n.kind, NodeKind::Object(_))).unwrap().label.to_string();
        let kept = filter_nodes(all.clone(), &target);
        let at = kept.iter().position(|n| n.label.as_ref() == target).expect("match kept");
        // Every kept node is a match or an ancestor of one, and the match has its full chain above it.
        assert_eq!((0..kept[at].depth).filter(|d| kept[..at].iter().any(|n| n.depth == *d)).count(), kept[at].depth);
        assert!(kept.len() < all.len());
    }

    #[test]
    fn validate_rename_cases() {
        let others = ["local.db", "staging-pg"];
        let cases = [
            ("  prod-main ", Ok("prod-main")),
            ("", Err("Name can't be empty")),
            ("   ", Err("Name can't be empty")),
            ("local.db", Err("Another connection is already named local.db")),
            (" local.db ", Err("Another connection is already named local.db")),
            ("Prod-PG", Ok("Prod-PG")),
            ("Local.db", Ok("Local.db")),
        ];
        for (input, want) in cases {
            let want = want.map(String::from).map_err(String::from);
            assert_eq!(validate_rename(input, &others), want, "input {input:?}");
        }
    }

    #[test]
    fn rename_keys() {
        let others = ["local.db"];
        let mut rn = Rename { conn: 0, text: "prod-pg".into(), error: None };
        // Typing edits the buffer and the input stays open.
        assert_eq!(rn.key("x", Some("x"), &others), RenameKey::Open);
        assert_eq!(rn.text, "prod-pgx");
        assert_eq!(rn.key("backspace", None, &others), RenameKey::Open);
        assert_eq!(rn.text, "prod-pg");
        // Enter commits the trimmed name.
        rn.text = "  prod-main ".into();
        assert_eq!(rn.key("enter", None, &others), RenameKey::Commit("prod-main".into()));
        // A rejected name keeps the input open with the message; typing clears it.
        rn.text = "local.db".into();
        assert_eq!(rn.key("enter", None, &others), RenameKey::Open);
        assert_eq!(rn.error.as_deref(), Some("Another connection is already named local.db"));
        rn.text.clear();
        assert_eq!(rn.key("enter", None, &others), RenameKey::Open);
        assert_eq!(rn.error.as_deref(), Some("Name can't be empty"));
        assert_eq!(rn.key("a", Some("a"), &others), RenameKey::Open);
        assert_eq!(rn.error, None);
        // Esc cancels.
        assert_eq!(rn.key("escape", None, &others), RenameKey::Cancel);
    }

    #[test]
    fn rename_to_own_name_is_allowed() {
        let ws = &mock::workspaces()[0];
        let own = ws.connections[0].name.clone();
        let others = super::other_names(ws, 0);
        assert!(!others.contains(&own.as_str()));
        assert_eq!(others.len(), ws.connections.len() - 1);
        assert_eq!(validate_rename(&own, &others), Ok(own.clone()));
    }

    #[test]
    fn renamed_connection_shows_under_filter() {
        let mut ws = mock::workspaces()[0].clone();
        let filter = "prod";
        ws.connections[0].name = "prod-main".into();
        let nodes = tree_nodes(&ws, &Default::default(), &Default::default(), filter);
        let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_ref()).collect();
        assert!(labels.contains(&ws.connections[0].name.as_str()), "{labels:?}");
        assert!(!labels.contains(&ws.connections[2].name.as_str()), "filter still applied: {labels:?}");
    }

    #[test]
    fn rename_lets_chords_through() {
        use gpui_kit::Modifiers;
        assert!(rename_swallows(Modifiers::none()));
        assert!(rename_swallows(Modifiers::shift()));
        assert!(!rename_swallows(Modifiers::control()));
        assert!(!rename_swallows(Modifiers::alt()));
        assert!(!rename_swallows(Modifiers::command()));
    }

    #[test]
    fn f2_renames_connections_only() {
        let cases = [
            (NodeKind::Connection(DbKind::Postgres), true),
            (NodeKind::Connection(DbKind::Sqlite), true),
            (NodeKind::Database, false),
            (NodeKind::Schema, false),
            (NodeKind::Group(ObjectKind::Table), false),
            (NodeKind::Object(ObjectKind::Table), false),
            (NodeKind::Object(ObjectKind::View), false),
            (NodeKind::Loading, false),
            (NodeKind::ConnError, false),
        ];
        for (kind, want) in cases {
            assert_eq!(can_rename(kind), want, "{kind:?}");
        }
    }

    #[test]
    fn status_dot_and_tooltip() {
        let cases = [
            (ConnectionStatus::Connected, true, "Connected"),
            (ConnectionStatus::Disconnected, false, "Disconnected"),
            (ConnectionStatus::Error("timeout".into()), false, "Error: timeout"),
            (ConnectionStatus::Connecting, false, "Connecting…"),
        ];
        for (status, solid, tip) in cases {
            assert_eq!(status_style(&status), (solid, tip.to_string()), "{status:?}");
        }
    }

    #[test]
    fn group_click_selects_chevron_or_double_click_toggles() {
        let containers = [
            NodeKind::Group(ObjectKind::Table),
            NodeKind::Group(ObjectKind::View),
            NodeKind::Connection(DbKind::Postgres),
            NodeKind::Database,
            NodeKind::Schema,
        ];
        for kind in containers {
            let cases = [
                (Hit::Row { clicks: 1 }, Click::Select),
                (Hit::Chevron, Click::Toggle),
                (Hit::Row { clicks: 2 }, Click::Toggle),
                (Hit::Row { clicks: 3 }, Click::Select),
            ];
            for (hit, want) in cases {
                assert_eq!(click_action(kind, hit), want, "{kind:?} {hit:?}");
            }
        }
        for kind in [NodeKind::Loading, NodeKind::ConnError] {
            assert_eq!(click_action(kind, Hit::Row { clicks: 1 }), Click::Select, "{kind:?}");
        }
    }

    #[test]
    fn table_click_opens_chevron_shows_structure() {
        for kind in [ObjectKind::Table, ObjectKind::View] {
            let kind = NodeKind::Object(kind);
            let cases = [
                (Hit::Row { clicks: 1 }, Click::Open { preview: true }),
                (Hit::Row { clicks: 2 }, Click::Open { preview: false }),
                (Hit::Chevron, Click::Toggle),
            ];
            for (hit, want) in cases {
                assert_eq!(click_action(kind, hit), want, "{kind:?} {hit:?}");
            }
        }
        for kind in [NodeKind::Part(Part::Columns), NodeKind::Item(Part::Columns)] {
            assert_eq!(click_action(kind, Hit::Row { clicks: 1 }), Click::Select, "{kind:?}");
        }

        let ws = &mock::workspaces()[0];
        let mut open = default_expanded(ws);
        let nodes = flatten(ws, &open, &Default::default(), false);
        let table = nodes.iter().position(|n| n.kind == NodeKind::Object(ObjectKind::Table)).unwrap();
        assert!(nodes[table].expandable && !nodes[table].expanded, "a table has a chevron, closed by default");
        assert_eq!(nodes[table + 1].kind, NodeKind::Object(ObjectKind::Table), "children not expanded");

        open.insert(nodes[table].key.clone());
        let nodes = flatten(ws, &open, &Default::default(), false);
        let parts: Vec<_> = nodes[table + 1..].iter().take_while(|n| n.depth > nodes[table].depth).map(|n| (n.kind, n.label.to_string())).collect();
        assert_eq!(
            parts,
            [
                (NodeKind::Part(Part::Columns), "Columns".to_string()),
                (NodeKind::Part(Part::Keys), "Keys".to_string()),
                (NodeKind::Part(Part::Indexes), "Indexes".to_string()),
            ]
        );
    }

    #[test]
    fn row_estimate_labels() {
        let cases = [
            (None, None),
            (Some(0), Some("0")),
            (Some(1), Some("1")),
            (Some(999), Some("999")),
            (Some(1000), Some("1k")),
            (Some(1234), Some("1.2k")),
            (Some(1950), Some("2k")),
            (Some(9_999), Some("10k")),
            (Some(10_000), Some("10k")),
            (Some(12_345), Some("12k")),
            (Some(99_999), Some("100k")),
            (Some(999_499), Some("999k")),
            (Some(999_500), Some("1M")),
            (Some(999_999), Some("1M")),
            (Some(1_000_000), Some("1M")),
            (Some(1_234_567), Some("1.2M")),
            (Some(12_345_678), Some("12M")),
            (Some(1_500_000_000), Some("1.5B")),
        ];
        for (rows, want) in cases {
            assert_eq!(row_estimate_label(rows).as_deref(), want, "{rows:?}");
        }

        let ws = &mock::workspaces()[0];
        let nodes = flatten(ws, &Default::default(), &Default::default(), true);
        let first = |kind| nodes.iter().find(|n| n.kind == NodeKind::Object(kind)).unwrap();
        let table = first(ObjectKind::Table);
        let conn = ws.connections.iter().find(|c| c.status == ConnectionStatus::Connected).unwrap();
        let obj = conn.databases[0].schemas[0].objects.iter().find(|o| o.kind == ObjectKind::Table).unwrap();
        assert_eq!(obj.name, table.label.as_ref());
        let estimate = obj.row_estimate;
        assert_eq!(table.suffix.as_deref(), row_estimate_label(estimate).as_deref(), "table row shows its estimate");
        assert!(table.suffix.is_some());
        assert_eq!(first(ObjectKind::View).suffix, None, "views have no estimate");
    }

    #[test]
    fn structure_omits_empty_parts() {
        let mut ws = mock::workspaces()[0].clone();
        let conn = ws.connections.iter_mut().find(|c| c.status == ConnectionStatus::Connected).unwrap();
        let obj = conn.databases[0].schemas[0].objects.iter_mut().find(|o| o.kind == ObjectKind::Table).unwrap();
        obj.columns.truncate(3);
        obj.keys.truncate(1);
        obj.indexes.clear();
        let name = obj.name.clone();

        let all = flatten(&ws, &Default::default(), &Default::default(), true);
        let table = all.iter().find(|n| n.label.as_ref() == name).unwrap();
        let nodes = flatten(&ws, &[table.key.clone()].into(), &Default::default(), true);
        let at = nodes.iter().position(|n| n.key == table.key).unwrap();
        let children: Vec<_> = nodes[at + 1..].iter().take_while(|n| n.depth > table.depth).map(|n| (n.label.to_string(), n.count)).collect();
        assert_eq!(children, [("Columns".to_string(), Some(3)), ("Keys".to_string(), Some(1))]);

        let conn = ws.connections.iter_mut().find(|c| c.status == ConnectionStatus::Connected).unwrap();
        let obj = conn.databases[0].schemas[0].objects.iter_mut().find(|o| o.name == name).unwrap();
        (obj.columns.clear(), obj.keys.clear());
        let nodes = flatten(&ws, &[table.key.clone()].into(), &Default::default(), true);
        let empty = nodes.iter().find(|n| n.key == table.key).unwrap();
        assert!(!empty.expandable, "nothing to show, no chevron");
    }

    #[test]
    fn column_rows_show_type_and_pk() {
        let ws = &mock::workspaces()[0];
        let conn = ws.connections.iter().find(|c| c.status == ConnectionStatus::Connected).unwrap();
        let obj = conn.databases[0].schemas[0].objects.iter().find(|o| o.kind == ObjectKind::Table).unwrap();
        let table = flatten(ws, &Default::default(), &Default::default(), true).into_iter().find(|n| n.label.as_ref() == obj.name).unwrap();
        let open = ["", "/Columns", "/Keys", "/Indexes"].map(|part| format!("{}{part}", table.key)).into();
        let nodes = flatten(ws, &open, &Default::default(), true);
        let items = |part| nodes.iter().filter(|n| n.kind == NodeKind::Item(part) && n.key.starts_with(&table.key)).collect::<Vec<_>>();

        let columns = items(Part::Columns);
        let got: Vec<_> = columns.iter().map(|n| (n.label.to_string(), n.suffix.as_ref().map(|s| s.to_string()), n.pk)).collect();
        let want: Vec<_> = obj.columns.iter().map(|c| (c.name.clone(), Some(c.data_type.clone()), c.is_pk)).collect();
        assert_eq!(got, want);
        assert_eq!(got[0], ("id".to_string(), Some("bigint".to_string()), true), "id bigint PK");
        assert!(got[1..].iter().all(|(_, _, pk)| !pk), "only id is a PK");

        let labels = |part| items(part).iter().map(|n| n.label.to_string()).collect::<Vec<_>>();
        assert_eq!(labels(Part::Keys), obj.keys.iter().map(|k| match k {
            crate::model::Key::Primary { name, .. } | crate::model::Key::Foreign { name, .. } | crate::model::Key::Unique { name, .. } => name.clone(),
        }).collect::<Vec<_>>());
        assert_eq!(labels(Part::Indexes), obj.indexes.iter().map(|i| i.name.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn typing_ordr_shows_orders_highlighted_with_ancestors() {
        let ws = &mock::workspaces()[0];
        let nodes = tree_nodes(ws, &Default::default(), &Default::default(), "ordr");
        let at = nodes.iter().position(|n| n.label.as_ref() == "orders").expect("orders shown");
        assert_eq!(highlight_ranges("orders", "ordr"), vec![0..1, 1..2, 2..3, 4..5]);
        assert_eq!(highlight_ranges("orders", ""), Vec::<std::ops::Range<usize>>::new());
        assert_eq!(highlight_ranges("users", "ordr"), Vec::<std::ops::Range<usize>>::new());
        // The real parent chain (connection > database > schema > Tables group), not just any shallower rows.
        let mut chain = vec![];
        let mut cur = at;
        while let Some(p) = parent_of(&nodes, cur) {
            chain.push(nodes[p].key.to_string());
            cur = p;
        }
        chain.reverse();
        let key = nodes[at].key.to_string();
        let want: Vec<String> = key.match_indices('/').take(4).map(|(i, _)| key[..i].to_string()).collect();
        assert_eq!(chain, want, "ancestors of orders are its own parent chain");
        assert_eq!(chain.len(), 4);
    }

    #[test]
    fn empty_field_restores_the_expanded_set() {
        let ws = &mock::workspaces()[0];
        let expanded = default_expanded(ws);
        let none = Default::default();
        let normal = flatten(ws, &expanded, &none, false);
        let keys = |v: &[super::FlatNode]| v.iter().map(|n| n.key.clone()).collect::<Vec<_>>();
        let filtered = tree_nodes(ws, &expanded, &none, "ordr");
        assert_ne!(keys(&filtered), keys(&normal), "filter changes the tree");
        assert_eq!(keys(&tree_nodes(ws, &expanded, &none, "")), keys(&normal));
        assert_eq!(first_match(&normal, ""), None, "no first match without a filter");
        assert_eq!(first_match(&filtered, "ordr").map(|i| filtered[i].label.as_ref()), Some("orders"));
        assert_eq!(first_match(&filtered, "zzzz"), None);
    }

    #[test]
    fn typing_in_the_tree_moves_into_the_field() {
        use gpui_kit::Modifiers as M;
        use KeyRoute::*;
        let ch = |c: &'static str| Some(c);
        // (key, key_char, modifiers, field_empty, renaming, expected)
        let cases = [
            ("u", ch("u"), M::none(), true, false, ToField("u".into())),
            ("u", ch("U"), M::shift(), true, false, ToField("U".into())),
            ("u", ch("u"), M::none(), false, false, ToField("u".into())),
            ("u", ch("u"), M::control(), true, false, Pass),
            ("u", ch("u"), M::alt(), true, false, Pass),
            ("u", ch("u"), M::command(), true, false, Pass),
            ("u", ch("u"), M::none(), true, true, Pass),
            ("space", ch(" "), M::none(), true, false, Pass),
            ("enter", ch("\n"), M::none(), true, false, Pass),
            ("*", ch("*"), M::none(), true, false, Pass),
            ("*", ch("*"), M::none(), false, false, ToField("*".into())),
            ("up", None, M::none(), true, false, Pass),
            ("backspace", None, M::none(), true, false, Pass),
            ("backspace", None, M::none(), false, false, PopField),
            ("escape", None, M::none(), false, false, ClearField),
            ("escape", None, M::none(), true, false, Pass),
        ];
        for (key, key_char, m, empty, renaming, want) in cases {
            assert_eq!(route_key(Focus::Tree, key, key_char, m, empty, || true, renaming), want, "tree {key:?} {key_char:?} {m:?} empty={empty} renaming={renaming}");
        }
    }

    #[test]
    fn field_keys_escape_and_down() {
        use gpui_kit::Modifiers as M;
        use KeyRoute::*;
        // (key, modifiers, field_empty, has_match, expected)
        let cases = [
            ("escape", M::none(), false, true, ClearField),
            ("escape", M::none(), false, false, ClearField),
            ("escape", M::none(), true, false, FocusTree),
            ("escape", M::none(), true, true, FocusTree),
            ("down", M::none(), false, true, FocusFirstMatch),
            ("down", M::none(), false, false, Pass),
            ("down", M::none(), true, true, Pass),
            ("down", M::shift(), false, true, Pass),
            ("u", M::none(), false, true, Pass),
            ("backspace", M::none(), false, true, Pass),
        ];
        for (key, m, empty, has_match, want) in cases {
            assert_eq!(route_key(Focus::Field, key, Some("x").filter(|_| key == "u"), m, empty, || has_match, false), want, "field {key:?} empty={empty} match={has_match}");
        }
        // Only "down" needs the (costly) match scan.
        for key in ["escape", "u", "backspace", "left"] {
            route_key(Focus::Field, key, None, M::none(), false, || panic!("{key:?} must not compute first_match"), false);
        }
        // Esc twice: first clears, second leaves for the tree.
        let (mut text, mut seq) = ("ord".to_string(), vec![]);
        for _ in 0..2 {
            let r = route_key(Focus::Field, "escape", None, M::none(), text.is_empty(), || true, false);
            if r == ClearField {
                text.clear();
            }
            seq.push(r);
        }
        assert_eq!(seq, vec![ClearField, FocusTree]);
        // F2 rename in progress keeps its keys even if Esc reaches the router.
        assert_eq!(route_key(Focus::Tree, "escape", None, M::none(), false, || true, true), Pass);
    }

    #[test]
    fn no_match_shows_one_muted_row() {
        let ws = &mock::workspaces()[0];
        assert!(tree_nodes(ws, &Default::default(), &Default::default(), "xyz").is_empty(), "nothing survives the filter");
        assert_eq!(no_match_text("xyz"), "No objects match \"xyz\"");
        assert_eq!(no_match_text("a b"), "No objects match \"a b\"");
    }

    #[gpui_kit::test]
    fn reveal_clears_the_field_text_and_the_filter(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Entity};
        cx.update(gpui_kit::init);
        let ws = mock::workspaces().remove(0);
        let key = flatten(&ws, &default_expanded(&ws), &Default::default(), true).into_iter().find(|n| n.label.as_ref() == "orders").expect("orders").key;
        let (window, explorer): (_, Entity<super::Explorer>) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Explorer::new(cx.new(|_| ws), window, cx))).expect("open test window")
        });
        cx.update_window(window.into(), |_, window, cx| {
            explorer.update(cx, |e, cx| {
                e.put_filter("ordr".into(), window, cx);
                e.reveal(&key, window, cx);
                assert_eq!(e.field.read(cx).value().to_string(), "", "field text cleared");
                assert_eq!(e.filter, "", "filter cleared");
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn truncated_tree_names_show_their_full_text_in_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Bounds, Entity, WindowBounds, WindowOptions, point, px, size};
        cx.update(gpui_kit::init);
        let long = "shipment_line_item_carrier_tracking_events_archive_partitioned_by_month_2026_q3x";
        assert_eq!(long.len(), 80);
        // (explorer width, table name, tooltip shown)
        let cases = [(260., long, true), (260., "orders", false), (1200., long, false)];
        for (width, name, tip) in cases {
            let mut ws = mock::workspaces().remove(0);
            let table = ws.connections[0].databases[0].schemas[0].objects.iter_mut().find(|o| o.name == "orders").expect("orders");
            table.name = name.to_string();
            let key = flatten(&ws, &default_expanded(&ws), &Default::default(), true).into_iter().find(|n| n.label.as_ref() == name).expect("table row").key;
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(width), px(600.))))),
                ..Default::default()
            };
            let (window, explorer): (_, Entity<super::Explorer>) = cx.update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| super::Explorer::new(cx.new(|_| ws), window, cx))).expect("open test window")
            });
            let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
            explorer.update_in(&mut cx, |e, window, cx| e.reveal(&key, window, cx));
            cx.run_until_parked();
            let ix = explorer.read_with(&cx, |e, _| e.selected);
            let shown = crate::theme::tests::hover_shows_truncation_tip(&mut cx, format!("tree-label-{ix}"));
            assert_eq!(shown, tip, "{} chars at {width}px: tooltip shown", name.len());
        }
    }
}
