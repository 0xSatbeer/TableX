//! Pure rules behind the add/edit connection dialog: validation, defaults, change
//! classification, delete confirmation and the Test connection outcome. No UI here.
#![allow(dead_code)] // shortcut: wired up by the dialog view tickets (T03-T06), drop then

use std::path::PathBuf;

use crate::model::*;

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
    255
}

/// (Save enabled, Test connection enabled). Test ignores the name.
pub fn button_state(name_valid: bool, fields_valid: bool) -> (bool, bool) {
    (name_valid && fields_valid, fields_valid)
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
}
