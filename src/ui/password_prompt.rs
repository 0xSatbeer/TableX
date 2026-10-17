//! Asks for a password that isn't saved, on connect, and tells once when the keychain fails.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    notification::Notification,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div,
};

use crate::model::Secret;
use crate::persist::Keys;
use crate::theme;

/// The prompt's body. Connect emits `Entered`; the caller connects with it.
pub struct PasswordPrompt {
    name: String,
    pub password: Entity<InputState>,
}

/// Emitted on Connect with the typed password, empty included.
pub struct Entered(pub Secret);

impl EventEmitter<Entered> for PasswordPrompt {}

impl PasswordPrompt {
    /// Opens "Password for <name>" over `window`, with the field focused.
    pub fn open(name: &str, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| Self {
            name: name.to_string(),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
        });
        let body = view.clone();
        let title = format!("Password for {name}");
        window.open_dialog(cx, move |dialog, _, _| {
            let on_enter = body.clone();
            dialog
                .title(title.clone())
                .child(body.clone())
                .on_ok(move |_, _, cx| {
                    on_enter.update(cx, |p, cx| p.connect(cx));
                    true
                })
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1())
                        .child(
                            Button::new("prompt-cancel")
                                .debug_selector(|| "prompt-cancel".into())
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("prompt-connect")
                                .debug_selector(|| "prompt-connect".into())
                                .primary()
                                .small()
                                .label("Connect")
                                .on_click({
                                    let body = body.clone();
                                    move |_, window, cx| {
                                        body.update(cx, |p, cx| p.connect(cx));
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
        view.read(cx)
            .password
            .clone()
            .update(cx, |password, cx| password.focus(window, cx));
        view
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let typed = self.password.read(cx).value().to_string();
        cx.emit(Entered(Secret::new(typed)));
    }
}

impl Render for PasswordPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(theme::space_8())
            .child(
                div()
                    .text_size(theme::ui_sm())
                    .text_color(theme::text_secondary(cx.theme()))
                    .child(format!(
                        "{} has no saved password. It is kept until QueryX quits.",
                        self.name
                    )),
            )
            .child(Input::new(&self.password).small())
    }
}

/// Shows `error` as a notification on the active window, the first time only.
pub fn keychain_failed(error: &str, cx: &mut App) {
    let Some(notice) = Keys::first_error(cx, error) else {
        return;
    };
    // Deferred: callers run inside entity or window updates.
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| {
                window.push_notification(Notification::error(notice), cx)
            });
        }
    });
}
