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

use std::sync::Arc;

use crate::db::{Driver, Session};
use crate::model::{Column, Connection, Database, Schema, ConnectionId, ConnectionParams, ConnectionStatus, DbKind, DbObject, Key, ObjectKind, Workspace};
use crate::connection_dialog::{Confirmed, ConnectionDialog, DeleteConfirm, Saved};
use crate::tabs::Origin;
use crate::{assets, theme};

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
    Failed(String),
}

/// The row shown in place of `key`'s children while they load or after their load failed.
fn load_row(loads: &HashMap<String, Load>, key: &str) -> Option<(String, NodeKind, String)> {
    match loads.get(key)? {
        Load::Loading => Some((format!("{key}/{LOADING}"), NodeKind::Loading, "Loading…".into())),
        Load::Failed(reason) => Some((format!("{key}/{ERROR}"), NodeKind::LoadError, format!("Could not load: {reason}"))),
    }
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

/// `name` as one tree-key segment: `\` and `/` escaped, so a name never splits or merges segments.
fn seg(name: &str) -> String {
    name.replace('\\', r"\\").replace('/', r"\/")
}

/// The keys of `key`'s ancestors, outermost first: its prefixes up to each unescaped `/`.
fn ancestors(key: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut escaped = false;
    for (i, c) in key.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '/' => out.push(&key[..i]),
            _ => {}
        }
    }
    out
}

/// `key`'s segments with their escapes undone: connection id, database, schema, kind, object, ...
fn segments(key: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = key.chars();
    while let Some(c) = chars.next() {
        let last = out.last_mut().expect("never empty");
        match c {
            '\\' => last.extend(chars.next()),
            '/' => out.push(String::new()),
            c => last.push(c),
        }
    }
    out
}

/// `params` pointed at database `db`: a Postgres server's other database, or the SQLite file itself.
fn on_database(params: &ConnectionParams, db: &str) -> ConnectionParams {
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

/// What a load of one tree level brings back.
enum Children {
    Databases(Vec<String>),
    Schemas(Vec<String>),
    Objects(Vec<DbObject>),
    Columns(Vec<Column>),
}

/// Starts the load of the children of the node whose key has `segments`.
fn fetch(driver: &dyn Driver, session: &Session, segments: &[String]) -> futures::future::BoxFuture<'static, Result<Children, String>> {
    use futures::TryFutureExt as _;
    match segments {
        [_] => Box::pin(driver.databases(session).map_ok(Children::Databases)),
        [_, _] => Box::pin(driver.schemas(session).map_ok(Children::Schemas)),
        [_, _, schema] => Box::pin(driver.objects(session, schema).map_ok(Children::Objects)),
        [_, _, schema, _, object] => Box::pin(driver.columns(session, schema, object).map_ok(Children::Columns)),
        _ => Box::pin(futures::future::ready(Err("nothing to load".to_string()))),
    }
}

/// A node of the tree whose children load: connection, database, schema or table-like object.
enum NodeMut<'a> {
    Connection(&'a mut Connection),
    Database(&'a mut Database),
    Schema(&'a mut Schema),
    Object(&'a mut DbObject),
}

/// The node with `segments` in connection `id`'s tree.
fn locate<'a>(ws: &'a mut Workspace, id: ConnectionId, segments: &[String]) -> Option<NodeMut<'a>> {
    let conn = ws.connections.iter_mut().find(|c| c.id == id)?;
    let Some(name) = segments.get(1) else { return Some(NodeMut::Connection(conn)) };
    let db = conn.databases.iter_mut().flatten().find(|d| &d.name == name)?;
    let Some(name) = segments.get(2) else { return Some(NodeMut::Database(db)) };
    let schema = db.schemas.iter_mut().flatten().find(|s| &s.name == name)?;
    let (Some(kind), Some(name)) = (segments.get(3), segments.get(4)) else { return Some(NodeMut::Schema(schema)) };
    schema.objects.iter_mut().flatten().find(|o| o.kind.as_str() == kind && &o.name == name).map(NodeMut::Object)
}

/// Puts `children` in connection `id`'s tree, under the node with `segments`. Returns the key
/// tails (`name`, or `kind/name` for objects) of that node's children before and after, so the caller can follow what came and went.
/// A child that is still there and that `keep` names by its tail keeps its own loaded children until they are re-read.
fn store(ws: &mut Workspace, id: ConnectionId, segments: &[String], children: Children, keep: impl Fn(&str) -> bool) -> (Vec<String>, Vec<String>) {
    let db_tail = |d: &Database| seg(&d.name);
    let schema_tail = |s: &Schema| seg(&s.name);
    let object_tail = |o: &DbObject| format!("{}/{}", o.kind.as_str(), seg(&o.name));
    match (locate(ws, id, segments), children) {
        (Some(NodeMut::Connection(conn)), Children::Databases(names)) => {
            let mut old = conn.databases.take().unwrap_or_default();
            let mut kept = |name: &str| old.iter_mut().find(|d| d.name == name && keep(&db_tail(d))).and_then(|d| d.schemas.take());
            let new = names.into_iter().map(|name| Database { schemas: kept(&name), name }).collect::<Vec<_>>();
            let old = old.iter().map(db_tail).collect();
            (old, conn.databases.insert(new).iter().map(db_tail).collect())
        }
        (Some(NodeMut::Database(db)), Children::Schemas(names)) => {
            let mut old = db.schemas.take().unwrap_or_default();
            let mut kept = |name: &str| old.iter_mut().find(|s| s.name == name && keep(&schema_tail(s))).and_then(|s| s.objects.take());
            let new = names.into_iter().map(|name| Schema { objects: kept(&name), name }).collect::<Vec<_>>();
            let old = old.iter().map(schema_tail).collect();
            (old, db.schemas.insert(new).iter().map(schema_tail).collect())
        }
        (Some(NodeMut::Schema(schema)), Children::Objects(mut objects)) => {
            let mut old = schema.objects.take().unwrap_or_default();
            for object in &mut objects {
                let tail = object_tail(object);
                object.columns = old.iter_mut().find(|o| object_tail(o) == tail && keep(&tail)).and_then(|o| o.columns.take());
            }
            let old = old.iter().map(object_tail).collect();
            (old, schema.objects.insert(objects).iter().map(object_tail).collect())
        }
        (Some(NodeMut::Object(object)), Children::Columns(columns)) => {
            object.columns = Some(columns);
            (Vec::new(), Vec::new())
        }
        _ => (Vec::new(), Vec::new()),
    }
}

/// Whether `key` is connection `id`'s row or one under it. The id segment is a plain number.
fn of_connection(key: &str, id: ConnectionId) -> bool {
    key.split('/').next() == Some(id.0.to_string().as_str())
}

/// The last segment of a status row's key. A backslash never starts an escaped name's segment, so
/// these never clash with a database or schema named "loading" or "error".
const LOADING: &str = r"\loading";
const ERROR: &str = r"\error";

/// Whether the status dot is solid (else a ring), and the tooltip naming the status.
fn status_style(status: &ConnectionStatus) -> (bool, String) {
    match status {
        ConnectionStatus::Connected => (true, "Connected".into()),
        ConnectionStatus::Disconnected => (false, "Disconnected".into()),
        ConnectionStatus::Connecting => (false, "Connecting…".into()),
        ConnectionStatus::Error(msg) => (false, format!("Could not connect: {msg}")),
    }
}

/// The nodes to show: the expanded tree, or the filtered full tree while a filter is typed.
fn tree_nodes(ws: &Workspace, expanded: &HashSet<String>, loads: &HashMap<String, Load>, filter: &str) -> Vec<FlatNode> {
    if filter.is_empty() {
        flatten(ws, expanded, loads, false)
    } else {
        filter_nodes(flatten(ws, expanded, &HashMap::new(), true), filter)
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
        Part::Columns => obj.columns.iter().flatten().map(|c| (c.name.as_str(), Some(c.data_type.as_str()), c.is_pk)).collect(),
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
    /// Its children are not loaded yet, so expanding it loads them.
    unloaded: bool,
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
    for conn in ws.connections.iter().filter(|c| c.is_production) {
        let key = conn.id.0.to_string();
        set.insert(key.clone());
        let db = conn.databases.iter().flatten().next();
        if let Some((db, schema)) = db.and_then(|db| Some((db, db.schemas.iter().flatten().next()?))) {
            let db = format!("{key}/{}", seg(&db.name));
            let schema_key = format!("{db}/{}", seg(&schema.name));
            if schema.objects.iter().flatten().any(|o| o.kind == ObjectKind::Table) {
                set.insert(format!("{schema_key}/{}", ObjectKind::Table.as_str()));
            }
            set.extend([db, schema_key]);
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
/// in `loads` show a single loading or error row instead of their children.
fn flatten(ws: &Workspace, expanded: &HashSet<String>, loads: &HashMap<String, Load>, all: bool) -> Vec<FlatNode> {
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
            unloaded: false,
        };
        let key = conn.id.0.to_string();
        // Expanding a connection that isn't connected connects it, even with no databases yet.
        let expandable = conn.databases.as_ref().is_none_or(|d| !d.is_empty()) || conn.status != ConnectionStatus::Connected;
        let unloaded = conn.databases.is_none();
        let open = push(FlatNode { expandable, unloaded, ..node(key.clone(), NodeKind::Connection(conn.kind()), &conn.name, 0) });
        match &conn.status {
            ConnectionStatus::Connected => {}
            ConnectionStatus::Error(msg) => {
                push(node(format!("{key}/{ERROR}"), NodeKind::ConnError, &format!("Could not connect: {msg}"), 1));
                continue;
            }
            ConnectionStatus::Connecting if open => {
                push(node(format!("{key}/{LOADING}"), NodeKind::Loading, &format!("Connecting to {}…", target(conn)), 1));
                continue;
            }
            ConnectionStatus::Connecting | ConnectionStatus::Disconnected => continue,
        }
        if !open {
            continue;
        }
        // While a level loads, or once its load failed, one row says so in place of its children.
        if let Some((key, kind, label)) = load_row(loads, &key) {
            push(node(key, kind, &label, 1));
            continue;
        }
        for db in conn.databases.iter().flatten() {
            let key = format!("{key}/{}", seg(&db.name));
            let (expandable, unloaded) = (db.schemas.as_ref().is_none_or(|s| !s.is_empty()), db.schemas.is_none());
            if !push(FlatNode { expandable, unloaded, ..node(key.clone(), NodeKind::Database, &db.name, 1) }) {
                continue;
            }
            if let Some((key, kind, label)) = load_row(loads, &key) {
                push(node(key, kind, &label, 2));
                continue;
            }
            for schema in db.schemas.iter().flatten() {
                let key = format!("{key}/{}", seg(&schema.name));
                let (expandable, unloaded) = (schema.objects.as_ref().is_none_or(|o| !o.is_empty()), schema.objects.is_none());
                if !push(FlatNode { expandable, unloaded, ..node(key.clone(), NodeKind::Schema, &schema.name, 2) }) {
                    continue;
                }
                if let Some((key, kind, label)) = load_row(loads, &key) {
                    push(node(key, kind, &label, 3));
                    continue;
                }
                let mut groups: BTreeMap<ObjectKind, Vec<&DbObject>> = BTreeMap::new();
                for obj in schema.objects.iter().flatten() {
                    groups.entry(obj.kind).or_default().push(obj);
                }
                for (kind, objects) in groups {
                    let key = format!("{key}/{}", kind.as_str());
                    let group = node(key.clone(), NodeKind::Group(kind), group_label(kind), 3);
                    if !push(FlatNode { count: Some(objects.len()), expandable: true, ..group }) {
                        continue;
                    }
                    for obj in objects {
                        let key = format!("{key}/{}", seg(&obj.name));
                        let kind = NodeKind::Object(kind);
                        let suffix = row_estimate_label(obj.row_estimate).map(Into::into);
                        let parts = [(Part::Columns, obj.columns.as_ref().map_or(0, Vec::len)), (Part::Keys, obj.keys.len()), (Part::Indexes, obj.indexes.len())];
                        let parts: Vec<_> = parts.into_iter().filter(|(_, n)| *n > 0).collect();
                        let unloaded = opens_tab(kind) && obj.columns.is_none();
                        let expandable = opens_tab(kind) && !parts.is_empty() || unloaded;
                        if !push(FlatNode { expandable, unloaded, suffix, ..node(key.clone(), kind, &obj.name, 4) }) {
                            continue;
                        }
                        if let Some((key, kind, label)) = load_row(loads, &key) {
                            push(node(key, kind, &label, 5));
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
    pub fn new(workspace: Entity<Workspace>, driver: Arc<dyn Driver>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // ponytail: starts collapsed; QX-020 restores the last session's expanded nodes
        let expanded = HashSet::new();
        let nodes = flatten(workspace.read(cx), &expanded, &HashMap::new(), false);
        // Edits made elsewhere (rename, recolor, connect, new params, delete) rebuild the rows.
        cx.observe(&workspace, |this, _, cx| {
            this.forget_changed(cx);
            this.refresh(cx);
        })
        .detach();
        let params = workspace.read(cx).connections.iter().map(|c| (c.id, c.params.clone())).collect();
        let field = cx.new(|cx| InputState::new(window, cx).placeholder("Filter objects…"));
        cx.subscribe(&field, |this, field, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                let text = field.read(cx).value().to_string();
                this.set_filter(text, cx);
            }
        })
        .detach();
        Self { field, workspace, driver, expanded, loads: HashMap::new(), gens: HashMap::new(), sessions: HashMap::new(), params, filter: String::new(), nodes, selected: 0, picked: false, marked: HashSet::new(), rename: None, scroll: UniformListScrollHandle::new(), focus: cx.focus_handle() }
    }

    fn set_expanded(&mut self, ix: usize, open: bool, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        if !node.expandable {
            return;
        }
        // Before the already-open check: a failed connection stays open and still connects again.
        if open && matches!(node.kind, NodeKind::Connection(_)) && self.workspace.read(cx).connections[node.conn].status != ConnectionStatus::Connected {
            let ci = node.conn;
            return self.connect(ci, cx);
        }
        if node.expanded == open {
            return;
        }
        let key = node.key.clone();
        if open && node.unloaded {
            self.load(key.clone(), cx);
        }
        if open {
            self.expanded.insert(key);
        } else {
            self.expanded.remove(&key);
        }
        self.refresh(cx);
    }

    /// Connects connection `ci`, then lists its databases. Expanding a connection that is not
    /// connected and Retry on a failed one both come here.
    fn connect(&mut self, ci: usize, cx: &mut Context<Self>) {
        let key = self.workspace.read(cx).connections[ci].id.0.to_string();
        // Opened even when a connect is already running, so an expand while Connecting isn't lost.
        self.expanded.insert(key.clone());
        let started = self.workspace.update(cx, |ws, cx| {
            let conn = &mut ws.connections[ci];
            if matches!(conn.status, ConnectionStatus::Connected | ConnectionStatus::Connecting) {
                return None;
            }
            conn.status = ConnectionStatus::Connecting;
            cx.notify();
            Some((conn.id, conn.params.clone()))
        });
        let Some((id, params)) = started else { return self.refresh(cx) };
        let generation = self.next_gen(id);
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
                        this.load(key, cx);
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

    /// Closes the sessions of every connection whose params changed or that was deleted, drops
    /// what it was loading and collapses it, so nothing from its old server lands or stays.
    fn forget_changed(&mut self, cx: &mut Context<Self>) {
        let now: HashMap<_, _> = self.workspace.read(cx).connections.iter().map(|c| (c.id, c.params.clone())).collect();
        let changed: Vec<ConnectionId> = self.params.iter().filter(|(id, params)| now.get(id) != Some(params)).map(|(id, _)| *id).collect();
        for id in changed {
            self.next_gen(id);
            self.sessions.retain(|(conn, _), _| *conn != id);
            self.expanded.retain(|k| !of_connection(k, id));
            self.loads.retain(|k, _| !of_connection(k, id));
        }
        self.params = now;
    }

    /// Bumps connection `id`'s generation, so every result started before is dropped.
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

    /// Loads the children of the node `key` through its database's session.
    fn load(&mut self, key: String, cx: &mut Context<Self>) {
        let segments = segments(&key);
        let ws = self.workspace.read(cx);
        let Some(conn) = ws.connections.iter().find(|c| c.id.0.to_string() == segments[0]) else { return };
        let id = conn.id;
        let db = segments.get(1).cloned().unwrap_or_else(|| conn.params.database());
        if matches!(self.loads.get(&key), Some(Load::Loading)) {
            return;
        }
        // Another Postgres database needs a session of its own, opened with the same settings.
        let open: crate::db::Reply<Session> = match self.sessions.get(&(id, db.clone())) {
            Some(session) => Box::pin(futures::future::ready(Ok(session.clone()))),
            None => self.driver.connect(&on_database(&conn.params, &db)),
        };
        self.loads.insert(key.clone(), Load::Loading);
        let generation = self.gens.get(&id).copied().unwrap_or_default();
        let driver = self.driver.clone();
        cx.spawn(async move |this, cx| {
            let result = match open.await {
                Ok(session) => {
                    let current = this.update(cx, |this, _| {
                        let current = this.gens.get(&id) == Some(&generation);
                        if current {
                            this.sessions.entry((id, db.clone())).or_insert_with(|| session.clone());
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
                            store(ws, id, &segments, children, |tail| expanded.contains(&format!("{key}/{tail}")))
                        });
                        // A child that is gone takes its expansion with it, and its descendants'.
                        for tail in old.iter().filter(|t| !new.contains(t)) {
                            let child = format!("{key}/{tail}");
                            this.expanded.retain(|k| *k != child && !k.starts_with(&format!("{child}/")));
                        }
                        // Expanded children are re-read in turn, and so on down.
                        for tail in new {
                            let child = format!("{key}/{tail}");
                            if this.expanded.contains(&child) {
                                this.load(child, cx);
                            }
                        }
                    }
                    Err(reason) => {
                        // A session the server closed is dropped, so Retry opens a new one.
                        let at = (id, db);
                        if this.sessions.get(&at).is_some_and(Session::is_closed) {
                            this.sessions.remove(&at);
                        }
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
    fn retry(&mut self, ix: usize, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        match node.kind {
            NodeKind::ConnError => self.connect(node.conn, cx),
            NodeKind::LoadError => {
                let parent = node.key[..node.key.len() - ERROR.len() - 1].to_string();
                self.load(parent, cx);
            }
            _ => {}
        }
    }

    /// Refresh in row `ix`'s menu: re-reads that node's subtree.
    fn refresh_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(key) = self.nodes.get(ix).map(|n| n.key.clone()) else { return };
        self.load(key, cx);
    }

    /// Opens every ancestor of `key`, selects it and focuses the tree.
    pub fn reveal(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.expanded.extend(ancestors(key).into_iter().map(String::from));
        self.put_filter(String::new(), window, cx);
        if let Some(ix) = self.nodes.iter().position(|n| n.key == key) {
            self.select(ix, cx);
        }
        window.focus(&self.focus, cx);
    }

    /// `*`: opens the node and everything loaded under it.
    fn expand_all_under(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(root) = self.nodes.get(ix).map(|n| n.key.clone()) else { return };
        let prefix = format!("{root}/");
        let all = flatten(self.workspace.read(cx), &HashSet::new(), &HashMap::new(), true);
        for n in all.into_iter().filter(|n| n.expandable && (n.key.starts_with(&prefix) || n.key == root)) {
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

    /// Asks to confirm deleting the selected connection row (Delete in its menu, `DeleteConnection`).
    pub fn delete_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected).filter(|n| matches!(n.kind, NodeKind::Connection(_))) else { return };
        let conn = self.workspace.read(cx).connections[node.conn].clone();
        let dialog = DeleteConfirm::open(&conn, window, cx);
        cx.subscribe(&dialog, move |this, _, Confirmed, cx| this.remove_connection(conn.id, cx)).detach();
    }

    /// Deletes connection `id`. A connect still running for it finds it gone and is discarded.
    fn remove_connection(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
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
        let next = connections.get(ix.min(connections.len().saturating_sub(1))).map(|c| c.id.0.to_string());
        if let Some(row) = self.nodes.iter().position(|n| Some(&n.key) == next.as_ref()) {
            self.select(row, cx);
        }
    }

    fn open_dialog(&mut self, editing: Option<ConnectionId>, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = ConnectionDialog::open(self.workspace.clone(), editing, window, cx);
        cx.subscribe_in(&dialog, window, |this, _, Saved(id), window, cx| {
            if this.workspace.read(cx).connections.iter().any(|c| c.id == *id) {
                this.reveal(&id.0.to_string(), window, cx);
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
        self.nodes = tree_nodes(self.workspace.read(cx), &self.expanded, &self.loads, &self.filter);
        self.selected = self.selected.min(self.nodes.len().saturating_sub(1));
        cx.notify();
    }

    fn set_filter(&mut self, filter: String, cx: &mut Context<Self>) {
        self.filter = filter;
        self.nodes = tree_nodes(self.workspace.read(cx), &self.expanded, &self.loads, &self.filter);
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

    /// Whether `node` is a connection whose connect failed.
    fn failed(&self, node: &FlatNode, cx: &App) -> bool {
        matches!(node.kind, NodeKind::Connection(_)) && matches!(self.workspace.read(cx).connections[node.conn].status, ConnectionStatus::Error(_))
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
            "right" if node.expandable && (!node.expanded || self.failed(node, cx)) => self.set_expanded(ix, true, cx),
            "right" if node.expanded => self.select(ix + 1, cx),
            "left" if node.expanded => self.set_expanded(ix, false, cx),
            "left" => {
                if let Some(parent) = parent_of(&self.nodes, ix) {
                    self.select(parent, cx);
                }
            }
            "enter" | "space" if matches!(node.kind, NodeKind::ConnError | NodeKind::LoadError) => self.retry(ix, cx),
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
        // Each row's menu: Some(true) for a connection's, Some(false) for Refresh alone, None for no menu.
        let menus: Vec<Option<bool>> = range
            .clone()
            .map(|ix| match self.nodes[ix].kind {
                NodeKind::Connection(_) => Some(true),
                NodeKind::Database | NodeKind::Schema => Some(false),
                kind if opens_tab(kind) => Some(false),
                _ => None,
            })
            .collect();
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
                    NodeKind::ConnError | NodeKind::LoadError => (Icon::new(Lucide::CircleAlert), theme.danger),
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
                    }).when(matches!(node.kind, NodeKind::Loading | NodeKind::ConnError | NodeKind::LoadError), |d| d.text_size(theme::ui_sm()).text_color(theme.muted_foreground)))
                    .when(node.kind == NodeKind::ConnError, |d| {
                        let help: SharedString = format!(
                            "{}\nCheck that {} is reachable (VPN, firewall), then retry. Ctrl+C copies this message.",
                            node.label,
                            target(conn)
                        )
                        .into();
                        d.tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx))
                    })
                    .when(matches!(node.kind, NodeKind::ConnError | NodeKind::LoadError), |d| {
                        d.child(
                            div().debug_selector(move || format!("retry-{ix}")).child(
                                Button::new(("retry", ix)).ghost().small().label("Retry").on_click(cx.listener(move |this, _, _, cx| this.retry(ix, cx))),
                            ),
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
                        d.child(theme::prod_badge(theme).debug_selector(|| format!("prod-badge-{ix}")))
                    })
            })
            .zip(menus)
            .map(|(row, menu)| match menu {
                Some(is_conn) => row
                    .context_menu(move |menu, _, _| {
                        let menu = if is_conn {
                            menu.menu("Edit…", Box::new(crate::EditConnection)).menu_element(Box::new(crate::DeleteConnection), |_, cx| {
                                div().debug_selector(|| "menu-delete".into()).text_color(cx.theme().danger).child("Delete")
                            })
                        } else {
                            menu
                        };
                        menu.menu_element(Box::new(crate::RefreshNode), |_, _| div().debug_selector(|| "menu-refresh".into()).child("Refresh"))
                    })
                    .into_any_element(),
                None => row.into_any_element(),
            })
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
            .on_action(cx.listener(|this, _: &crate::RefreshNode, _, cx| this.refresh_row(this.selected, cx)))
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
            .map(|d| if self.workspace.read(cx).connections.is_empty() {
                d.child(
                    v_flex()
                        .debug_selector(|| "explorer-empty".into())
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(theme::space_8())
                        .child(div().text_size(theme::ui_sm()).text_color(theme.muted_foreground).child("No connections yet"))
                        .child(
                            div().debug_selector(|| "empty-add-connection".into()).child(
                                Button::new("empty-add-connection")
                                    .primary()
                                    .small()
                                    .label("Add connection")
                                    .on_click(cx.listener(|this, _, window, cx| this.add_connection(window, cx))),
                            ),
                        ),
                )
            } else if self.nodes.is_empty() && !self.filter.is_empty() {
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
    use std::sync::Arc;

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
        let loading = flatten(ws, &open, &[(schema.clone(), super::Load::Loading)].into(), false);
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
        let prefix = format!("{}/", ws.connections[ci].id.0);
        let under = |nodes: &[super::FlatNode]| nodes.iter().filter(|n| n.key.starts_with(&prefix)).cloned().collect::<Vec<_>>();
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
        assert_eq!((failed.len(), failed[0].kind, failed[0].label.as_ref()), (1, NodeKind::ConnError, "Could not connect: boom"));
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
            (ConnectionStatus::Error("timeout".into()), false, "Could not connect: timeout"),
            (ConnectionStatus::Error("".into()), false, "Could not connect: "),
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
        let obj = conn.databases.as_ref().unwrap()[0].schemas.as_ref().unwrap()[0].objects.iter().flatten().find(|o| o.kind == ObjectKind::Table).unwrap();
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
        let obj = conn.databases.as_mut().unwrap()[0].schemas.as_mut().unwrap()[0].objects.iter_mut().flatten().find(|o| o.kind == ObjectKind::Table).unwrap();
        obj.columns.as_mut().unwrap().truncate(3);
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
        let obj = conn.databases.as_mut().unwrap()[0].schemas.as_mut().unwrap()[0].objects.iter_mut().flatten().find(|o| o.name == name).unwrap();
        (obj.columns.as_mut().unwrap().clear(), obj.keys.clear());
        let nodes = flatten(&ws, &[table.key.clone()].into(), &Default::default(), true);
        let empty = nodes.iter().find(|n| n.key == table.key).unwrap();
        assert!(!empty.expandable, "nothing to show, no chevron");
    }

    #[test]
    fn column_rows_show_type_and_pk() {
        let ws = &mock::workspaces()[0];
        let conn = ws.connections.iter().find(|c| c.status == ConnectionStatus::Connected).unwrap();
        let obj = conn.databases.as_ref().unwrap()[0].schemas.as_ref().unwrap()[0].objects.iter().flatten().find(|o| o.kind == ObjectKind::Table).unwrap();
        let table = flatten(ws, &Default::default(), &Default::default(), true).into_iter().find(|n| n.label.as_ref() == obj.name).unwrap();
        let open = ["", "/Columns", "/Keys", "/Indexes"].map(|part| format!("{}{part}", table.key)).into();
        let nodes = flatten(ws, &open, &Default::default(), true);
        let items = |part| nodes.iter().filter(|n| n.kind == NodeKind::Item(part) && n.key.starts_with(&table.key)).collect::<Vec<_>>();

        let columns = items(Part::Columns);
        let got: Vec<_> = columns.iter().map(|n| (n.label.to_string(), n.suffix.as_ref().map(|s| s.to_string()), n.pk)).collect();
        let want: Vec<_> = obj.columns.iter().flatten().map(|c| (c.name.clone(), Some(c.data_type.clone()), c.is_pk)).collect();
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

    /// A Postgres connection `id` named `name` on database `database`, with nothing loaded.
    fn pg(id: u32, name: &str, database: &str) -> crate::model::Connection {
        use crate::model::{Connection, ConnectionId, ConnectionParams, SslMode};
        let params = ConnectionParams::Postgres { host: "h".into(), port: 5432, user: "u".into(), password: None, database: database.into(), ssl: SslMode::Prefer };
        Connection { id: ConnectionId(id), name: name.into(), color: 0, is_production: false, status: ConnectionStatus::Disconnected, params, databases: None }
    }

    /// A table `name` whose columns are not loaded yet.
    fn table(name: &str) -> crate::model::DbObject {
        crate::model::DbObject { kind: ObjectKind::Table, name: name.into(), columns: None, keys: vec![], indexes: vec![], row_estimate: None }
    }

    /// A column `name int`.
    fn column(name: &str) -> crate::model::Column {
        crate::model::Column { name: name.into(), data_type: "int".into(), nullable: true, default: None, is_pk: false, enum_values: vec![] }
    }

    /// A connected `conn` holding one database `db` with one schema `schema` of `objects`.
    fn loaded(mut conn: crate::model::Connection, db: &str, schema: &str, objects: Vec<crate::model::DbObject>) -> crate::model::Connection {
        use crate::model::{Database, Schema};
        let schemas = Some(vec![Schema { name: schema.into(), objects: Some(objects) }]);
        (conn.status, conn.databases) = (ConnectionStatus::Connected, Some(vec![Database { name: db.into(), schemas }]));
        conn
    }

    /// An explorer on a workspace of `connections`, reading through a fake driver the test answers.
    fn open_explorer(
        cx: &mut gpui_kit::TestAppContext,
        connections: Vec<crate::model::Connection>,
    ) -> (gpui_kit::VisualTestContext, gpui_kit::Entity<super::Explorer>, gpui_kit::Entity<crate::model::Workspace>, Arc<crate::db::fake::Fake>) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let fake = Arc::new(crate::db::fake::Fake::default());
        let driver: Arc<dyn crate::db::Driver> = fake.clone();
        let workspace = cx.update(|cx| cx.new(|_| crate::model::Workspace { id: 1, name: "w".into(), connections }));
        let ws = workspace.clone();
        let (window, explorer) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Explorer::new(ws, driver, window, cx))).expect("open test window")
        });
        let cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        (cx, explorer, workspace, fake)
    }

    #[gpui_kit::test]
    fn keys_are_name_paths_with_slash_and_backslash_escaped(cx: &mut gpui_kit::TestAppContext) {
        let mut users = table("users");
        users.columns = Some(vec![column("id"), column("a/b")]);
        let objects = vec![users, table("a/b"), table("a\\b"), table("\\/"), table("x\\"), table("")];
        let conn = loaded(pg(7, "pg", "app"), "my/db", "s\\1", objects);
        let (mut cx, explorer, _, _) = open_explorer(cx, vec![conn]);
        explorer.update_in(&mut cx, |e, _, cx| {
            e.expand_all_under(0, cx);
            let columns = e.nodes.iter().position(|n| n.label.as_ref() == "Columns").expect("Columns row");
            e.set_expanded(columns, true, cx);
        });
        // (row, key): segments are connection id, database, schema, kind, object, then the structure tail
        let cases = [
            ("pg", r"7"),
            ("my/db", r"7/my\/db"),
            ("s\\1", r"7/my\/db/s\\1"),
            ("Tables", r"7/my\/db/s\\1/table"),
            ("users", r"7/my\/db/s\\1/table/users"),
            ("Columns", r"7/my\/db/s\\1/table/users/Columns"),
            ("id", r"7/my\/db/s\\1/table/users/Columns/0"),
            ("a/b", r"7/my\/db/s\\1/table/users/Columns/1"),
            ("a/b", r"7/my\/db/s\\1/table/a\/b"),
            ("a\\b", r"7/my\/db/s\\1/table/a\\b"),
            ("\\/", r"7/my\/db/s\\1/table/\\\/"),
            ("x\\", r"7/my\/db/s\\1/table/x\\"),
            ("", r"7/my\/db/s\\1/table/"),
        ];
        let rows: Vec<(String, String)> = explorer.read_with(&cx, |e, _| e.nodes.iter().map(|n| (n.label.to_string(), n.key.clone())).collect());
        let want: Vec<(String, String)> = cases.iter().map(|(label, key)| (label.to_string(), key.to_string())).collect();
        assert_eq!(rows, want);
        // Each key reveals its own row from a collapsed tree: the escapes keep names whole.
        for (label, key) in cases {
            explorer.update_in(&mut cx, |e, window, cx| {
                e.expanded.clear();
                e.refresh(cx);
                e.reveal(key, window, cx);
            });
            let selected = explorer.read_with(&cx, |e, _| e.nodes.get(e.selected).map(|n| (n.label.to_string(), n.key.clone())));
            assert_eq!(selected, Some((label.to_string(), key.to_string())), "reveal {key}");
        }
    }

    #[gpui_kit::test]
    fn a_tab_on_t_keeps_opening_and_highlighting_t_after_a_table_is_inserted_before_it(cx: &mut gpui_kit::TestAppContext) {
        use std::{cell::RefCell, rc::Rc};
        let conn = loaded(pg(1, "pg", "app"), "app", "public", vec![table("t"), table("u")]);
        let (mut cx, explorer, workspace, _) = open_explorer(cx, vec![conn]);
        let opened = Rc::new(RefCell::new(Vec::new()));
        let sink = opened.clone();
        let _sub = cx.update(|_, cx| cx.subscribe(&explorer, move |_, ev: &super::OpenObject, _| sink.borrow_mut().push(ev.key.to_string())));
        let open_t = |cx: &mut gpui_kit::VisualTestContext| {
            explorer.update_in(cx, |e, _, cx| {
                e.expand_all_under(0, cx);
                let ix = e.nodes.iter().position(|n| n.label.as_ref() == "t").expect("t row");
                e.open(ix, false, cx);
            });
        };
        open_t(&mut cx);
        workspace.update(&mut cx, |ws, cx| {
            let schema = &mut ws.connections[0].databases.as_mut().unwrap()[0].schemas.as_mut().unwrap()[0];
            schema.objects.as_mut().unwrap().insert(0, table("a"));
            cx.notify();
        });
        cx.run_until_parked();
        open_t(&mut cx);
        let keys = opened.borrow().clone();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], keys[1], "opening t again gives the same tab key, so the same tab");
        explorer.update_in(&mut cx, |e, window, cx| e.reveal(&keys[0], window, cx));
        assert_eq!(explorer.read_with(&cx, |e, _| e.selected_label()), Some("t".into()), "the tab highlights t, not a");
    }

    /// A SQLite connection `id` named `name` on the file `path`, with nothing loaded.
    fn sqlite(id: u32, name: &str, path: &str) -> crate::model::Connection {
        crate::model::Connection { params: crate::model::ConnectionParams::Sqlite { path: path.into() }, ..pg(id, name, "") }
    }

    fn status_of(workspace: &gpui_kit::Entity<crate::model::Workspace>, cx: &gpui_kit::VisualTestContext) -> ConnectionStatus {
        workspace.read_with(cx, |ws, _| ws.connections[0].status.clone())
    }

    fn labels(explorer: &gpui_kit::Entity<super::Explorer>, cx: &gpui_kit::VisualTestContext) -> Vec<String> {
        explorer.read_with(cx, |e, _| e.row_labels().iter().map(|l| l.to_string()).collect())
    }

    /// Expands the first row, the connection.
    fn expand_connection(explorer: &gpui_kit::Entity<super::Explorer>, cx: &mut gpui_kit::VisualTestContext) {
        explorer.update_in(cx, |e, _, cx| e.set_expanded(0, true, cx));
    }

    #[gpui_kit::test]
    fn expanding_a_disconnected_connection_connects_then_lists_its_databases(cx: &mut gpui_kit::TestAppContext) {
        use crate::db::fake::session;
        // (connection, database the connect opens, databases listed)
        let cases = [
            (pg(1, "pg", "app"), "app", vec!["analytics", "app"]),
            (sqlite(1, "lite", "/x/local.db"), "local.db", vec!["local.db"]),
        ];
        for (conn, db, databases) in cases {
            let name = conn.name.clone();
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![conn]);
            expand_connection(&explorer, &mut cx);
            assert_eq!(status_of(&workspace, &cx), ConnectionStatus::Connecting, "{name}");
            assert_eq!(fake.pending(), [format!("connect {db}")], "{name}");
            fake.resolve(&format!("connect {db}"), Ok(session(db)));
            cx.run_until_parked();
            assert_eq!(status_of(&workspace, &cx), ConnectionStatus::Connected, "{name}");
            assert_eq!(fake.pending(), [format!("databases {db}")], "{name}");
            fake.resolve(&format!("databases {db}"), Ok(databases.iter().map(|d| d.to_string()).collect::<Vec<_>>()));
            cx.run_until_parked();
            let mut want = vec![name.clone()];
            want.extend(databases.iter().map(|d| d.to_string()));
            assert_eq!(labels(&explorer, &cx), want, "{name}: the child rows are the databases");
        }
    }

    #[gpui_kit::test]
    fn a_failed_connection_connects_again_on_expand_or_retry(cx: &mut gpui_kit::TestAppContext) {
        use crate::db::fake::session;
        use ConnectionStatus::*;
        // (how, the connect's answer, status after)
        let cases = [
            ("expand", Ok(()), Connected),
            ("expand", Err("password authentication failed for user \"u\""), Error("password authentication failed for user \"u\"".into())),
            ("retry", Ok(()), Connected),
            ("retry", Err("timeout expired after 10 s"), Error("timeout expired after 10 s".into())),
            ("retry", Err("old"), Error("old".into())),
        ];
        for (how, answer, want) in cases {
            let label = format!("{how} {answer:?}");
            let conn = crate::model::Connection { status: Error("old".into()), ..pg(1, "pg", "app") };
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![conn]);
            match how {
                "expand" => expand_connection(&explorer, &mut cx),
                _ => {
                    explorer.update_in(&mut cx, |e, window, cx| e.reveal(&format!("1/{}", super::ERROR), window, cx));
                    assert_eq!(explorer.read_with(&cx, |e, _| e.nodes[e.selected].kind), NodeKind::ConnError, "{label}");
                    cx.simulate_keystrokes("enter");
                }
            }
            assert_eq!(status_of(&workspace, &cx), Connecting, "{label}");
            fake.resolve("connect app", answer.map(|()| session("app")).map_err(String::from));
            cx.run_until_parked();
            assert_eq!(status_of(&workspace, &cx), want, "{label}");
        }
    }

    #[gpui_kit::test]
    fn a_connect_that_failed_on_expand_connects_again_on_enter_or_right_on_its_open_row(cx: &mut gpui_kit::TestAppContext) {
        use crate::db::fake::session;
        use ConnectionStatus::*;
        // (key on the failed, still open connection row, the new connect's answer, status after)
        let cases = [
            ("enter", Ok(()), Connected),
            ("enter", Err("new"), Error("new".into())),
            ("enter", Err("boom"), Error("boom".into())),
            ("right", Ok(()), Connected),
            ("right", Err("new"), Error("new".into())),
        ];
        for (key, answer, want) in cases {
            let label = format!("{key} {answer:?}");
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            expand_connection(&explorer, &mut cx);
            fake.resolve::<crate::db::Session>("connect app", Err("boom".into()));
            cx.run_until_parked();
            explorer.update_in(&mut cx, |e, window, cx| e.reveal("1", window, cx));
            assert!(explorer.read_with(&cx, |e, _| e.nodes[0].expanded), "{label}: the failed row is open");
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
            assert_eq!(status_of(&workspace, &cx), Connecting, "{label}");
            assert_eq!(fake.pending(), ["connect app"], "{label}");
            fake.resolve("connect app", answer.map(|()| session("app")).map_err(String::from));
            cx.run_until_parked();
            assert_eq!(status_of(&workspace, &cx), want, "{label}");
        }
    }

    #[gpui_kit::test]
    fn expanding_again_while_connecting_starts_no_second_connect(cx: &mut gpui_kit::TestAppContext) {
        // (keys pressed on the connecting connection's row)
        let cases = ["right", "left right", "enter", "left enter", "space space"];
        for keys in cases {
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            expand_connection(&explorer, &mut cx);
            explorer.update_in(&mut cx, |e, window, cx| e.reveal("1", window, cx));
            cx.simulate_keystrokes(keys);
            cx.run_until_parked();
            assert_eq!(fake.pending(), ["connect app"], "{keys}");
            assert_eq!(status_of(&workspace, &cx), ConnectionStatus::Connecting, "{keys}");
        }
    }

    #[gpui_kit::test]
    fn a_connection_collapsed_and_expanded_again_while_connecting_opens_when_the_connect_lands(cx: &mut gpui_kit::TestAppContext) {
        use crate::db::fake::session;
        // (expanded again before the connect lands, the connect's answer, open after, rows after)
        let cases: [(bool, Result<(), &str>, bool, &[&str]); 4] = [
            (true, Ok(()), true, &["pg", "app"]),
            (true, Err("boom"), true, &["pg", "Could not connect: boom"]),
            (false, Ok(()), false, &["pg"]),
            (false, Err("boom"), false, &["pg", "Could not connect: boom"]),
        ];
        for (again, answer, open, rows) in cases {
            let label = format!("again {again} {answer:?}");
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            expand_connection(&explorer, &mut cx);
            collapse(&explorer, &mut cx, "pg");
            if again {
                expand(&explorer, &mut cx, "pg");
            }
            assert_eq!(fake.pending(), ["connect app"], "{label}: no second connect");
            fake.resolve("connect app", answer.map(|()| session("app")).map_err(String::from));
            cx.run_until_parked();
            if answer.is_ok() {
                assert_eq!(status_of(&workspace, &cx), ConnectionStatus::Connected, "{label}");
                fake.resolve("databases app", Ok(strings(&["app"])));
                cx.run_until_parked();
            }
            assert_eq!(explorer.read_with(&cx, |e, _| e.nodes[0].expanded), open, "{label}: open");
            assert_eq!(labels(&explorer, &cx), rows, "{label}: rows");
        }
    }

    #[gpui_kit::test]
    fn a_failed_connect_shows_could_not_connect_in_the_error_row_and_the_dot_tooltip(cx: &mut gpui_kit::TestAppContext) {
        // (reason the connect fails with)
        let cases = ["password authentication failed for user \"u\"", "timeout expired after 10 s", "No file at /x/local.db"];
        for reason in cases {
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            expand_connection(&explorer, &mut cx);
            fake.resolve::<crate::db::Session>("connect app", Err(reason.into()));
            cx.run_until_parked();
            let want = format!("Could not connect: {reason}");
            assert_eq!(labels(&explorer, &cx), ["pg".to_string(), want.clone()], "tree error row");
            assert_eq!(status_style(&status_of(&workspace, &cx)), (false, want), "ring dot and its tooltip");
        }
    }

    /// Connection `conn`'s params with `password` and `database`.
    fn pg_params(password: Option<&str>, database: &str) -> crate::model::ConnectionParams {
        use crate::model::{ConnectionParams, Secret, SslMode};
        ConnectionParams::Postgres { host: "h".into(), port: 5432, user: "u".into(), password: password.map(Secret::new), database: database.into(), ssl: SslMode::Prefer }
    }

    #[gpui_kit::test]
    fn a_connect_whose_connection_is_edited_or_deleted_while_connecting_is_dropped(cx: &mut gpui_kit::TestAppContext) {
        use crate::db::fake::session;
        use ConnectionStatus::*;
        // (change while connecting, the connect's answer, the connection's status once it lands; None = deleted)
        let cases: [(&str, Result<(), &str>, Option<ConnectionStatus>); 8] = [
            ("database", Ok(()), Some(Disconnected)),
            ("database", Err("boom"), Some(Disconnected)),
            ("password", Ok(()), Some(Disconnected)),
            ("password", Err("boom"), Some(Disconnected)),
            ("delete", Ok(()), None),
            ("delete", Err("boom"), None),
            // Name only: the params are the same, so the connect still lands.
            ("name", Ok(()), Some(Connected)),
            ("name", Err("boom"), Some(Error("boom".into()))),
        ];
        for (change, answer, want) in cases {
            let label = format!("{change} {answer:?}");
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            expand_connection(&explorer, &mut cx);
            let id = crate::model::ConnectionId(1);
            workspace.update(&mut cx, |ws, cx| {
                match change {
                    "database" => ws.edit(id, "pg".into(), 0, false, pg_params(None, "other")),
                    "password" => ws.edit(id, "pg".into(), 0, false, pg_params(Some("secret"), "app")),
                    "name" => ws.edit(id, "renamed".into(), 0, false, pg_params(None, "app")),
                    _ => drop(ws.delete(id)),
                }
                cx.notify();
            });
            cx.run_until_parked();
            fake.resolve("connect app", answer.map(|()| session("app")).map_err(String::from));
            cx.run_until_parked();
            let status = workspace.read_with(&cx, |ws, _| ws.connections.first().map(|c| c.status.clone()));
            assert_eq!(status, want, "{label}");
            if change != "name" {
                assert_eq!(fake.pending(), Vec::<String>::new(), "{label}: nothing loads from the dropped result");
                let rows = labels(&explorer, &cx);
                assert!(rows.iter().all(|r| !r.starts_with("Could not connect")), "{label}: no error row from it: {rows:?}");
            }
        }
    }

    /// Expands the connection and answers its connect and databases; returns a handle that dies with the session.
    fn connect_answered(
        explorer: &gpui_kit::Entity<super::Explorer>,
        fake: &crate::db::fake::Fake,
        cx: &mut gpui_kit::VisualTestContext,
        databases: &[&str],
    ) -> std::sync::Weak<crate::db::fake::FakeSession> {
        expand_connection(explorer, cx);
        let session = crate::db::fake::session("app");
        let crate::db::Session::Fake(alive) = &session else { unreachable!() };
        let alive = Arc::downgrade(alive);
        fake.resolve("connect app", Ok(session));
        cx.run_until_parked();
        fake.resolve("databases app", Ok(databases.iter().map(|d| d.to_string()).collect::<Vec<_>>()));
        cx.run_until_parked();
        alive
    }

    #[gpui_kit::test]
    fn editing_the_params_or_deleting_a_connected_connection_closes_its_sessions(cx: &mut gpui_kit::TestAppContext) {
        use ConnectionStatus::*;
        // (change, session closed, status after (None = deleted), rows after)
        let cases: [(&str, bool, Option<ConnectionStatus>, &[&str]); 5] = [
            ("password", true, Some(Disconnected), &["pg"]),
            ("database", true, Some(Disconnected), &["pg"]),
            ("delete", true, None, &[]),
            ("name", false, Some(Connected), &["renamed", "app"]),
            ("nothing", false, Some(Connected), &["pg", "app"]),
        ];
        for (change, closed, want, rows) in cases {
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            let session = connect_answered(&explorer, &fake, &mut cx, &["app"]);
            assert_eq!(labels(&explorer, &cx), ["pg", "app"], "{change}: connected and open");
            let id = crate::model::ConnectionId(1);
            workspace.update(&mut cx, |ws, cx| {
                match change {
                    "password" => ws.edit(id, "pg".into(), 0, false, pg_params(Some("secret"), "app")),
                    "database" => ws.edit(id, "pg".into(), 0, false, pg_params(None, "other")),
                    "name" => ws.edit(id, "renamed".into(), 0, false, pg_params(None, "app")),
                    "nothing" => ws.edit(id, "pg".into(), 0, false, pg_params(None, "app")),
                    _ => drop(ws.delete(id)),
                }
                cx.notify();
            });
            cx.run_until_parked();
            assert_eq!(session.upgrade().is_none(), closed, "{change}: session closed");
            assert_eq!(workspace.read_with(&cx, |ws, _| ws.connections.first().map(|c| c.status.clone())), want, "{change}: status");
            assert_eq!(labels(&explorer, &cx), rows, "{change}: rows");
            let collapsed = explorer.read_with(&cx, |e, _| e.nodes.first().is_none_or(|n| !n.expanded));
            assert_eq!(collapsed, closed, "{change}: tree collapsed");
        }
    }

    /// Expands the row labelled `label`.
    fn expand(explorer: &gpui_kit::Entity<super::Explorer>, cx: &mut gpui_kit::VisualTestContext, label: &str) {
        explorer.update_in(cx, |e, _, cx| {
            let ix = e.nodes.iter().position(|n| n.label.as_ref() == label).unwrap_or_else(|| panic!("{label} row"));
            e.set_expanded(ix, true, cx);
        });
        cx.run_until_parked();
    }

    /// The labels of the rows under the row labelled `label`.
    fn under(explorer: &gpui_kit::Entity<super::Explorer>, cx: &gpui_kit::VisualTestContext, label: &str) -> Vec<String> {
        explorer.read_with(cx, |e, _| {
            let ix = e.nodes.iter().position(|n| n.label.as_ref() == label).unwrap_or_else(|| panic!("{label} row"));
            e.nodes[ix + 1..].iter().take_while(|n| n.depth > e.nodes[ix].depth).map(|n| n.label.to_string()).collect()
        })
    }

    fn view(name: &str) -> crate::model::DbObject {
        crate::model::DbObject { kind: ObjectKind::View, ..table(name) }
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[gpui_kit::test]
    fn each_level_loads_only_when_expanded_and_shows_a_loading_row_meanwhile(cx: &mut gpui_kit::TestAppContext) {
        let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
        assert_eq!(fake.pending(), Vec::<String>::new(), "no database loads before it is expanded");

        expand(&explorer, &mut cx, "app");
        assert_eq!(fake.pending(), ["schemas app"]);
        assert_eq!(under(&explorer, &cx, "app"), ["Loading…"]);
        fake.resolve("schemas app", Ok(strings(&["public", "empty"])));
        cx.run_until_parked();
        assert_eq!(under(&explorer, &cx, "app"), ["public", "empty"]);
        assert_eq!(under(&explorer, &cx, "analytics"), Vec::<String>::new(), "a database not expanded stays unloaded");
        assert_eq!(fake.pending(), Vec::<String>::new(), "no schema loads before it is expanded");

        expand(&explorer, &mut cx, "public");
        assert_eq!(fake.pending(), ["objects app/public"]);
        assert_eq!(under(&explorer, &cx, "public"), ["Loading…"]);
        fake.resolve("objects app/public", Ok(vec![table("t"), view("v")]));
        cx.run_until_parked();
        assert_eq!(under(&explorer, &cx, "public"), ["Tables", "Views"]);
        expand(&explorer, &mut cx, "Tables");
        assert_eq!(under(&explorer, &cx, "Tables"), ["t"]);
        assert_eq!(fake.pending(), Vec::<String>::new(), "no column loads before the table is expanded");

        expand(&explorer, &mut cx, "t");
        assert_eq!(fake.pending(), ["columns app/public/t"]);
        assert_eq!(under(&explorer, &cx, "t"), ["Loading…"]);
        fake.resolve("columns app/public/t", Ok(vec![column("id"), column("name")]));
        cx.run_until_parked();
        expand(&explorer, &mut cx, "Columns");
        assert_eq!(under(&explorer, &cx, "t"), ["Columns", "id", "name"]);

        // (row expanded, the load it starts; it brings back nothing)
        let empty = [("empty", "objects app/empty"), ("analytics", "schemas analytics")];
        for (row, call) in empty {
            expand(&explorer, &mut cx, row);
            if row == "analytics" {
                // Another database, so it opens a session of its own first.
                assert_eq!(fake.pending(), ["connect analytics"], "{row}");
                fake.resolve("connect analytics", Ok(crate::db::fake::session("analytics")));
                cx.run_until_parked();
            }
            assert_eq!(fake.pending(), [call], "{row}");
            assert_eq!(under(&explorer, &cx, row), ["Loading…"], "{row}");
            if row == "analytics" {
                fake.resolve::<Vec<String>>(call, Ok(vec![]));
            } else {
                fake.resolve::<Vec<crate::model::DbObject>>(call, Ok(vec![]));
            }
            cx.run_until_parked();
            assert_eq!(under(&explorer, &cx, row), Vec::<String>::new(), "{row}: no children");
            let chevron = explorer.read_with(&cx, |e, _| e.nodes.iter().find(|n| n.label.as_ref() == row).unwrap().expandable);
            assert!(!chevron, "{row}: nothing to expand once loaded empty");
        }
    }

    /// Answers the pending call `label` with a sample of its level, or with `Err(reason)`.
    fn answer(fake: &crate::db::fake::Fake, label: &str, result: Result<(), &str>) {
        let err = |reason: &str| reason.to_string();
        match label.split(' ').next() {
            Some("schemas") => fake.resolve(label, result.map(|()| strings(&["public", "sibling"])).map_err(err)),
            Some("objects") => fake.resolve(label, result.map(|()| vec![table("t"), table("u")]).map_err(err)),
            Some("columns") => fake.resolve(label, result.map(|()| vec![column("id")]).map_err(err)),
            Some("databases") => fake.resolve(label, result.map(|()| strings(&["app"])).map_err(err)),
            _ => fake.resolve(label, result.map(|()| crate::db::fake::session("app")).map_err(err)),
        }
    }

    #[gpui_kit::test]
    fn a_failed_load_shows_could_not_load_with_retry_under_that_node_only(cx: &mut gpui_kit::TestAppContext) {
        // (row whose load fails, that load, how Retry is pressed, its rows once Retry succeeds)
        let cases = [
            ("public", "objects app/public", "enter", vec!["Tables"]),
            ("public", "objects app/public", "click", vec!["Tables"]),
            ("app", "schemas app", "enter", vec!["public", "sibling"]),
            ("t", "columns app/public/t", "click", vec!["Columns"]),
        ];
        for (row, call, how, after) in cases {
            let label = format!("{row} {how}");
            let reason = format!("permission denied for {row}");
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            connect_answered(&explorer, &fake, &mut cx, &["app"]);
            for step in ["app", "public", "Tables", "t"] {
                expand(&explorer, &mut cx, step);
                for pending in fake.pending() {
                    answer(&fake, &pending, if pending == call { Err(&reason) } else { Ok(()) });
                }
                cx.run_until_parked();
                if step == row {
                    break;
                }
            }
            let error = format!("Could not load: {reason}");
            assert_eq!(under(&explorer, &cx, row), [error.clone()], "{label}: error row under {row}");
            let rows = labels(&explorer, &cx);
            assert_eq!(rows.iter().filter(|r| r.starts_with("Could not")).count(), 1, "{label}: under that node only: {rows:?}");
            assert_eq!(status_style(&status_of(&workspace, &cx)).0, true, "{label}: the connection dot stays solid");

            let ix = rows.iter().position(|r| *r == error).unwrap();
            match how {
                "enter" => {
                    let key = explorer.read_with(&cx, |e, _| e.nodes[ix].key.clone());
                    explorer.update_in(&mut cx, |e, window, cx| e.reveal(&key, window, cx));
                    cx.simulate_keystrokes("enter");
                }
                _ => {
                    let bounds = cx.debug_bounds(format!("retry-{ix}").leak()).unwrap_or_else(|| panic!("{label}: Retry drawn"));
                    cx.simulate_click(bounds.center(), Default::default());
                }
            }
            cx.run_until_parked();
            assert_eq!(fake.pending(), [call], "{label}: Retry runs that node's load again");
            answer(&fake, call, Ok(()));
            cx.run_until_parked();
            assert_eq!(under(&explorer, &cx, row), after, "{label}: the error row gives way to the children");
            assert!(labels(&explorer, &cx).iter().all(|r| !r.starts_with("Could not")), "{label}: no error row left");
        }
    }

    /// Collapses the row labelled `label`.
    fn collapse(explorer: &gpui_kit::Entity<super::Explorer>, cx: &mut gpui_kit::VisualTestContext, label: &str) {
        explorer.update_in(cx, |e, _, cx| {
            let ix = e.nodes.iter().position(|n| n.label.as_ref() == label).unwrap_or_else(|| panic!("{label} row"));
            e.set_expanded(ix, false, cx);
        });
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn collapsing_during_a_load_still_stores_its_result_and_the_node_stays_collapsed(cx: &mut gpui_kit::TestAppContext) {
        // (row collapsed while loading, its load, its rows once expanded again with no new load)
        let cases = [("public", "objects app/public", vec!["Tables"]), ("app", "schemas app", vec!["public", "sibling"]), ("t", "columns app/public/t", vec!["Columns"])];
        for (row, call, after) in cases {
            let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            connect_answered(&explorer, &fake, &mut cx, &["app"]);
            for step in ["app", "public", "Tables", "t"] {
                expand(&explorer, &mut cx, step);
                if step == row {
                    break;
                }
                for pending in fake.pending() {
                    answer(&fake, &pending, Ok(()));
                }
                cx.run_until_parked();
            }
            assert_eq!(fake.pending(), [call], "{row}");
            collapse(&explorer, &mut cx, row);
            answer(&fake, call, Ok(()));
            cx.run_until_parked();
            let expanded = explorer.read_with(&cx, |e, _| e.nodes.iter().find(|n| n.label.as_ref() == row).unwrap().expanded);
            assert!(!expanded, "{row}: stays collapsed");
            assert_eq!(under(&explorer, &cx, row), Vec::<String>::new(), "{row}: no rows under it");
            expand(&explorer, &mut cx, row);
            assert_eq!(fake.pending(), Vec::<String>::new(), "{row}: the stored result shows without a new load");
            assert_eq!(under(&explorer, &cx, row), after, "{row}");
        }
    }

    #[gpui_kit::test]
    fn a_load_from_before_an_edit_and_reconnect_is_dropped_when_it_lands(cx: &mut gpui_kit::TestAppContext) {
        use crate::db::fake::session;
        // (the old load's answer, whether the new load of the same schema has started when it lands)
        let cases = [(Ok(()), true), (Ok(()), false), (Err("server closed the connection"), true), (Err("server closed the connection"), false)];
        for (old, new_started) in cases {
            let label = format!("old {old:?}, new load started {new_started}");
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            connect_answered(&explorer, &fake, &mut cx, &["app"]);
            expand(&explorer, &mut cx, "app");
            fake.resolve("schemas app", Ok(strings(&["public"])));
            cx.run_until_parked();
            expand(&explorer, &mut cx, "public");
            assert_eq!(fake.pending(), ["objects app/public"], "{label}: the old server's load in flight");

            workspace.update(&mut cx, |ws, cx| {
                ws.edit(crate::model::ConnectionId(1), "pg".into(), 0, false, pg_params(Some("new"), "app"));
                cx.notify();
            });
            cx.run_until_parked();
            expand_connection(&explorer, &mut cx);
            fake.resolve("connect app", Ok(session("app")));
            cx.run_until_parked();
            fake.resolve("databases app", Ok(strings(&["app"])));
            cx.run_until_parked();
            expand(&explorer, &mut cx, "app");
            fake.resolve("schemas app", Ok(strings(&["public"])));
            cx.run_until_parked();
            if new_started {
                expand(&explorer, &mut cx, "public");
            }

            // FIFO: the oldest "objects app/public" is the old server's.
            match old {
                Ok(()) => fake.resolve("objects app/public", Ok(vec![table("old_t")])),
                Err(reason) => fake.resolve::<Vec<crate::model::DbObject>>("objects app/public", Err(reason.into())),
            }
            cx.run_until_parked();
            let want: Vec<String> = if new_started { strings(&["Loading…"]) } else { vec![] };
            assert_eq!(under(&explorer, &cx, "public"), want, "{label}: the old result is dropped");
            if !new_started {
                expand(&explorer, &mut cx, "public");
            }
            fake.resolve("objects app/public", Ok(vec![table("new_t")]));
            cx.run_until_parked();
            expand(&explorer, &mut cx, "Tables");
            assert_eq!(labels(&explorer, &cx), ["pg", "app", "public", "Tables", "new_t"], "{label}: only the new server's data");
        }
    }

    #[gpui_kit::test]
    fn a_session_an_old_load_opens_after_an_edit_is_never_reused(cx: &mut gpui_kit::TestAppContext) {
        use crate::db::fake::session;
        // (the old session lands before or after the reconnect)
        for after_reconnect in [false, true] {
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
            expand(&explorer, &mut cx, "analytics");
            assert_eq!(fake.pending(), ["connect analytics"], "another database opens its own session");
            workspace.update(&mut cx, |ws, cx| {
                ws.edit(crate::model::ConnectionId(1), "pg".into(), 0, false, pg_params(Some("new"), "app"));
                cx.notify();
            });
            cx.run_until_parked();
            if !after_reconnect {
                fake.resolve("connect analytics", Ok(session("analytics")));
                cx.run_until_parked();
            }
            connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
            if after_reconnect {
                fake.resolve("connect analytics", Ok(session("analytics")));
                cx.run_until_parked();
            }
            expand(&explorer, &mut cx, "analytics");
            assert_eq!(fake.pending(), ["connect analytics"], "after reconnect {after_reconnect}: the old server's session is not reused");
        }
    }

    #[gpui_kit::test]
    fn expanding_a_database_again_while_it_loads_opens_no_second_session(cx: &mut gpui_kit::TestAppContext) {
        // (database expanded, collapsed and expanded again while loading, the calls pending)
        let cases = [("analytics", vec!["connect analytics"]), ("app", vec!["schemas app"])];
        for (db, want) in cases {
            let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
            expand(&explorer, &mut cx, db);
            collapse(&explorer, &mut cx, db);
            expand(&explorer, &mut cx, db);
            assert_eq!(fake.pending(), want, "{db}");
        }
    }

    #[gpui_kit::test]
    fn a_load_that_finds_its_session_closed_drops_it_and_retry_reconnects(cx: &mut gpui_kit::TestAppContext) {
        use crate::db::fake::{close, session};
        // (row whose load fails, that load, session closed by the server, the calls Retry makes in turn)
        let cases = [
            ("app", "schemas app", true, vec!["connect app", "schemas app"]),
            ("app", "schemas app", false, vec!["schemas app"]),
            ("pg", "databases app", true, vec!["connect app", "databases app"]),
            ("pg", "databases app", false, vec!["databases app"]),
        ];
        for (row, call, closed, retry) in cases {
            let label = format!("{call} closed {closed}");
            let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            expand_connection(&explorer, &mut cx);
            let first = session("app");
            fake.resolve("connect app", Ok(first.clone()));
            cx.run_until_parked();
            if row == "app" {
                fake.resolve("databases app", Ok(strings(&["app"])));
                cx.run_until_parked();
                expand(&explorer, &mut cx, "app");
            }
            if closed {
                close(&first);
            }
            answer(&fake, call, Err("server closed the connection unexpectedly"));
            cx.run_until_parked();
            let error = "Could not load: server closed the connection unexpectedly";
            assert_eq!(under(&explorer, &cx, row), [error], "{label}: error row");
            assert_eq!(status_of(&workspace, &cx), ConnectionStatus::Connected, "{label}");

            let ix = labels(&explorer, &cx).iter().position(|r| r == error).unwrap();
            explorer.update_in(&mut cx, |e, _, cx| e.retry(ix, cx));
            cx.run_until_parked();
            for call in retry {
                assert_eq!(fake.pending(), [call], "{label}: Retry");
                answer(&fake, call, Ok(()));
                cx.run_until_parked();
            }
            assert!(under(&explorer, &cx, row).iter().all(|r| !r.starts_with("Could not")), "{label}: loaded after Retry");
        }
    }

    #[gpui_kit::test]
    fn reveal_clears_the_field_text_and_the_filter(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Entity};
        cx.update(gpui_kit::init);
        let ws = mock::workspaces().remove(0);
        let key = flatten(&ws, &default_expanded(&ws), &Default::default(), true).into_iter().find(|n| n.label.as_ref() == "orders").expect("orders").key;
        let (window, explorer): (_, Entity<super::Explorer>) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Explorer::new(cx.new(|_| ws), Arc::new(crate::db::Live::default()), window, cx))).expect("open test window")
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
            let table = ws.connections[0].databases.as_mut().unwrap()[0].schemas.as_mut().unwrap()[0].objects.iter_mut().flatten().find(|o| o.name == "orders").expect("orders");
            table.name = name.to_string();
            let key = flatten(&ws, &default_expanded(&ws), &Default::default(), true).into_iter().find(|n| n.label.as_ref() == name).expect("table row").key;
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(width), px(600.))))),
                ..Default::default()
            };
            let (window, explorer): (_, Entity<super::Explorer>) = cx.update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| super::Explorer::new(cx.new(|_| ws), Arc::new(crate::db::Live::default()), window, cx))).expect("open test window")
            });
            let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
            explorer.update_in(&mut cx, |e, window, cx| e.reveal(&key, window, cx));
            cx.run_until_parked();
            let ix = explorer.read_with(&cx, |e, _| e.selected);
            let shown = crate::theme::tests::hover_shows_truncation_tip(&mut cx, format!("tree-label-{ix}"));
            assert_eq!(shown, tip, "{} chars at {width}px: tooltip shown", name.len());
        }
    }

    /// Connected `pg` on database app, open down to schema public's tables t and u.
    fn tables_open(cx: &mut gpui_kit::TestAppContext) -> (gpui_kit::VisualTestContext, gpui_kit::Entity<super::Explorer>, gpui_kit::Entity<crate::model::Workspace>, Arc<crate::db::fake::Fake>) {
        let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
        connect_answered(&explorer, &fake, &mut cx, &["app"]);
        for step in ["app", "public", "Tables"] {
            expand(&explorer, &mut cx, step);
            for pending in fake.pending() {
                answer(&fake, &pending, Ok(()));
            }
            cx.run_until_parked();
        }
        (cx, explorer, workspace, fake)
    }

    /// Right-clicks the row labelled `label` and picks Refresh from its menu.
    fn refresh(explorer: &gpui_kit::Entity<super::Explorer>, cx: &mut gpui_kit::VisualTestContext, label: &str) {
        let ix = labels(explorer, cx).iter().position(|l| l == label).unwrap_or_else(|| panic!("{label} row"));
        let at = cx.debug_bounds(format!("tree-label-{ix}").leak()).unwrap_or_else(|| panic!("{label} row drawn")).center();
        cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
        cx.run_until_parked();
        let item = cx.debug_bounds("menu-refresh").unwrap_or_else(|| panic!("Refresh in {label}'s menu"));
        cx.simulate_click(item.center(), Default::default());
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn refreshing_a_schema_after_a_table_was_created_elsewhere_shows_it_sorted(cx: &mut gpui_kit::TestAppContext) {
        let (mut cx, explorer, _, fake) = tables_open(cx);
        assert_eq!(under(&explorer, &cx, "Tables"), ["t", "u"]);
        refresh(&explorer, &mut cx, "public");
        assert_eq!(fake.pending(), ["objects app/public"]);
        fake.resolve("objects app/public", Ok(vec![table("t"), table("t2"), table("u")]));
        cx.run_until_parked();
        assert_eq!(under(&explorer, &cx, "Tables"), ["t", "t2", "u"]);
    }

    /// Whether the row labelled `label` is expanded.
    fn is_expanded(explorer: &gpui_kit::Entity<super::Explorer>, cx: &gpui_kit::VisualTestContext, label: &str) -> bool {
        explorer.read_with(cx, |e, _| e.nodes.iter().find(|n| n.label.as_ref() == label).unwrap_or_else(|| panic!("{label} row")).expanded)
    }

    #[gpui_kit::test]
    fn refreshing_a_schema_after_a_table_was_dropped_removes_it_and_forgets_its_expansion(cx: &mut gpui_kit::TestAppContext) {
        // (t expanded before it was dropped)
        for t_open in [true, false] {
            let (mut cx, explorer, _, fake) = tables_open(cx);
            if t_open {
                expand(&explorer, &mut cx, "t");
                answer(&fake, "columns app/public/t", Ok(()));
                cx.run_until_parked();
            }
            refresh(&explorer, &mut cx, "public");
            fake.resolve("objects app/public", Ok(vec![table("u")]));
            cx.run_until_parked();
            assert_eq!(under(&explorer, &cx, "Tables"), ["u"], "t open {t_open}: t gone");

            // t created again: it comes back collapsed, with no column load.
            refresh(&explorer, &mut cx, "public");
            fake.resolve("objects app/public", Ok(vec![table("t"), table("u")]));
            cx.run_until_parked();
            assert_eq!(under(&explorer, &cx, "Tables"), ["t", "u"], "t open {t_open}");
            assert!(!is_expanded(&explorer, &cx, "t"), "t open {t_open}: its expansion is forgotten");
            assert_eq!(fake.pending(), Vec::<String>::new(), "t open {t_open}");
        }
    }

    #[gpui_kit::test]
    fn refreshing_a_schema_with_table_t_expanded_keeps_t_expanded_with_its_columns_re_read(cx: &mut gpui_kit::TestAppContext) {
        // (row refreshed, the loads it runs before t's columns, answered with the sample)
        let cases = [("public", vec!["objects app/public"]), ("app", vec!["schemas app", "objects app/public"])];
        for (row, loads) in cases {
            let (mut cx, explorer, _, fake) = tables_open(cx);
            expand(&explorer, &mut cx, "t");
            fake.resolve("columns app/public/t", Ok(vec![column("id")]));
            cx.run_until_parked();
            expand(&explorer, &mut cx, "Columns");
            refresh(&explorer, &mut cx, row);
            for call in loads {
                assert_eq!(fake.pending(), [call], "{row}");
                answer(&fake, call, Ok(()));
                cx.run_until_parked();
            }
            assert_eq!(fake.pending(), ["columns app/public/t"], "{row}: t's columns are re-read");
            assert!(is_expanded(&explorer, &cx, "t"), "{row}: t stays expanded");
            fake.resolve("columns app/public/t", Ok(vec![column("id"), column("name")]));
            cx.run_until_parked();
            assert_eq!(under(&explorer, &cx, "t"), ["Columns", "id", "name"], "{row}");
        }
    }

    #[gpui_kit::test]
    fn refreshing_a_connection_re_lists_its_databases_and_re_reads_its_expanded_databases_and_schemas(cx: &mut gpui_kit::TestAppContext) {
        // (databases the refresh lists, rows after)
        let cases = [
            (vec!["analytics", "app", "new"], vec!["pg", "analytics", "app", "public", "Tables", "sibling", "new"]),
            (vec!["app"], vec!["pg", "app", "public", "Tables", "sibling"]),
        ];
        for (databases, rows) in cases {
            let (mut cx, explorer, _, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
            connect_answered(&explorer, &fake, &mut cx, &["analytics", "app"]);
            for step in ["app", "public"] {
                expand(&explorer, &mut cx, step);
                for pending in fake.pending() {
                    answer(&fake, &pending, Ok(()));
                }
                cx.run_until_parked();
            }
            refresh(&explorer, &mut cx, "pg");
            assert_eq!(fake.pending(), ["databases app"], "{databases:?}: no new connect, the databases are re-listed");
            fake.resolve("databases app", Ok(strings(&databases)));
            cx.run_until_parked();
            // analytics is not expanded, so it is not read.
            for call in ["schemas app", "objects app/public"] {
                assert_eq!(fake.pending(), [call], "{databases:?}");
                answer(&fake, call, Ok(()));
                cx.run_until_parked();
            }
            assert_eq!(fake.pending(), Vec::<String>::new(), "{databases:?}");
            assert_eq!(labels(&explorer, &cx), rows, "{databases:?}");
        }
    }
}
