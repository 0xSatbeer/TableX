//! The object dialogs (Modify…), and the pieces they share: a labeled field, the SQL preview,
//! the production confirmation, the Cancel/Apply footer and the error line. The explorer runs
//! the statements a dialog's `Apply` carries and reports a failure back with `failed`.

mod name_owner;
mod role;

pub use name_owner::*;
pub use role::*;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div,
};

use crate::theme;
use crate::ui::connection_dialog::delete_enabled;

/// Emitted when Apply is pressed with the statements to run.
pub struct Apply(pub Vec<String>);

/// A labeled control. `id` names it for tests (`field-<id>`).
fn field(id: &'static str, label: String, control: impl IntoElement, cx: &App) -> AnyElement {
    v_flex()
        .debug_selector(move || format!("field-{id}"))
        .gap(theme::space_4())
        .child(
            div()
                .text_size(theme::ui_sm())
                .text_color(theme::text_secondary(cx.theme()))
                .child(label),
        )
        .child(control)
        .into_any_element()
}

/// The statements Apply runs, read-only, one per line.
pub fn sql_preview(statements: &[String], cx: &App) -> AnyElement {
    let theme = cx.theme();
    let text = if statements.is_empty() {
        "-- No changes".to_string()
    } else {
        statements
            .iter()
            .map(|s| format!("{s};"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    div()
        .debug_selector(|| "object-sql".into())
        .p(theme::space_8())
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_size(theme::mono())
        .whitespace_normal()
        .child(text)
        .into_any_element()
}

/// The failure under the form, if any.
pub fn error_line(error: Option<&str>, cx: &App) -> Option<AnyElement> {
    let error = error?.to_string();
    Some(
        div()
            .debug_selector(|| "object-error".into())
            .text_size(theme::ui_sm())
            .text_color(cx.theme().danger)
            .child(error)
            .into_any_element(),
    )
}

/// Cancel, and Apply when `enabled`.
pub fn footer(enabled: bool, on_apply: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    h_flex()
        .gap(theme::space_8())
        .child(div().flex_1())
        .child(
            Button::new("object-cancel")
                .debug_selector(|| "object-cancel".into())
                .small()
                .label("Cancel")
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(
            Button::new("object-apply")
                .debug_selector(|| "object-apply".into())
                .primary()
                .small()
                .label("Apply")
                .disabled(!enabled)
                .on_click(move |_, window, cx| on_apply(window, cx)),
        )
        .into_any_element()
}

/// On a production connection, the object's name typed to enable Apply.
pub struct ProductionConfirm {
    name: String,
    is_production: bool,
    pub typed: Entity<InputState>,
}

impl ProductionConfirm {
    /// A confirmation for object `name`; typing in it re-renders the dialog `cx` belongs to.
    pub fn new<V: 'static>(
        name: &str,
        is_production: bool,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        let typed = cx.new(|cx| InputState::new(window, cx).placeholder(name.to_string()));
        cx.subscribe(&typed, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        Self {
            name: name.into(),
            is_production,
            typed,
        }
    }

    /// At once, or on production once the name is typed exactly.
    pub fn confirmed(&self, cx: &App) -> bool {
        delete_enabled(&self.name, self.is_production, &self.typed.read(cx).value())
    }

    /// The input, on production only.
    pub fn render(&self, cx: &App) -> Option<AnyElement> {
        self.is_production.then(|| {
            div()
                .debug_selector(|| "object-confirm".into())
                .child(field(
                    "confirm",
                    format!("Type {} to confirm", self.name),
                    Input::new(&self.typed).small(),
                    cx,
                ))
                .into_any_element()
        })
    }
}
