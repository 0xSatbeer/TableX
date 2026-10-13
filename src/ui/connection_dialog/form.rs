use super::*;

pub(super) const PORT_ERROR: &str = "Port must be a number from 1 to 65535";

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
        let base = Form {
            name: conn.name.clone(),
            color: conn.color,
            is_production: conn.is_production,
            kind: conn.kind(),
            ..Form::new(&[])
        };
        match &conn.params {
            ConnectionParams::Postgres {
                host,
                port,
                user,
                password,
                database,
                ssl,
            } => Form {
                host: host.clone(),
                port: port.to_string(),
                user: user.clone(),
                password: password
                    .as_ref()
                    .map_or(String::new(), |p| p.expose().to_string()),
                database: database.clone(),
                ssl: *ssl,
                ..base
            },
            ConnectionParams::Sqlite { path } => Form {
                path: path.display().to_string(),
                ..base
            },
        }
    }

    /// The trimmed params, or the error under each invalid field.
    pub fn params(&self) -> Result<ConnectionParams, FieldErrors> {
        let required = |value: &str, error: &str| {
            let value = value.trim();
            if value.is_empty() {
                Err(error.to_string())
            } else {
                Ok(value.to_string())
            }
        };
        if self.kind == DbKind::Sqlite {
            return match required(&self.path, "Path can't be empty") {
                Ok(path) => Ok(ConnectionParams::Sqlite {
                    path: PathBuf::from(path),
                }),
                Err(path) => Err(FieldErrors {
                    path: Some(path),
                    ..Default::default()
                }),
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
            (Ok(host), Ok(port), Ok(user), Ok(database)) => Ok(ConnectionParams::Postgres {
                host,
                port,
                user,
                password: (!self.password.is_empty()).then(|| Secret::new(&self.password)),
                database,
                ssl: self.ssl,
            }),
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
