//! Rendering the tree rows and what a press on them does.

use super::*;

/// Where a mouse press landed on a row.
#[derive(Clone, Copy, Debug)]
pub(super) enum Hit {
    Chevron,
    Row { clicks: usize },
}

/// What a press does after selecting the row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Click {
    Select,
    Toggle,
    Open { preview: bool },
}

pub(super) fn click_action(kind: NodeKind, hit: Hit) -> Click {
    match hit {
        Hit::Chevron => Click::Toggle,
        Hit::Row { clicks } if opens_tab(kind) => Click::Open {
            preview: clicks < 2,
        },
        Hit::Row { clicks: 2 } => Click::Toggle,
        Hit::Row { .. } => Click::Select,
    }
}

/// A row's menu: a connection's, a Postgres database's, schema's or table's, or Refresh alone.
#[derive(Clone, Copy)]
enum Menu {
    Connection,
    Modify,
    Refresh,
}

/// Table-like objects open in a tab and expand into their structure.
pub(super) fn opens_tab(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Object(ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView)
    )
}

impl Explorer {
    pub(super) fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = cx.theme();
        let focused = self.focus.contains_focused(window, cx);
        let ws = self.workspace.read(cx);
        let menus: Vec<Option<Menu>> = range
            .clone()
            .map(|ix| match self.nodes[ix].kind {
                NodeKind::Connection(_) => Some(Menu::Connection),
                NodeKind::Database | NodeKind::Schema | NodeKind::Object(ObjectKind::Table)
                    if ws.connections[self.nodes[ix].conn].kind() == DbKind::Postgres =>
                {
                    Some(Menu::Modify)
                }
                NodeKind::Database | NodeKind::Schema => Some(Menu::Refresh),
                kind if opens_tab(kind) => Some(Menu::Refresh),
                _ => None,
            })
            .collect();
        // + New: Postgres database, schema and table rows.
        let news: Vec<bool> = range
            .clone()
            .map(|ix| {
                let node = &self.nodes[ix];
                matches!(
                    node.kind,
                    NodeKind::Database | NodeKind::Schema | NodeKind::Object(ObjectKind::Table)
                ) && ws.connections[node.conn].kind() == DbKind::Postgres
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
                    NodeKind::Loading => (Icon::empty(), theme.muted_foreground),
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
                    .map(|d| match node.kind {
                        NodeKind::Loading => d.child(Spinner::new().small().color(color)),
                        _ => d.child(icon.small().text_color(color)),
                    })
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
                    .when(is_conn && self.refreshing(&node.key), |d| {
                        d.child(
                            div()
                                .flex_none()
                                .debug_selector(move || format!("conn-refreshing-{ix}"))
                                .child(Spinner::new().xsmall().color(theme.muted_foreground)),
                        )
                    })
                    .when(is_conn && conn.is_production, |d| {
                        d.child(theme::prod_badge(theme).debug_selector(|| format!("prod-badge-{ix}")))
                    })
            })
            .zip(menus.into_iter().zip(news))
            .map(|(row, (menu, new))| match menu {
                Some(kind) => row
                    .context_menu(move |menu, window, cx| {
                        let menu = match kind {
                            Menu::Connection => menu.menu_element(Box::new(crate::EditConnection), |_, _| div().debug_selector(|| "menu-edit".into()).child("Edit…")).menu_element(Box::new(crate::DeleteConnection), |_, cx| {
                                div().debug_selector(|| "menu-delete".into()).text_color(cx.theme().danger).child("Delete")
                            }),
                            Menu::Modify => menu.menu_element(Box::new(crate::ModifyObject), |_, _| div().debug_selector(|| "menu-modify".into()).child("Modify…")),
                            Menu::Refresh => menu,
                        };
                        let menu = if new {
                            menu.submenu("+ New", window, cx, |menu, _, _| {
                                menu.menu_element(Box::new(crate::NewTable), |_, _| div().debug_selector(|| "menu-new-table".into()).child("Table"))
                                    .menu_element(Box::new(crate::NewDatabase), |_, _| div().debug_selector(|| "menu-new-database".into()).child("Database"))
                                    .menu_element(Box::new(crate::NewRole), |_, _| div().debug_selector(|| "menu-new-role".into()).child("Role"))
                                    .menu_element(Box::new(crate::NewUser), |_, _| div().debug_selector(|| "menu-new-user".into()).child("User"))
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
