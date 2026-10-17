//! New workspace and Rename workspace: one name field.

use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div,
};

use crate::theme;

/// The dialog's body. Save emits `Named` with the trimmed name; a blank one does nothing.
pub struct NameDialog {
    pub name: Entity<InputState>,
}

/// Emitted on Save with the typed name.
pub struct Named(pub String);

impl EventEmitter<Named> for NameDialog {}

impl NameDialog {
    /// Opens `title` over `window` with `text` in the focused field.
    pub fn open(title: &str, text: &str, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| Self {
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Workspace name")
                    .default_value(text.to_string())
            }),
        });
        let body = view.clone();
        let title = title.to_string();
        window.open_dialog(cx, move |dialog, _, _| {
            let on_enter = body.clone();
            dialog
                .title(title.clone())
                .child(body.clone())
                .on_ok(move |_, _, cx| on_enter.update(cx, |d, cx| d.save(cx)))
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1())
                        .child(
                            Button::new("name-cancel")
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("name-save")
                                .debug_selector(|| "name-save".into())
                                .primary()
                                .small()
                                .label("Save")
                                .on_click({
                                    let body = body.clone();
                                    move |_, window, cx| {
                                        if body.update(cx, |d, cx| d.save(cx)) {
                                            window.close_dialog(cx);
                                        }
                                    }
                                }),
                        ),
                )
        });
        view.read(cx)
            .name
            .clone()
            .update(cx, |name, cx| name.focus(window, cx));
        view
    }

    /// Emits `Named` unless the name is blank, and returns whether it did.
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return false;
        }
        cx.emit(Named(name));
        true
    }
}

impl Render for NameDialog {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Input::new(&self.name).small()
    }
}
