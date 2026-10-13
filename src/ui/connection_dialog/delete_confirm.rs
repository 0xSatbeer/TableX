use super::*;

/// The delete confirmation's body. Confirming emits `Confirmed`; the caller removes the connection.
pub struct DeleteConfirm {
    name: String,
    is_production: bool,
    /// Where a Production connection's name is typed to enable the button.
    pub typed: Entity<InputState>,
}

/// Emitted when the user confirms the delete.
pub struct Confirmed;

impl EventEmitter<Confirmed> for DeleteConfirm {}

impl DeleteConfirm {
    /// Opens "Delete connection <name>?" over `window`.
    pub fn open(conn: &Connection, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| {
            let typed = cx.new(|cx| InputState::new(window, cx).placeholder("Connection name"));
            cx.subscribe(&typed, |_, _, _: &InputEvent, cx| cx.notify())
                .detach();
            Self {
                name: conn.name.clone(),
                is_production: conn.is_production,
                typed,
            }
        });
        let body = view.clone();
        let title = format!("Delete connection {}?", conn.name);
        window.open_dialog(cx, move |dialog, _, cx| {
            let title = title.clone();
            let enabled = body.read(cx).enabled(cx);
            let on_enter = body.clone();
            dialog
                .title(
                    div()
                        .debug_selector({
                            let title = title.clone();
                            move || format!("delete-title {title}")
                        })
                        .child(title),
                )
                .child(body.clone())
                .on_ok(move |_, _, cx| on_enter.update(cx, |d, cx| d.confirm(cx)))
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1())
                        .child(
                            Button::new("delete-cancel")
                                .debug_selector(|| "delete-cancel".into())
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("delete-confirm")
                                .debug_selector(|| "delete-confirm".into())
                                .danger()
                                .small()
                                .label("Delete connection")
                                .disabled(!enabled)
                                .on_click({
                                    let body = body.clone();
                                    move |_, window, cx| {
                                        if body.update(cx, |d, cx| d.confirm(cx)) {
                                            window.close_dialog(cx);
                                        }
                                    }
                                }),
                        ),
                )
        });
        if conn.is_production {
            view.read(cx)
                .typed
                .clone()
                .update(cx, |typed, cx| typed.focus(window, cx));
        }
        view
    }

    /// Whether Delete connection is enabled: at once, or for Production once the name is typed.
    pub fn enabled(&self, cx: &App) -> bool {
        delete_enabled(&self.name, self.is_production, &self.typed.read(cx).value())
    }

    /// Emits `Confirmed` if the delete is enabled, and returns whether it did.
    fn confirm(&mut self, cx: &mut Context<Self>) -> bool {
        let enabled = self.enabled(cx);
        if enabled {
            cx.emit(Confirmed);
        }
        enabled
    }
}

impl Render for DeleteConfirm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let note = div()
            .text_size(theme::ui_sm())
            .text_color(theme::text_secondary(cx.theme()))
            .child(format!(
                "{} is removed from the workspace and its tabs close. This can't be undone.",
                self.name
            ));
        v_flex()
            .gap(theme::space_12())
            .child(note)
            .when(self.is_production, |d| {
                d.child(
                    v_flex()
                        .debug_selector(|| "delete-name".into())
                        .gap(theme::space_4())
                        .child(
                            div()
                                .text_size(theme::ui_sm())
                                .text_color(theme::text_secondary(cx.theme()))
                                .child(format!(
                                    "{} is a Production connection. Type its name to confirm.",
                                    self.name
                                )),
                        )
                        .child(Input::new(&self.typed).small()),
                )
            })
    }
}
