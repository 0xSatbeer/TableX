//! The explorer panel: header, filter field and tree.

use super::*;
use gpui_kit::component::{Disableable as _, Selectable as _};

/// Whether the status dot is solid (else a ring), and the tooltip naming the status.
pub(crate) fn status_style(status: &ConnectionStatus) -> (bool, String) {
    match status {
        ConnectionStatus::Connected => (true, "Connected".into()),
        ConnectionStatus::Disconnected => (false, "Disconnected".into()),
        ConnectionStatus::Connecting => (false, "Connecting…".into()),
        ConnectionStatus::Error(msg) => (false, format!("Could not connect: {msg}")),
    }
}

fn tool(id: &'static str, icon: Lucide, tip: &'static str, enabled: bool) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(icon)
        .tooltip(tip)
        .accessibility_label(tip)
        .disabled(!enabled)
}

/// Wraps a toolbar button so tests can find it by its id.
fn slot(id: &'static str, button: Button) -> Div {
    div().debug_selector(move || id.into()).child(button)
}

/// What the toolbar can act on for the selected row: the same gates as its context menu.
#[derive(Default)]
struct Enabled {
    edit: bool,
    refresh: bool,
    disconnect: bool,
    open: bool,
    modify: bool,
    console: bool,
}

impl Explorer {
    fn enabled(&self, cx: &App) -> Enabled {
        let ws = self.workspace.read(cx);
        let Some(node) = self.nodes.get(self.selected) else {
            return Enabled::default();
        };
        let conn = &ws.connections[node.conn];
        let connected = conn.status == ConnectionStatus::Connected;
        let a = row_actions(node.kind, conn.kind(), connected, &node.label);
        Enabled {
            edit: true,
            refresh: a.is_some(),
            disconnect: connected,
            open: a.is_some_and(|a| a.object.is_some_and(|(opens, _)| opens)),
            modify: a.is_some_and(|a| matches!(a.menu, RowMenu::Modify(_))),
            console: self.console_place(cx).is_some(),
        }
    }

    /// Edit connection from any row: opens the dialog on that row's connection.
    fn edit_selected_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };
        let id = self.workspace.read(cx).connections[node.conn].id;
        self.open_dialog(Some(id), window, cx);
    }

    /// Open console from any row: on its connection, database and schema, as New console picks.
    fn console_selected(&mut self, cx: &mut Context<Self>) {
        if let Some((conn, database, schema)) = self.console_place(cx) {
            cx.emit(OpenConsole {
                conn,
                database,
                schema,
            });
        }
    }

    /// The icon row above the filter: add/edit, refresh/disconnect, open/modify/console/DDL.
    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let on = self.enabled(cx);
        let theme = cx.theme();
        let sep = || {
            div()
                .flex_none()
                .w_px()
                .h(theme::space_16())
                .mx(theme::space_2())
                .bg(theme.border)
        };
        // A solid stop square, red while there is a session to close.
        let stop = div()
            .size(theme::space_12() - theme::space_2())
            .rounded(px(2.))
            .bg(if on.disconnect {
                theme.danger
            } else {
                theme.muted_foreground.opacity(0.4)
            });
        h_flex()
            .flex_none()
            .debug_selector(|| "explorer-toolbar".into())
            .gap(theme::space_2())
            .px(theme::space_4())
            .py(theme::space_4())
            .border_b_1()
            .border_color(theme.border)
            .child(slot(
                "add-connection",
                tool("add-connection", Lucide::Plus, "Add connection", true)
                    .on_click(cx.listener(|this, _, w, cx| this.add_connection(w, cx))),
            ))
            .child(slot(
                "tb-edit",
                tool("tb-edit", Lucide::Pencil, "Edit connection", on.edit)
                    .on_click(cx.listener(|this, _, w, cx| this.edit_selected_connection(w, cx))),
            ))
            .child(sep())
            .child(slot(
                "tb-refresh",
                tool("tb-refresh", Lucide::RefreshCw, "Refresh", on.refresh)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_row(this.selected, cx))),
            ))
            .child(slot(
                "tb-disconnect",
                Button::new("tb-disconnect")
                    .ghost()
                    .small()
                    .tooltip("Disconnect")
                    .accessibility_label("Disconnect")
                    .disabled(!on.disconnect)
                    .child(stop)
                    .on_click(cx.listener(|this, _, _, cx| this.disconnect_selected(cx))),
            ))
            .child(sep())
            .child(slot(
                "tb-open",
                tool("tb-open", Lucide::Table, "Open", on.open)
                    .on_click(cx.listener(|this, _, _, cx| this.open_selected(cx))),
            ))
            .child(slot(
                "tb-modify",
                tool("tb-modify", Lucide::SquarePen, "Modify", on.modify)
                    .on_click(cx.listener(|this, _, w, cx| this.modify_object(w, cx))),
            ))
            .child(slot(
                "tb-console",
                tool(
                    "tb-console",
                    Lucide::SquareTerminal,
                    "Open console",
                    on.console,
                )
                .on_click(cx.listener(|this, _, _, cx| this.console_selected(cx))),
            ))
            .child(slot(
                "tb-ddl",
                tool("tb-ddl", Lucide::Code, "Show DDL", on.open)
                    .on_click(cx.listener(|this, _, _, cx| this.show_ddl(cx))),
            ))
            .child(div().flex_1())
            .child(slot(
                "tb-filter",
                tool(
                    "tb-filter",
                    Lucide::Funnel,
                    "Filter objects (Ctrl+Alt+F)",
                    true,
                )
                .selected(self.filter_shown())
                .on_click(cx.listener(|this, _, w, cx| this.toggle_filter(w, cx))),
            ))
    }
}

impl Render for Explorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toolbar = self.toolbar(cx);
        let theme = cx.theme();
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &crate::RefreshNode, _, cx| {
                this.refresh_row(this.selected, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::Disconnect, _, cx| this.disconnect_selected(cx)))
            .on_action(cx.listener(|this, _: &crate::OpenConsole, _, cx| this.open_console(cx)))
            .on_action(cx.listener(|this, _: &crate::OpenSelected, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &crate::ShowDdl, _, cx| this.show_ddl(cx)))
            .on_action(cx.listener(|this, _: &crate::CopyName, _, cx| this.copy_name(cx)))
            .on_action(cx.listener(|this, _: &crate::CopyQualifiedName, _, cx| {
                this.copy_qualified_name(cx)
            }))
            .on_action(cx.listener(|this, _: &crate::DropObject, window, cx| {
                this.drop_object(window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::ModifyObject, window, cx| {
                this.modify_object(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::NewTable, window, cx| this.new_table(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &crate::NewDatabase, window, cx| {
                    this.new_database(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::NewRole, window, cx| {
                    this.new_role(false, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::NewUser, window, cx| this.new_role(true, window, cx)),
            )
            .child(toolbar)
            .when(self.filter_shown(), |d| d.child(
                div()
                    .flex_none()
                    .p(theme::space_6())
                    .border_b_1()
                    .border_color(theme.border)
                    .capture_key_down(cx.listener(Self::on_field_key))
                    .child(
                        Input::new(&self.field).small().prefix(
                            Icon::new(Lucide::Search)
                                .xsmall()
                                .text_color(theme.muted_foreground),
                        ),
                    ),
            ))
            .when_some(
                self.rename.as_ref().and_then(|rn| rn.error.clone()),
                |d, msg| {
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
                },
            )
            .map(|d| {
                if self.workspace.read(cx).connections.is_empty() {
                    d.child(
                        v_flex()
                            .debug_selector(|| "explorer-empty".into())
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .px(theme::space_24())
                            .gap(theme::space_4())
                            .child(
                                div()
                                    .size(theme::space_24() + theme::space_16())
                                    .mb(theme::space_8())
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(theme::radius_lg())
                                    .bg(theme.muted)
                                    .text_color(theme.muted_foreground)
                                    .child(Icon::new(Lucide::Database).size(theme::space_16() + theme::space_4())),
                            )
                            .child(
                                div()
                                    .text_size(theme::ui())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.foreground)
                                    .child("No connections yet"),
                            )
                            .child(
                                div()
                                    .text_size(theme::ui_sm())
                                    .text_color(theme.muted_foreground)
                                    .text_center()
                                    .child("Connect to PostgreSQL or SQLite to browse schemas and run queries."),
                            )
                            .child(
                                div()
                                    .mt(theme::space_12())
                                    .debug_selector(|| "empty-add-connection".into())
                                    .child(
                                        Button::new("empty-add-connection")
                                            .primary()
                                            .small()
                                            .icon(Lucide::Plus)
                                            .label("Add connection")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.add_connection(window, cx)
                                            })),
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
                        uniform_list(
                            "explorer-rows",
                            self.nodes.len(),
                            cx.processor(Self::render_rows),
                        )
                        .track_scroll(&self.scroll)
                        .flex_1(),
                    )
                }
            })
    }
}
