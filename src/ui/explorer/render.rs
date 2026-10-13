//! The explorer panel: header, filter field and tree.

use super::*;

/// Whether the status dot is solid (else a ring), and the tooltip naming the status.
pub(crate) fn status_style(status: &ConnectionStatus) -> (bool, String) {
    match status {
        ConnectionStatus::Connected => (true, "Connected".into()),
        ConnectionStatus::Disconnected => (false, "Disconnected".into()),
        ConnectionStatus::Connecting => (false, "Connecting…".into()),
        ConnectionStatus::Error(msg) => (false, format!("Could not connect: {msg}")),
    }
}

impl Render for Explorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &crate::RefreshNode, _, cx| {
                this.refresh_row(this.selected, cx)
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
                            .child(
                                Input::new(&self.field).small().prefix(
                                    Icon::new(Lucide::Search)
                                        .xsmall()
                                        .text_color(theme.muted_foreground),
                                ),
                            ),
                    )
                    .child(
                        div().debug_selector(|| "add-connection".into()).child(
                            Button::new("add-connection")
                                .ghost()
                                .small()
                                .icon(Lucide::Plus)
                                .tooltip("Add connection")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.add_connection(window, cx)
                                })),
                        ),
                    ),
            )
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
                            .gap(theme::space_8())
                            .child(
                                div()
                                    .text_size(theme::ui_sm())
                                    .text_color(theme.muted_foreground)
                                    .child("No connections yet"),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "empty-add-connection".into())
                                    .child(
                                        Button::new("empty-add-connection")
                                            .primary()
                                            .small()
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
