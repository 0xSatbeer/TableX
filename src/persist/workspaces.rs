//! `workspaces.toml` (C6): every workspace and its connections, never a password.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{
    Connection, ConnectionId, ConnectionParams, ConnectionStatus, SslMode, Workspace, Workspaces,
};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct FileToml {
    version: u32,
    active: u32,
    #[serde(default)]
    workspace: Vec<WorkspaceToml>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct WorkspaceToml {
    id: u32,
    name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    connection: Vec<ConnectionToml>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ConnectionToml {
    id: u32,
    name: String,
    color: u8,
    production: bool,
    #[serde(flatten)]
    params: ParamsToml,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ParamsToml {
    Postgres {
        host: String,
        port: u16,
        user: String,
        database: String,
        ssl: SslToml,
        save_password: bool,
    },
    Sqlite {
        path: PathBuf,
    },
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SslToml {
    Disable,
    Prefer,
    Require,
}

/// `all` as `workspaces.toml` text.
pub fn workspaces_to_toml(all: &Workspaces) -> String {
    let file = FileToml {
        version: 1,
        active: all.active,
        workspace: all.list.iter().map(workspace_to_toml).collect(),
    };
    toml::to_string(&file).expect("workspaces serialize")
}

/// The workspaces in `text`, disconnected, with ids shared across them.
pub fn workspaces_from_toml(text: &str) -> Result<Workspaces, toml::de::Error> {
    let file: FileToml = toml::from_str(text)?;
    let mut all = Workspaces {
        active: file.active,
        list: file
            .workspace
            .into_iter()
            .map(workspace_from_toml)
            .collect(),
    };
    all.share_ids();
    Ok(all)
}

fn workspace_to_toml(w: &Workspace) -> WorkspaceToml {
    WorkspaceToml {
        id: w.id,
        name: w.name.clone(),
        connection: w
            .connections
            .iter()
            .map(|c| ConnectionToml {
                id: c.id.0,
                name: c.name.clone(),
                color: c.color,
                production: c.is_production,
                params: match &c.params {
                    ConnectionParams::Postgres {
                        host,
                        port,
                        user,
                        database,
                        ssl,
                        password: _,
                    } => ParamsToml::Postgres {
                        host: host.clone(),
                        port: *port,
                        user: user.clone(),
                        database: database.clone(),
                        ssl: match ssl {
                            SslMode::Disable => SslToml::Disable,
                            SslMode::Prefer => SslToml::Prefer,
                            SslMode::Require => SslToml::Require,
                        },
                        save_password: c.save_password,
                    },
                    ConnectionParams::Sqlite { path } => ParamsToml::Sqlite { path: path.clone() },
                },
            })
            .collect(),
    }
}

fn workspace_from_toml(w: WorkspaceToml) -> Workspace {
    Workspace {
        id: w.id,
        name: w.name,
        connections: w
            .connection
            .into_iter()
            .map(|c| {
                let (params, save_password) = match c.params {
                    ParamsToml::Postgres {
                        host,
                        port,
                        user,
                        database,
                        ssl,
                        save_password,
                    } => (
                        ConnectionParams::Postgres {
                            host,
                            port,
                            user,
                            password: None,
                            database,
                            ssl: match ssl {
                                SslToml::Disable => SslMode::Disable,
                                SslToml::Prefer => SslMode::Prefer,
                                SslToml::Require => SslMode::Require,
                            },
                        },
                        save_password,
                    ),
                    ParamsToml::Sqlite { path } => (ConnectionParams::Sqlite { path }, false),
                };
                Connection {
                    id: ConnectionId(c.id),
                    name: c.name,
                    color: c.color,
                    is_production: c.production,
                    status: ConnectionStatus::Disconnected,
                    params,
                    save_password,
                    databases: None,
                }
            })
            .collect(),
        id_floor: 0,
    }
}

/// `workspaces.toml` and what was last read from or written to it.
pub struct WorkspacesFile {
    pub path: PathBuf,
    pub all: Workspaces,
    saved: String,
}

impl WorkspacesFile {
    /// Loads `<root>/workspaces.toml`; empty when absent. A corrupt file is moved to `.bak` and
    /// the notice says so.
    pub fn load(root: &Path) -> (Self, Option<String>) {
        let path = root.join("workspaces.toml");
        let (all, notice) = match super::read_or_backup(&path, workspaces_from_toml) {
            Ok(all) => (all, None),
            Err(notice) => (None, Some(notice)),
        };
        let all = all.unwrap_or(Workspaces {
            active: 1,
            list: Vec::new(),
        });
        let saved = workspaces_to_toml(&all);
        (Self { path, all, saved }, notice)
    }

    /// The active workspace, or a new empty one added as active when there is none.
    pub fn active(&mut self) -> Workspace {
        if let Some(w) = self.all.list.iter().find(|w| w.id == self.all.active) {
            return w.clone();
        }
        let w = self.all.list.first().cloned().unwrap_or(Workspace {
            id: 1,
            name: "Workspace".into(),
            connections: Vec::new(),
            id_floor: 0,
        });
        if self.all.list.is_empty() {
            self.all.list.push(w.clone());
        }
        self.all.active = w.id;
        self.all.share_ids();
        self.all.list[0].clone()
    }

    /// Writes the file with `active` as it now is, when that changes what the file holds.
    pub fn save(&mut self, active: &Workspace) -> std::io::Result<()> {
        if let Some(slot) = self.all.list.iter_mut().find(|w| w.id == active.id) {
            *slot = active.clone();
        }
        let text = workspaces_to_toml(&self.all);
        if text == self.saved {
            return Ok(());
        }
        super::write_atomic(&self.path, text.as_bytes())?;
        self.saved = text;
        Ok(())
    }
}
