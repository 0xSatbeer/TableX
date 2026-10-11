//! The add/edit connection dialog, and the pure rules behind it: validation, defaults, change
//! classification, delete confirmation and the Test connection outcome.
#![allow(dead_code)] // shortcut: wired up by the dialog view tickets (T03-T06), drop then

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonGroup},
    h_flex,
    input::{Input, InputState},
    select::{Select, SelectState},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, StatefulInteractiveElement as _, ParentElement as _, Render, Styled as _, Window, div};

use crate::model::*;
use crate::{mock, theme};

/// SSL choices in the order the select lists them.
const SSL_MODES: [(SslMode, &str); 3] = [(SslMode::Disable, "Disable"), (SslMode::Prefer, "Prefer"), (SslMode::Require, "Require")];

/// The Add connection dialog's body. The dialog itself lives in the window's dialog layer.
pub struct ConnectionDialog {
    workspace: Entity<Workspace>,
    color: u8,
    is_production: bool,
    kind: DbKind,
    pub name: Entity<InputState>,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub user: Entity<InputState>,
    pub database: Entity<InputState>,
    pub path: Entity<InputState>,
    ssl: Entity<SelectState<Vec<&'static str>>>,
}

impl ConnectionDialog {
    /// Opens the Add connection dialog over `window`, with the name field focused.
    pub fn open(workspace: Entity<Workspace>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(workspace, window, cx));
        let body = view.clone();
        window.open_dialog(cx, move |dialog, _, _| dialog.title("Add connection").child(body.clone()));
        view.read(cx).name.clone().update(cx, |name, cx| name.focus(window, cx));
        view
    }

    fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let used: Vec<u8> = workspace.read(cx).connections.iter().map(|c| c.color).collect();
        let form = Form::new(&used);
        let mut input = |value: &str, placeholder: &str| {
            let (value, placeholder) = (value.to_string(), placeholder.to_string());
            cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder))
        };
        let (name, host, port) = (input(&form.name, "e.g. prod-pg"), input(&form.host, "localhost"), input(&form.port, ""));
        let (user, database, path) = (input(&form.user, "postgres"), input(&form.database, ""), input(&form.path, "/path/to/file.db"));
        let selected = SSL_MODES.iter().position(|(mode, _)| *mode == form.ssl);
        let ssl = cx.new(|cx| SelectState::new(SSL_MODES.iter().map(|(_, label)| *label).collect(), selected.map(IndexPath::new), window, cx));
        Self { workspace, color: form.color, is_production: form.is_production, kind: form.kind, name, host, port, user, database, path, ssl }
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
            database: text(&self.database),
            ssl,
            path: text(&self.path),
        }
    }
}

/// A labeled control. `id` names it for tests (`field-<id>`).
fn field(id: &'static str, label: &'static str, control: impl IntoElement, cx: &App) -> gpui_kit::Stateful<gpui_kit::Div> {
    v_flex()
        .id(id)
        .debug_selector(move || format!("field-{id}"))
        .flex_1()
        .gap(theme::space_4())
        .child(div().text_size(theme::ui_sm()).text_color(theme::text_secondary(cx.theme())).child(label))
        .child(control)
}

impl Render for ConnectionDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let swatches = (0..theme::CONNECTION_COLORS.len() as u8).map(|ix| {
            let picked = ix == self.color;
            div()
                .id(("swatch", ix as usize))
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
        let kinds = ButtonGroup::new("kind")
            .small()
            .outline()
            .child(Button::new("kind-postgres").debug_selector(|| "kind-postgres".into()).label("Postgres").selected(self.kind == DbKind::Postgres))
            .child(Button::new("kind-sqlite").debug_selector(|| "kind-sqlite".into()).label("SQLite").selected(self.kind == DbKind::Sqlite));
        let fields: Vec<AnyElement> = match self.kind {
            DbKind::Postgres => vec![
                h_flex()
                    .gap(theme::space_8())
                    .child(field("host", "Host", Input::new(&self.host).small(), cx))
                    .child(field("port", "Port", Input::new(&self.port).small(), cx))
                    .into_any_element(),
                h_flex()
                    .gap(theme::space_8())
                    .child(field("user", "User", Input::new(&self.user).small(), cx))
                    .child(field("database", "Database", Input::new(&self.database).small(), cx))
                    .into_any_element(),
                field("ssl", "SSL", Select::new(&self.ssl).small(), cx).into_any_element(),
            ],
            DbKind::Sqlite => vec![field("path", "File path", Input::new(&self.path).small(), cx).into_any_element()],
        };
        v_flex()
            .gap(theme::space_12())
            .child(field("kind", "Type", kinds, cx))
            .child(field("name", "Name", Input::new(&self.name).small(), cx))
            .child(field("color", "Color", h_flex().gap(theme::space_6()).children(swatches), cx))
            .child(
                Switch::new("production")
                    .small()
                    .label("Production")
                    .checked(self.is_production)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.is_production = *checked;
                        cx.notify();
                    })),
            )
            .children(fields)
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
            database: "postgres".into(),
            ssl: SslMode::Prefer,
            path: String::new(),
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
                Ok(ConnectionParams::Postgres { host, port, user, database, ssl: self.ssl })
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

/// Whether saving `form` over a connection with params `old` changes them (compared trimmed).
/// An invalid form counts as changed; it can't be saved anyway.
pub fn params_changed(old: &ConnectionParams, form: &Form) -> bool {
    form.params().as_ref() != Ok(old)
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
                database: "postgres".into(),
                ssl: SslMode::Prefer,
                path: "".into(),
            }
        );
    }

    #[test]
    fn params_changed_cases() {
        let old = ConnectionParams::Postgres {
            host: "db.local".into(),
            port: 5432,
            user: "u".into(),
            database: "d".into(),
            ssl: SslMode::Prefer,
        };
        let base = Form { host: "db.local".into(), ..ok_pg() };
        let base = Form { user: "u".into(), database: "d".into(), ..base };
        let sqlite_old = ConnectionParams::Sqlite { path: PathBuf::from("/a.db") };
        let sqlite = |path: &str| Form { kind: DbKind::Sqlite, path: path.into(), ..Form::new(&[]) };
        let cases = [
            (&old, base.clone(), false),
            (&old, Form { name: "new".into(), ..base.clone() }, false),
            (&old, Form { color: 5, ..base.clone() }, false),
            (&old, Form { is_production: true, ..base.clone() }, false),
            (&old, Form { host: " db.local ".into(), ..base.clone() }, false),
            (&old, Form { port: " 5432 ".into(), ..base.clone() }, false),
            (&old, Form { port: "5433".into(), ..base.clone() }, true),
            (&old, Form { ssl: SslMode::Require, ..base.clone() }, true),
            (&old, Form { host: "other".into(), ..base.clone() }, true),
            (&old, Form { user: "v".into(), ..base.clone() }, true),
            (&old, Form { database: "e".into(), ..base.clone() }, true),
            (&old, Form { host: "".into(), ..base.clone() }, true),
            (&sqlite_old, sqlite("/a.db"), false),
            (&sqlite_old, sqlite(" /a.db "), false),
            (&sqlite_old, sqlite("/b.db"), true),
        ];
        for (old, form, want) in cases {
            assert_eq!(params_changed(old, &form), want, "{form:?}");
        }
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
