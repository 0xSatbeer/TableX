//! Domain types the UI binds to. Filled by `mock` today and by drivers in Phase 5.
//! No UI imports here.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: u32,
    pub name: String,
    /// A connection belongs to exactly one workspace, so the workspace owns it.
    pub connections: Vec<Connection>,
}

impl Workspace {
    /// Appends a new, disconnected connection with an id no current connection has.
    pub fn add(
        &mut self,
        name: String,
        color: u8,
        is_production: bool,
        params: ConnectionParams,
    ) -> ConnectionId {
        let id = ConnectionId(self.connections.iter().map(|c| c.id.0).max().unwrap_or(0) + 1);
        self.connections.push(Connection {
            id,
            name,
            color,
            is_production,
            status: ConnectionStatus::Disconnected,
            params,
            databases: None,
        });
        id
    }

    /// Removes connection `id` and returns the position it had, or `None` if it is gone.
    pub fn delete(&mut self, id: ConnectionId) -> Option<usize> {
        let ix = self.connections.iter().position(|c| c.id == id)?;
        self.connections.remove(ix);
        Some(ix)
    }

    /// Replaces connection `id`'s name, color, Production flag and params in place. New params
    /// disconnect it and clear its databases, which came from the old server. Does nothing if it
    /// is gone.
    pub fn edit(
        &mut self,
        id: ConnectionId,
        name: String,
        color: u8,
        is_production: bool,
        params: ConnectionParams,
    ) {
        let Some(conn) = self.connections.iter_mut().find(|c| c.id == id) else {
            return;
        };
        if conn.params != params {
            (conn.status, conn.databases) = (ConnectionStatus::Disconnected, None);
        }
        (conn.name, conn.color, conn.is_production, conn.params) =
            (name, color, is_production, params);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbKind {
    Postgres,
    Sqlite,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
}

/// A password. In memory only, never written anywhere (QX-035 moves it to the keyring).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionParams {
    Postgres {
        host: String,
        port: u16,
        user: String,
        password: Option<Secret>,
        database: String,
        ssl: SslMode,
    },
    Sqlite {
        path: PathBuf,
    },
}

impl ConnectionParams {
    /// The database a connect opens: Postgres's from the dialog, SQLite's file name.
    pub fn database(&self) -> String {
        match self {
            Self::Postgres { database, .. } => database.clone(),
            Self::Sqlite { path } => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub id: ConnectionId,
    pub name: String,
    /// Index into the connection color palette (QX-002).
    pub color: u8,
    pub is_production: bool,
    pub status: ConnectionStatus,
    pub params: ConnectionParams,
    /// `None` until listed; each tree level below is loaded the same way, when first expanded.
    pub databases: Option<Vec<Database>>,
}

impl Connection {
    pub fn kind(&self) -> DbKind {
        match self.params {
            ConnectionParams::Postgres { .. } => DbKind::Postgres,
            ConnectionParams::Sqlite { .. } => DbKind::Sqlite,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Database {
    pub name: String,
    /// SQLite has one implicit `main` schema.
    pub schemas: Option<Vec<Schema>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub name: String,
    pub objects: Option<Vec<DbObject>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Sequence,
    Function,
    Procedure,
    Type,
    Index,
    Trigger,
}

impl ObjectKind {
    pub const ALL: [Self; 9] = [
        Self::Table,
        Self::View,
        Self::MaterializedView,
        Self::Sequence,
        Self::Function,
        Self::Procedure,
        Self::Type,
        Self::Index,
        Self::Trigger,
    ];

    /// Stable lowercase name, also the icon file stem.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::View => "view",
            Self::MaterializedView => "matview",
            Self::Sequence => "sequence",
            Self::Function => "function",
            Self::Procedure => "procedure",
            Self::Type => "type",
            Self::Index => "index",
            Self::Trigger => "trigger",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DbObject {
    pub kind: ObjectKind,
    pub name: String,
    /// Tables and views, `None` until loaded; empty for other kinds.
    pub columns: Option<Vec<Column>>,
    pub keys: Vec<Key>,
    pub indexes: Vec<Index>,
    /// Tables only; views and other kinds have none.
    pub row_estimate: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub is_pk: bool,
    /// The labels of an enum column, in declared order; empty for every other type.
    pub enum_values: Vec<String>,
    /// GENERATED … AS IDENTITY; serial columns stay false (their default shows nextval).
    pub identity: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Role {
    pub name: String,
    pub can_login: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    /// SQL keyword: "NO ACTION" | "RESTRICT" | "CASCADE" | "SET NULL" | "SET DEFAULT"
    pub on_delete: String,
    pub on_update: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub name: String,
    /// "BEFORE" | "AFTER" | "INSTEAD OF"
    pub timing: String,
    /// e.g. "INSERT OR UPDATE"
    pub events: String,
    /// The full CREATE TRIGGER statement as the server gives it.
    pub definition: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableDef {
    pub name: String,
    pub columns: Vec<Column>, // existing Column; is_pk editable, enum_values ignored
    pub indexes: Vec<Index>,  // existing Index; excludes PK/constraint-backing indexes
    pub foreign_keys: Vec<ForeignKey>,
    /// The primary key constraint's name, `None` when the table has none.
    pub primary_key: Option<String>,
    /// UNIQUE constraints (existing `Index`, `unique` always true), by constraint name.
    pub unique_keys: Vec<Index>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Primary {
        name: String,
        columns: Vec<String>,
    },
    Foreign {
        name: String,
        columns: Vec<String>,
        ref_table: String,
        ref_columns: Vec<String>,
    },
    #[allow(dead_code)] // introspection does not read unique constraints yet
    Unique {
        name: String,
        columns: Vec<String>,
    },
}

mod results;

pub use results::*;

#[cfg(test)]
mod tests;
