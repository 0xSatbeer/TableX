//! Explorer tree: the visible nodes flattened into a `Vec<FlatNode>` and rendered with
//! `uniform_list`. The flat list is rebuilt only on expand or collapse.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{Connection, ConnectionParams, ConnectionStatus, DbKind, ObjectKind, Workspace};
use crate::tabs::Origin;
use crate::{assets, mock, theme};

#[derive(Clone, Copy, Debug, PartialEq)]
enum NodeKind {
    Connection(DbKind),
    Database,
    Schema,
    Group(ObjectKind),
    Object(ObjectKind),
    /// Placeholder under a connection that is connecting or a schema whose objects are loading.
    Loading,
    /// A failed connection's message, shown under it whether or not it is expanded.
    ConnError,
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
    let mut push = |key: String, kind, label: &str, depth, conn, count, expandable: bool| {
        let open = expandable && (all || expanded.contains(&key));
        out.push(FlatNode { key, kind, label: label.to_string().into(), depth, conn, count, expandable, expanded: open });
        open
    };
    for (ci, conn) in ws.connections.iter().enumerate() {
        let key = format!("{ci}");
        let open = push(key.clone(), NodeKind::Connection(conn.kind()), &conn.name, 0, ci, None, !conn.databases.is_empty());
        match &conn.status {
            ConnectionStatus::Connected => {}
            ConnectionStatus::Error(msg) => {
                push(format!("{key}/error"), NodeKind::ConnError, msg, 1, ci, None, false);
                continue;
            }
            ConnectionStatus::Connecting if open => {
                push(format!("{key}/loading"), NodeKind::Loading, &format!("Connecting to {}…", target(conn)), 1, ci, None, false);
                continue;
            }
            ConnectionStatus::Connecting | ConnectionStatus::Disconnected => continue,
        }
        if !open {
            continue;
        }
        for (di, db) in conn.databases.iter().enumerate() {
            let key = format!("{key}/{di}");
            if !push(key.clone(), NodeKind::Database, &db.name, 1, ci, None, !db.schemas.is_empty()) {
                continue;
            }
            for (si, schema) in db.schemas.iter().enumerate() {
                let key = format!("{key}/{si}");
                if !push(key.clone(), NodeKind::Schema, &schema.name, 2, ci, None, !schema.objects.is_empty()) {
                    continue;
                }
                if loading.contains(&key) {
                    push(format!("{key}/loading"), NodeKind::Loading, "Loading objects…", 3, ci, None, false);
                    continue;
                }
                let mut groups: BTreeMap<ObjectKind, Vec<&str>> = BTreeMap::new();
                for obj in &schema.objects {
                    groups.entry(obj.kind).or_default().push(&obj.name);
                }
                for (kind, names) in groups {
                    let key = format!("{key}/{}", kind.as_str());
                    let label = group_label(kind);
                    if !push(key.clone(), NodeKind::Group(kind), label, 3, ci, Some(names.len()), true) {
                        continue;
                    }
                    for (oi, name) in names.into_iter().enumerate() {
                        push(format!("{key}/{oi}"), NodeKind::Object(kind), name, 4, ci, None, false);
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
    workspace: Workspace,
    expanded: HashSet<String>,
    /// Schema keys whose objects have "arrived", and those still waiting on mock latency.
    loaded: HashSet<String>,
    loading: HashSet<String>,
    /// Type-to-filter query; non-empty shows only matches and their ancestors.
    filter: String,
    nodes: Vec<FlatNode>,
    selected: usize,
    /// False until the user picks a row, so launch doesn't highlight production as "here".
    picked: bool,
    /// Extra rows picked with Ctrl/Shift+click, by key so they survive re-flattening.
    marked: HashSet<String>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl Explorer {
    pub fn new(workspace: Workspace, cx: &mut Context<Self>) -> Self {
        // ponytail: starts collapsed; QX-020 restores the last session's expanded nodes
        let expanded = HashSet::new();
        let nodes = flatten(&workspace, &expanded, &HashSet::new(), false);
        let loaded = HashSet::new();
        Self { workspace, expanded, loaded, loading: HashSet::new(), filter: String::new(), nodes, selected: 0, picked: false, marked: HashSet::new(), scroll: UniformListScrollHandle::new(), focus: cx.focus_handle() }
    }

    fn set_expanded(&mut self, ix: usize, open: bool, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        if !node.expandable || node.expanded == open {
            return;
        }
        let key = node.key.clone();
        if open && matches!(node.kind, NodeKind::Connection(_)) && self.workspace.connections[node.conn].status != ConnectionStatus::Connected {
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
        let status = &mut self.workspace.connections[ci].status;
        if matches!(status, ConnectionStatus::Connected | ConnectionStatus::Connecting) {
            return;
        }
        *status = ConnectionStatus::Connecting;
        self.expanded.insert(format!("{ci}"));
        // ponytail: every mock connect succeeds, including retries; drivers report real failures (QX-023)
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(mock::latency()).await;
            this.update(cx, |this, cx| {
                this.workspace.connections[ci].status = ConnectionStatus::Connected;
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
        self.set_filter(String::new(), cx);
        if let Some(ix) = self.nodes.iter().position(|n| n.key == key) {
            self.select(ix, cx);
        }
        window.focus(&self.focus, cx);
    }

    /// `*`: opens the node and everything under it, skipping the loading step.
    fn expand_all_under(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(root) = self.nodes.get(ix).map(|n| n.key.clone()) else { return };
        let prefix = format!("{root}/");
        let all = flatten(&self.workspace, &HashSet::new(), &HashSet::new(), true);
        for n in all.into_iter().filter(|n| n.expandable && (n.key.starts_with(&prefix) || n.key == root)) {
            if n.kind == NodeKind::Schema {
                self.loaded.insert(n.key.clone());
            }
            self.expanded.insert(n.key);
        }
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.filter.is_empty() {
            self.nodes = flatten(&self.workspace, &self.expanded, &self.loading, false);
        }
        self.selected = self.selected.min(self.nodes.len().saturating_sub(1));
        cx.notify();
    }

    fn set_filter(&mut self, filter: String, cx: &mut Context<Self>) {
        self.filter = filter;
        self.nodes = if self.filter.is_empty() {
            flatten(&self.workspace, &self.expanded, &self.loading, false)
        } else {
            filter_nodes(flatten(&self.workspace, &self.expanded, &HashSet::new(), true), &self.filter)
        };
        // First match, or the top when clearing.
        let first = self.nodes.iter().position(|n| !self.filter.is_empty() && fuzzy_match(&n.label, &self.filter).is_some());
        self.select(first.unwrap_or(0), cx);
    }

    fn open(&mut self, ix: usize, preview: bool, cx: &mut Context<Self>) {
        let node = &self.nodes[ix];
        if let NodeKind::Object(ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView) = node.kind {
            let conn = &self.workspace.connections[node.conn];
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
                origin: Origin { color: conn.color, production: conn.is_production, path: names[..names.len() - 1].join(" › ").into() },
                path: names.join(" › ").into(),
                preview,
            });
        }
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        (self.selected, self.picked) = (ix, true);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = ev.keystroke.modifiers;
        match ev.keystroke.key.as_str() {
            "escape" if !self.filter.is_empty() => return self.set_filter(String::new(), cx),
            "escape" if !self.marked.is_empty() => {
                self.marked.clear();
                return cx.notify();
            }
            "backspace" if !self.filter.is_empty() => {
                let mut f = self.filter.clone();
                f.pop();
                return self.set_filter(f, cx);
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
            _ if !m.control && !m.alt && !m.platform => {
                if let Some(ch) = ev.keystroke.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) {
                    return self.set_filter(format!("{}{ch}", self.filter), cx);
                }
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
            "enter" if !node.expandable => self.open(ix, false, cx),
            "enter" if matches!(node.kind, NodeKind::Connection(_)) => self.set_expanded(ix, true, cx),
            "space" if !node.expandable => self.open(ix, true, cx),
            "enter" | "space" => self.set_expanded(ix, !node.expanded, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let theme = cx.theme();
        let focused = self.focus.contains_focused(window, cx);
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
                    NodeKind::Loading => (Icon::new(Lucide::LoaderCircle), theme.muted_foreground),
                    NodeKind::ConnError => (Icon::new(Lucide::CircleAlert), theme.danger),
                };
                let conn = &self.workspace.connections[node.conn];
                let is_conn = matches!(node.kind, NodeKind::Connection(_));
                let chevron = if node.expanded { Lucide::ChevronDown } else { Lucide::ChevronRight };
                let selected = ix == self.selected && (self.picked || focused);
                h_flex()
                    .id(ix)
                    .relative()
                    .h(theme::TREE_ROW)
                    .w_full()
                    .gap(theme::SPACE_6)
                    .pl(theme::SPACE_4 + theme::SPACE_12 * node.depth as f32)
                    .pr(theme::SPACE_8)
                    .when(!selected && self.marked.contains(&node.key), |d| d.bg(theme::selected_inactive(theme)))
                    .when(selected, |d| d.bg(if focused { theme.list_active } else { theme::selected_inactive(theme) }))
                    .when(!selected, |d| d.hover(|d| d.bg(theme.list_hover)))
                    // An overlay, so selection and hover don't wipe out production.
                    .when(conn.is_production, |d| d.child(div().absolute().inset_0().bg(theme::production_tint(theme))))
                    // Indent guides, one hairline per ancestor level.
                    .children((1..node.depth).map(|level| {
                        div()
                            .absolute()
                            .left(theme::SPACE_4 + theme::SPACE_12 * level as f32 + theme::SPACE_6)
                            .top_0()
                            .h_full()
                            .w(px(1.))
                            .bg(theme.border)
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
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
                            this.marked.clear();
                            this.select(ix, cx);
                            if ev.click_count == 1 {
                                let open = this.nodes[ix].expanded;
                                this.set_expanded(ix, !open, cx);
                            }
                            this.open(ix, ev.click_count < 2, cx);
                        }),
                    )
                    .child(
                        div().flex_none().w(theme::SPACE_12).when(node.expandable, |d| {
                            d.child(Icon::new(chevron).xsmall().text_color(theme.muted_foreground))
                        }),
                    )
                    .child(icon.small().text_color(color))
                    .when(is_conn, |d| {
                        // Status in the color dot: solid = connected, hollow = disconnected or
                        // connecting, red = error (message in the tooltip).
                        let color = theme::connection_color(conn.color, theme);
                        let dot = div().flex_none().size(theme::SPACE_6).rounded_full();
                        d.child(match &conn.status {
                            ConnectionStatus::Connected => dot.bg(color).into_any_element(),
                            ConnectionStatus::Disconnected | ConnectionStatus::Connecting => {
                                dot.border_1().border_color(color).into_any_element()
                            }
                            // Hollow in the connection's own color: red belongs to production, and the
                            // error already has its own row below.
                            ConnectionStatus::Error(_) => dot.border_1().border_color(color).into_any_element(),
                        })
                    })
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child({
                        let hits = fuzzy_match(&node.label, &self.filter).filter(|_| !self.filter.is_empty());
                        let style = HighlightStyle { color: Some(theme.primary), ..Default::default() };
                        let ranges = hits.unwrap_or_default().into_iter().map(|at| {
                            let len = node.label[at..].chars().next().map_or(1, char::len_utf8);
                            (at..at + len, style)
                        });
                        StyledText::new(node.label.clone()).with_highlights(ranges.collect::<Vec<_>>())
                    }).when(matches!(node.kind, NodeKind::Loading | NodeKind::ConnError), |d| d.text_size(theme::UI_SM).text_color(theme.muted_foreground)))
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
                    .when_some(node.count, |d, n| {
                        d.child(div().text_size(theme::UI_SM).text_color(theme.muted_foreground).child(n.to_string()))
                    })
                    .when(is_conn && conn.is_production, |d| {
                        d.child(theme::prod_badge(theme))
                    })
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
            .when(!self.filter.is_empty(), |d| {
                d.child(
                    h_flex()
                        .flex_none()
                        .h(theme::TREE_ROW)
                        .px(theme::SPACE_8)
                        .gap(theme::SPACE_6)
                        .border_b_1()
                        .border_color(theme.border)
                        .text_size(theme::UI_SM)
                        .child(Icon::new(Lucide::Search).xsmall().text_color(theme.muted_foreground))
                        .child(div().flex_1().child(self.filter.clone()))
                        .child(div().text_color(theme.muted_foreground).child("Esc clears")),
                )
            })
            .map(|d| if self.nodes.is_empty() && !self.filter.is_empty() {
                d.child(
                    v_flex()
                        .flex_1()
                        .p(theme::SPACE_16)
                        .gap(theme::SPACE_4)
                        .text_size(theme::UI_SM)
                        .child(format!("Nothing matches “{}”", self.filter))
                        .child(div().text_color(theme.muted_foreground).child(
                            "Only connected databases are searched. Backspace edits the filter, Esc clears it.",
                        )),
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
    use super::{NodeKind, default_expanded, filter_nodes, flatten, fuzzy_match, parent_of};
    use crate::{mock, model::{ConnectionStatus, ObjectKind}};

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
}
