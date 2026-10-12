//! The add/edit connection dialog, and the pure rules behind it: validation, defaults, delete
//! confirmation and the Test connection outcome. Whether an edit changes the params is
//! `Workspace::edit`'s call.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonGroup, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Hsla, InteractiveElement as _, IntoElement, StatefulInteractiveElement as _, ParentElement as _, Render, Styled as _, Task, Window, div};

use crate::db::Driver;
use crate::explorer::validate_rename;
use crate::model::*;
use crate::{mock, theme};

/// The text fields, by the name the errors and tests use.
const TEXT_FIELDS: [&str; 7] = ["name", "host", "port", "user", "password", "database", "path"];

/// SSL choices in the order the select lists them.
const SSL_MODES: [(SslMode, &str); 3] = [(SslMode::Disable, "Disable"), (SslMode::Prefer, "Prefer"), (SslMode::Require, "Require")];

/// The Add/Edit connection dialog's body. The dialog itself lives in the window's dialog layer.
pub struct ConnectionDialog {
    workspace: Entity<Workspace>,
    /// Test connection goes through it.
    driver: Arc<dyn Driver>,
    /// The connection being edited; `None` adds a new one.
    editing: Option<ConnectionId>,
    color: u8,
    is_production: bool,
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
    pub fn open(workspace: Entity<Workspace>, driver: Arc<dyn Driver>, editing: Option<ConnectionId>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(workspace, driver, editing, window, cx));
        let body = view.clone();
        let (title, save_label) = if editing.is_some() { ("Edit connection", "Save changes") } else { ("Add connection", "Add connection") };
        window.open_dialog(cx, move |dialog, _, cx| {
            let save_enabled = body.read(cx).save_enabled(cx);
            let (test_label, test_enabled) = body.read(cx).test_button(cx);
            let on_enter = body.clone();
            dialog.title(title).child(body.clone()).on_ok(move |_, _, cx| on_enter.update(cx, |d, cx| d.save(cx))).footer(
                h_flex()
                    .gap(theme::space_8())
                    .child(
                        Button::new("dialog-test").debug_selector(|| "dialog-test".into()).small().label(test_label).disabled(!test_enabled).on_click({
                            let body = body.clone();
                            move |_, _, cx| body.update(cx, |d, cx| d.run_test(cx))
                        }),
                    )
                    .child(div().flex_1())
                    .child(Button::new("dialog-cancel").debug_selector(|| "dialog-cancel".into()).small().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
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
        view.read(cx).name.clone().update(cx, |name, cx| name.focus(window, cx));
        view
    }

    fn new(workspace: Entity<Workspace>, driver: Arc<dyn Driver>, editing: Option<ConnectionId>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let connections = &workspace.read(cx).connections;
        let form = match connections.iter().find(|c| Some(c.id) == editing) {
            Some(conn) => Form::of(conn),
            None => Form::new(&connections.iter().map(|c| c.color).collect::<Vec<_>>()),
        };
        let mut input = |value: &str, placeholder: &str| {
            let (value, placeholder) = (value.to_string(), placeholder.to_string());
            cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder))
        };
        let (name, host, port) = (input(&form.name, "e.g. prod-pg"), input(&form.host, "e.g. localhost"), input(&form.port, ""));
        let (user, database, path) = (input(&form.user, "e.g. postgres"), input(&form.database, ""), input(&form.path, "e.g. /home/me/app.db"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true).default_value(form.password.clone()));
        let selected = SSL_MODES.iter().position(|(mode, _)| *mode == form.ssl);
        let ssl = cx.new(|cx| SelectState::new(SSL_MODES.iter().map(|(_, label)| *label).collect(), selected.map(IndexPath::new), window, cx));
        for (id, input) in TEXT_FIELDS.into_iter().zip([&name, &host, &port, &user, &password, &database, &path]) {
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
        Self { workspace, driver, editing, color: form.color, is_production: form.is_production, kind: form.kind, name, host, port, user, password, database, path, ssl, touched: HashSet::new(), test: None, test_result: None }
    }

    /// The fields as the dialog shows them.
    pub fn form(&self, cx: &App) -> Form {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let ssl = self.ssl.read(cx).selected_value().and_then(|v| SSL_MODES.iter().find(|(_, label)| label == v)).map_or(SslMode::Prefer, |(mode, _)| *mode);
        Form {
            name: text(&self.name),
            color: self.color,
            is_production: self.is_production,
            kind: self.kind,
            host: text(&self.host),
            port: text(&self.port),
            user: text(&self.user),
            password: text(&self.password),
            database: text(&self.database),
            ssl,
            path: text(&self.path),
        }
    }

    /// Adds the connection to the workspace, or applies the edit, if the form is valid. Returns
    /// whether it did; if not, every field shows its error.
    pub fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let (Ok(name), Ok(params)) = self.validate(cx) else {
            self.touched.extend(TEXT_FIELDS);
            cx.notify();
            return false;
        };
        self.test = None;
        let (color, is_production, editing) = (self.color, self.is_production, self.editing);
        let id = self.workspace.update(cx, |ws, cx| {
            cx.notify();
            match editing {
                Some(id) => {
                    ws.edit(id, name, color, is_production, params);
                    id
                }
                None => ws.add(name, color, is_production, params),
            }
        });
        cx.emit(Saved(id));
        true
    }

    /// The trimmed name (unique among the other connections) and the trimmed params.
    fn validate(&self, cx: &App) -> (Result<String, String>, Result<ConnectionParams, FieldErrors>) {
        let form = self.form(cx);
        let others: Vec<&str> = self.workspace.read(cx).connections.iter().filter(|c| Some(c.id) != self.editing).map(|c| c.name.as_str()).collect();
        (validate_rename(&form.name, &others), form.params())
    }

    /// The inline errors on screen, as (field, message).
    pub fn errors(&self, cx: &App) -> Vec<(&'static str, String)> {
        let (name, params) = self.validate(cx);
        let fields = params.err().unwrap_or_default();
        [("name", name.err()), ("host", fields.host), ("port", fields.port), ("user", fields.user), ("database", fields.database), ("path", fields.path)]
            .into_iter()
            .filter(|(id, _)| self.touched.contains(id))
            .filter_map(|(id, error)| Some((id, error?)))
            .collect()
    }

    /// Whether the Add connection button is enabled.
    pub fn save_enabled(&self, cx: &App) -> bool {
        let (name, params) = self.validate(cx);
        button_state(name.is_ok(), params.is_ok()).0
    }

    /// The Test connection button: (label, enabled).
    pub fn test_button(&self, cx: &App) -> (&'static str, bool) {
        if self.test.is_some() {
            return ("Testing…", false);
        }
        let (name, params) = self.validate(cx);
        ("Test connection", button_state(name.is_ok(), params.is_ok()).1)
    }

    /// The test result line above the footer: (text, color).
    pub fn test_line(&self, cx: &App) -> Option<(String, Hsla)> {
        Some(match self.test_result.as_ref()? {
            Ok(()) => ("Connection OK".into(), cx.theme().success),
            Err(reason) => (format!("Could not connect: {reason}"), cx.theme().danger),
        })
    }

    /// A connection field changed, so the last test, or the one running, no longer applies.
    fn connection_changed(&mut self) {
        self.test = None;
        self.test_result = None;
    }

    /// Starts Test connection on the current fields; its outcome lands after `mock::latency()`.
    fn run_test(&mut self, cx: &mut Context<Self>) {
        let Ok(params) = self.form(cx).params() else { return };
        self.test_result = None;
        self.test = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(mock::latency()).await;
            this.update(cx, |this, cx| {
                this.test = None;
                this.test_result = Some(test_outcome(&params));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}

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
            cx.subscribe(&typed, |_, _, _: &InputEvent, cx| cx.notify()).detach();
            Self { name: conn.name.clone(), is_production: conn.is_production, typed }
        });
        let body = view.clone();
        let title = format!("Delete connection {}?", conn.name);
        window.open_dialog(cx, move |dialog, _, cx| {
            let title = title.clone();
            let enabled = body.read(cx).enabled(cx);
            let on_enter = body.clone();
            dialog.title(div().debug_selector({
                let title = title.clone();
                move || format!("delete-title {title}")
            }).child(title)).child(body.clone()).on_ok(move |_, _, cx| on_enter.update(cx, |d, cx| d.confirm(cx))).footer(
                h_flex()
                    .gap(theme::space_8())
                    .child(div().flex_1())
                    .child(Button::new("delete-cancel").debug_selector(|| "delete-cancel".into()).small().label("Cancel").on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("delete-confirm").debug_selector(|| "delete-confirm".into()).danger().small().label("Delete connection").disabled(!enabled).on_click({
                        let body = body.clone();
                        move |_, window, cx| {
                            if body.update(cx, |d, cx| d.confirm(cx)) {
                                window.close_dialog(cx);
                            }
                        }
                    })),
            )
        });
        if conn.is_production {
            view.read(cx).typed.clone().update(cx, |typed, cx| typed.focus(window, cx));
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
            .child(format!("{} is removed from the workspace and its tabs close. This can't be undone.", self.name));
        v_flex().gap(theme::space_12()).child(note).when(self.is_production, |d| {
            d.child(
                v_flex()
                    .debug_selector(|| "delete-name".into())
                    .gap(theme::space_4())
                    .child(div().text_size(theme::ui_sm()).text_color(theme::text_secondary(cx.theme())).child(format!("{} is a Production connection. Type its name to confirm.", self.name)))
                    .child(Input::new(&self.typed).small()),
            )
        })
    }
}

/// A labeled control with its entry in `errors` under it. `id` names it for tests (`field-<id>`).
fn field(id: &'static str, label: &'static str, control: impl IntoElement, errors: &[(&str, String)], cx: &App) -> gpui_kit::Stateful<gpui_kit::Div> {
    let error = errors.iter().find(|(field, _)| *field == id).map(|(_, msg)| msg.clone());
    v_flex()
        .id(id)
        .debug_selector(move || format!("field-{id}"))
        .flex_1()
        .gap(theme::space_4())
        .child(div().text_size(theme::ui_sm()).text_color(theme::text_secondary(cx.theme())).child(label))
        .child(control)
        .when_some(error, |d, msg| {
            d.child(div().debug_selector(move || format!("error-{id}")).text_size(theme::ui_sm()).text_color(cx.theme().danger).child(msg))
        })
}

impl Render for ConnectionDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let errors = self.errors(cx);
        let theme = cx.theme();
        let swatches = (0..theme::CONNECTION_COLORS.len() as u8).map(|ix| {
            let picked = ix == self.color;
            div()
                .id(("swatch", ix as usize))
                .debug_selector(move || format!("swatch-{ix}"))
                .size(theme::space_16())
                .rounded_full()
                .bg(theme::connection_color(ix, theme))
                .when(picked, |d| d.border_2().border_color(theme.foreground))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.color = ix;
                    cx.notify();
                }))
                .into_any_element()
        });
        let locked = self.editing.is_some();
        let kinds = ButtonGroup::new("kind")
            .small()
            .outline()
            // Edit mode locks the type by disabling only the other one, since a disabled button hides
            // its selected style; `children` because `child` overwrites each button's disabled state.
            .children([
                Button::new("kind-postgres").debug_selector(|| "kind-postgres".into()).label("Postgres").selected(self.kind == DbKind::Postgres).disabled(locked && self.kind != DbKind::Postgres),
                Button::new("kind-sqlite").debug_selector(|| "kind-sqlite".into()).label("SQLite").selected(self.kind == DbKind::Sqlite).disabled(locked && self.kind != DbKind::Sqlite),
            ])
            .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
                let kind = if clicked.first() == Some(&1) { DbKind::Sqlite } else { DbKind::Postgres };
                if kind != this.kind {
                    this.kind = kind;
                    this.connection_changed();
                }
                cx.notify();
            }));
        let fields: Vec<AnyElement> = match self.kind {
            DbKind::Postgres => vec![
                h_flex()
                    .items_start()
                    .gap(theme::space_8())
                    .child(field("host", "Host", Input::new(&self.host).small(), &errors, cx))
                    .child(field("port", "Port", Input::new(&self.port).small(), &errors, cx))
                    .into_any_element(),
                h_flex()
                    .items_start()
                    .gap(theme::space_8())
                    .child(field("user", "User", Input::new(&self.user).small(), &errors, cx))
                    .child(field("database", "Database", Input::new(&self.database).small(), &errors, cx))
                    .into_any_element(),
                field("password", "Password", Input::new(&self.password).small(), &errors, cx).into_any_element(),
                field("ssl", "SSL", Select::new(&self.ssl).small(), &errors, cx).into_any_element(),
            ],
            DbKind::Sqlite => vec![field("path", "File path", Input::new(&self.path).small(), &errors, cx).into_any_element()],
        };
        v_flex()
            .gap(theme::space_12())
            .child(field("kind", "Type", kinds, &errors, cx))
            .child(field("name", "Name", Input::new(&self.name).small(), &errors, cx))
            .child(field("color", "Color", h_flex().gap(theme::space_6()).children(swatches), &errors, cx))
            .child(
                div().self_start().debug_selector(|| "production".into()).child(
                    Switch::new("production")
                        .small()
                        .label("Production")
                        .checked(self.is_production)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.is_production = *checked;
                            cx.notify();
                        })),
                ),
            )
            .children(fields)
            .when_some(self.test_line(cx), |d, (line, color)| {
                d.child(div().debug_selector(|| "test-result".into()).text_size(theme::ui_sm()).text_color(color).child(line))
            })
    }
}

const PORT_ERROR: &str = "Port must be a number from 1 to 65535";

/// The dialog's connection fields as typed. Strings are untrimmed.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub name: String,
    pub color: u8,
    pub is_production: bool,
    pub kind: DbKind,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub database: String,
    pub ssl: SslMode,
    pub path: String,
}

/// Inline error per field; `None` means that field is valid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldErrors {
    pub host: Option<String>,
    pub port: Option<String>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub path: Option<String>,
}

impl Form {
    pub fn new(used_colors: &[u8]) -> Self {
        Form {
            name: String::new(),
            color: default_color(used_colors),
            is_production: false,
            kind: DbKind::Postgres,
            host: String::new(),
            port: "5432".into(),
            user: String::new(),
            password: String::new(),
            database: "postgres".into(),
            ssl: SslMode::Prefer,
            path: String::new(),
        }
    }

    /// The form for editing `conn`. Fields its type doesn't use keep the new-connection defaults.
    pub fn of(conn: &Connection) -> Self {
        let base = Form { name: conn.name.clone(), color: conn.color, is_production: conn.is_production, kind: conn.kind(), ..Form::new(&[]) };
        match &conn.params {
            ConnectionParams::Postgres { host, port, user, password, database, ssl } => {
                Form { host: host.clone(), port: port.to_string(), user: user.clone(), password: password.as_ref().map_or(String::new(), |p| p.expose().to_string()), database: database.clone(), ssl: *ssl, ..base }
            }
            ConnectionParams::Sqlite { path } => Form { path: path.display().to_string(), ..base },
        }
    }

    /// The trimmed params, or the error under each invalid field.
    pub fn params(&self) -> Result<ConnectionParams, FieldErrors> {
        let required = |value: &str, error: &str| {
            let value = value.trim();
            if value.is_empty() { Err(error.to_string()) } else { Ok(value.to_string()) }
        };
        if self.kind == DbKind::Sqlite {
            return match required(&self.path, "Path can't be empty") {
                Ok(path) => Ok(ConnectionParams::Sqlite { path: PathBuf::from(path) }),
                Err(path) => Err(FieldErrors { path: Some(path), ..Default::default() }),
            };
        }
        let host = required(&self.host, "Host can't be empty");
        let port = match self.port.trim().parse::<u16>() {
            Ok(port @ 1..) if self.port.trim().bytes().all(|b| b.is_ascii_digit()) => Ok(port),
            _ => Err(PORT_ERROR.to_string()),
        };
        let user = required(&self.user, "User can't be empty");
        let database = required(&self.database, "Database can't be empty");
        match (host, port, user, database) {
            (Ok(host), Ok(port), Ok(user), Ok(database)) => {
                Ok(ConnectionParams::Postgres { host, port, user, password: (!self.password.is_empty()).then(|| Secret::new(&self.password)), database, ssl: self.ssl })
            }
            (host, port, user, database) => Err(FieldErrors {
                host: host.err(),
                port: port.err(),
                user: user.err(),
                database: database.err(),
                path: None,
            }),
        }
    }
}

/// Lowest swatch index (0-7) no connection uses, or 0 when all are used.
pub fn default_color(used: &[u8]) -> u8 {
    (0..8).find(|c| !used.contains(c)).unwrap_or(0)
}

/// (Save enabled, Test connection enabled). Test ignores the name.
pub fn button_state(name_valid: bool, fields_valid: bool) -> (bool, bool) {
    (name_valid && fields_valid, fields_valid)
}

/// Whether the confirmation's Delete button is enabled. A Production connection needs its name
/// typed exactly (no trimming, case-sensitive).
pub fn delete_enabled(name: &str, is_production: bool, typed: &str) -> bool {
    !is_production || typed == name
}

/// The Test connection result for valid params: `Ok` or the failure reason.
pub fn test_outcome(params: &ConnectionParams) -> Result<(), String> {
    match params {
        ConnectionParams::Postgres { host, .. } => mock::test_connection(host),
        ConnectionParams::Sqlite { path } if path.is_file() => Ok(()),
        ConnectionParams::Sqlite { path } => Err(format!("No file at {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(host: &str, port: &str, user: &str, database: &str) -> Form {
        Form {
            host: host.into(),
            port: port.into(),
            user: user.into(),
            database: database.into(),
            ..Form::new(&[])
        }
    }

    fn ok_pg() -> Form {
        pg("h", "5432", "u", "d")
    }

    #[test]
    fn field_validation_cases() {
        let port_err = Some(PORT_ERROR.to_string());
        let cases: Vec<(Form, FieldErrors)> = vec![
            (pg("", "5432", "u", "d"), FieldErrors { host: Some("Host can't be empty".into()), ..Default::default() }),
            (pg("  ", "5432", "u", "d"), FieldErrors { host: Some("Host can't be empty".into()), ..Default::default() }),
            (pg("h", "5432", "", "d"), FieldErrors { user: Some("User can't be empty".into()), ..Default::default() }),
            (pg("h", "5432", "  ", "d"), FieldErrors { user: Some("User can't be empty".into()), ..Default::default() }),
            (pg("h", "5432", "u", ""), FieldErrors { database: Some("Database can't be empty".into()), ..Default::default() }),
            (pg("h", "5432", "u", " "), FieldErrors { database: Some("Database can't be empty".into()), ..Default::default() }),
            (pg("h", "0", "u", "d"), FieldErrors { port: port_err.clone(), ..Default::default() }),
            (pg("h", "65536", "u", "d"), FieldErrors { port: port_err.clone(), ..Default::default() }),
            (pg("h", "-1", "u", "d"), FieldErrors { port: port_err.clone(), ..Default::default() }),
            (pg("h", "", "u", "d"), FieldErrors { port: port_err.clone(), ..Default::default() }),
            (pg("h", "54a", "u", "d"), FieldErrors { port: port_err.clone(), ..Default::default() }),
            (pg("h", "5432.0", "u", "d"), FieldErrors { port: port_err.clone(), ..Default::default() }),
            (pg("h", "+5432", "u", "d"), FieldErrors { port: port_err.clone(), ..Default::default() }),
            (pg("", "x", "", ""), FieldErrors {
                host: Some("Host can't be empty".into()),
                port: port_err.clone(),
                user: Some("User can't be empty".into()),
                database: Some("Database can't be empty".into()),
                path: None,
            }),
            (Form { kind: DbKind::Sqlite, ..Form::new(&[]) }, FieldErrors { path: Some("Path can't be empty".into()), ..Default::default() }),
            (Form { kind: DbKind::Sqlite, path: "  ".into(), ..Form::new(&[]) }, FieldErrors { path: Some("Path can't be empty".into()), ..Default::default() }),
        ];
        for (form, want) in cases {
            assert_eq!(form.params(), Err(want), "{form:?}");
        }
    }

    #[test]
    fn field_validation_ok_cases() {
        let postgres = |host: &str, port: u16| ConnectionParams::Postgres {
            host: host.into(),
            port,
            user: "u".into(),
            password: None,
            database: "d".into(),
            ssl: SslMode::Prefer,
        };
        let cases = [
            (pg(" db.local ", "5432", " u ", " d "), postgres("db.local", 5432)),
            (pg("h", " 5432 ", "u", "d"), postgres("h", 5432)),
            (pg("h", "1", "u", "d"), postgres("h", 1)),
            (pg("h", "65535", "u", "d"), postgres("h", 65535)),
            (
                Form { kind: DbKind::Sqlite, path: " /nope/x.db ".into(), host: "".into(), port: "bad".into(), ..Form::new(&[]) },
                ConnectionParams::Sqlite { path: PathBuf::from("/nope/x.db") },
            ),
        ];
        for (form, want) in cases {
            assert_eq!(form.params(), Ok(want), "{form:?}");
        }
        let ssl = Form { ssl: SslMode::Require, ..ok_pg() };
        assert!(matches!(ssl.params(), Ok(ConnectionParams::Postgres { ssl: SslMode::Require, .. })));
    }

    #[test]
    fn the_password_reaches_the_params_as_a_secret_untrimmed_and_none_when_empty() {
        let params = |password: &str, kind| Form { password: password.into(), kind, path: "/x.db".into(), ..ok_pg() }.params();
        let cases = [("", None), ("hunter2", Some("hunter2")), (" pad ", Some(" pad ")), ("p@ss w0rd/\\", Some("p@ss w0rd/\\"))];
        for (typed, want) in cases {
            let Ok(ConnectionParams::Postgres { password, .. }) = params(typed, DbKind::Postgres) else { panic!("{typed:?}: valid") };
            assert_eq!(password.as_ref().map(Secret::expose), want, "{typed:?}");
        }
        assert_eq!(params("hunter2", DbKind::Sqlite), Ok(ConnectionParams::Sqlite { path: PathBuf::from("/x.db") }), "SQLite ignores it");
    }

    #[test]
    fn button_state_cases() {
        let cases = [
            (false, true, (false, true)),
            (true, false, (false, false)),
            (true, true, (true, true)),
            (false, false, (false, false)),
        ];
        for (name_valid, fields_valid, want) in cases {
            assert_eq!(button_state(name_valid, fields_valid), want);
        }
    }

    #[test]
    fn default_color_cases() {
        let cases: [(&[u8], u8); 8] = [
            (&[], 0),
            (&[1], 0),
            (&[0, 1], 2),
            (&[0, 2], 1),
            (&[0, 1, 2, 3, 4, 5, 6], 7),
            (&[0, 1, 2, 3, 4, 5, 6, 7], 0),
            (&[7, 6, 5, 4, 3, 2, 1], 0),
            (&[0, 0, 1, 1], 2),
        ];
        for (used, want) in cases {
            assert_eq!(default_color(used), want, "{used:?}");
        }
    }

    #[test]
    fn new_form_defaults() {
        let form = Form::new(&[0, 1]);
        assert_eq!(
            form,
            Form {
                name: "".into(),
                color: 2,
                is_production: false,
                kind: DbKind::Postgres,
                host: "".into(),
                port: "5432".into(),
                user: "".into(),
                password: "".into(),
                database: "postgres".into(),
                ssl: SslMode::Prefer,
                path: "".into(),
            }
        );
    }

    #[test]
    fn delete_enabled_cases() {
        let cases = [
            ("local.db", false, "", true),
            ("local.db", false, "anything", true),
            ("prod-pg", true, "", false),
            ("prod-pg", true, "prod-p", false),
            ("prod-pg", true, "Prod-pg", false),
            ("prod-pg", true, " prod-pg", false),
            ("prod-pg", true, "prod-pg ", false),
            ("prod-pg", true, "prod-pgx", false),
            ("prod-pg", true, "prod-pg", true),
        ];
        for (name, prod, typed, want) in cases {
            assert_eq!(delete_enabled(name, prod, typed), want, "{name} {prod} {typed:?}");
        }
    }

    #[test]
    fn test_outcome_cases() {
        let dir = std::env::temp_dir();
        let file = std::env::current_exe().unwrap();
        let postgres = |host: &str| ConnectionParams::Postgres {
            host: host.into(),
            port: 5432,
            user: "u".into(),
            password: None,
            database: "d".into(),
            ssl: SslMode::Prefer,
        };
        let sqlite = |path: &std::path::Path| ConnectionParams::Sqlite { path: path.to_path_buf() };
        let no_file = |path: &std::path::Path| Err(format!("No file at {}", path.display()));
        let missing = PathBuf::from("/nope/x.db");
        let cases = [
            (postgres("db.local"), Ok(())),
            (postgres("db.invalid"), Err("Could not resolve host db.invalid".to_string())),
            (postgres("DB.INVALID"), Err("Could not resolve host DB.INVALID".to_string())),
            (postgres("invalid.db"), Ok(())),
            (postgres("invalid"), Ok(())),
            (sqlite(&file), Ok(())),
            (sqlite(&missing), no_file(&missing)),
            (sqlite(&dir), no_file(&dir)),
        ];
        for (params, want) in cases {
            assert_eq!(test_outcome(&params), want, "{params:?}");
        }
    }
}
