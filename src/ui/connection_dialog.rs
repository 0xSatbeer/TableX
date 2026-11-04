//! The add/edit connection dialog, and the pure rules behind it: validation, defaults, delete
//! confirmation and the Test connection outcome. Whether an edit changes the params is
//! `Workspace::edit`'s call.

mod actions;
mod delete_confirm;
mod form;
mod render;

pub use delete_confirm::*;
pub use form::*;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonGroup, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Task,
    Window, div,
};

use crate::db::Driver;
use crate::model::*;
use crate::theme;
use crate::ui::explorer::validate_rename;

/// The text fields, by the name the errors and tests use.
const TEXT_FIELDS: [&str; 7] = [
    "name", "host", "port", "user", "password", "database", "path",
];

/// SSL choices in the order the select lists them.
const SSL_MODES: [(SslMode, &str); 3] = [
    (SslMode::Disable, "Disable"),
    (SslMode::Prefer, "Prefer"),
    (SslMode::Require, "Require"),
];

/// The Add/Edit connection dialog's body. The dialog itself lives in the window's dialog layer.
pub struct ConnectionDialog {
    workspace: Entity<Workspace>,
    /// Test connection goes through it.
    driver: Arc<dyn Driver>,
    /// The connection being edited; `None` adds a new one.
    editing: Option<ConnectionId>,
    onboarding: bool,
    color: u8,
    is_production: bool,
    save_password: bool,
    /// The keychain failed to give the saved password on open, so an empty field means unknown.
    prefill_failed: bool,
    kind: DbKind,
    pub name: Entity<InputState>,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub user: Entity<InputState>,
    pub password: Entity<InputState>,
    pub database: Entity<InputState>,
    pub path: Entity<InputState>,
    pub ssl: Entity<SelectState<Vec<&'static str>>>,
    /// Fields the user has edited; only these show their error until a save is attempted.
    touched: HashSet<&'static str>,
    /// The running Test connection; dropping it discards its result.
    test: Option<Task<()>>,
    /// The last test's outcome, shown above the footer.
    test_result: Option<Result<(), String>>,
}

/// Emitted after Save added or edited connection `.0`.
pub struct Saved(pub ConnectionId);

impl EventEmitter<Saved> for ConnectionDialog {}

impl ConnectionDialog {
    /// Opens the Add connection dialog, or the Edit dialog for `editing`, over `window`, with
    /// the name field focused.
    pub fn open(
        workspace: Entity<Workspace>,
        driver: Arc<dyn Driver>,
        editing: Option<ConnectionId>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(workspace, driver, editing, window, cx));
        let body = view.clone();
        let (title, save_label) = if editing.is_some() {
            ("Edit connection", "Save changes")
        } else {
            ("Add connection", "Add connection")
        };
        window.open_dialog(cx, move |dialog, _, cx| {
            let save_enabled = body.read(cx).save_enabled(cx);
            let (test_label, test_enabled) = body.read(cx).test_button(cx);
            let on_enter = body.clone();
            dialog
                .title(title)
                .child(body.clone())
                .on_ok(move |_, _, cx| on_enter.update(cx, |d, cx| d.save(cx)))
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(
                            Button::new("dialog-test")
                                .debug_selector(|| "dialog-test".into())
                                .small()
                                .label(test_label)
                                .disabled(!test_enabled)
                                .on_click({
                                    let body = body.clone();
                                    move |_, _, cx| body.update(cx, |d, cx| d.run_test(cx))
                                }),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("dialog-cancel")
                                .debug_selector(|| "dialog-cancel".into())
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("dialog-save")
                                .debug_selector(|| "dialog-save".into())
                                .primary()
                                .small()
                                .label(save_label)
                                .disabled(!save_enabled)
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

    /// The Add form alone, name field focused, for a host that draws its own buttons (the
    /// welcome tour's last step).
    pub fn embedded(
        workspace: Entity<Workspace>,
        driver: Arc<dyn Driver>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(workspace, driver, None, window, cx));
        view.update(cx, |form, _| form.onboarding = true);
        view.read(cx)
            .name
            .clone()
            .update(cx, |name, cx| name.focus(window, cx));
        view
    }

    fn new(
        workspace: Entity<Workspace>,
        driver: Arc<dyn Driver>,
        editing: Option<ConnectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let connections = &workspace.read(cx).connections;
        let mut form = match connections.iter().find(|c| Some(c.id) == editing) {
            Some(conn) => Form::of(conn),
            None => Form::new(&connections.iter().map(|c| c.color).collect::<Vec<_>>()),
        };
        let mut prefill_failed = false;
        if let Some(id) = editing.filter(|_| form.save_password && form.password.is_empty()) {
            match crate::persist::Keys::of(cx).get(id) {
                Ok(saved) => form.password = saved.map_or(String::new(), |p| p.expose().into()),
                Err(_) => prefill_failed = true,
            }
        }
        let mut input = |value: &str, placeholder: &str| {
            let (value, placeholder) = (value.to_string(), placeholder.to_string());
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder(placeholder)
            })
        };
        let (name, host, port) = (
            input(&form.name, "e.g. prod-pg"),
            input(&form.host, "e.g. localhost"),
            input(&form.port, ""),
        );
        let (user, database, path) = (
            input(&form.user, "e.g. postgres"),
            input(&form.database, ""),
            input(&form.path, "e.g. /home/me/app.db"),
        );
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .default_value(form.password.clone())
        });
        let selected = SSL_MODES.iter().position(|(mode, _)| *mode == form.ssl);
        let ssl = cx.new(|cx| {
            SelectState::new(
                SSL_MODES.iter().map(|(_, label)| *label).collect(),
                selected.map(IndexPath::new),
                window,
                cx,
            )
        });
        for (id, input) in TEXT_FIELDS
            .into_iter()
            .zip([&name, &host, &port, &user, &password, &database, &path])
        {
            cx.subscribe(input, move |this, _, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    this.touched.insert(id);
                    if id != "name" {
                        this.connection_changed();
                    }
                    cx.notify();
                }
            })
            .detach();
        }
        cx.subscribe(&ssl, |this, _, _: &SelectEvent<Vec<&'static str>>, cx| {
            this.connection_changed();
            cx.notify();
        })
        .detach();
        Self {
            workspace,
            driver,
            editing,
            onboarding: false,
            color: form.color,
            is_production: form.is_production,
            save_password: form.save_password,
            prefill_failed,
            kind: form.kind,
            name,
            host,
            port,
            user,
            password,
            database,
            path,
            ssl,
            touched: HashSet::new(),
            test: None,
            test_result: None,
        }
    }
}

#[cfg(test)]
mod tests;
