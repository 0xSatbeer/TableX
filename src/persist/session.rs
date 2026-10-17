//! `sessions/<workspace id>.toml` (C8): a workspace's tabs, layout and expanded tree.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionFile {
    pub version: u32,
    /// Index into `tabs`; absent when there are none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tab: Option<usize>,
    pub explorer_visible: bool,
    /// Logical px.
    pub explorer_width: f32,
    pub panel_visible: bool,
    pub panel_height: f32,
    /// Expanded tree nodes by name path: connection id, then names.
    pub expanded: Vec<String>,
    #[serde(rename = "tab")]
    pub tabs: Vec<SessionTab>,
}

impl Default for SessionFile {
    fn default() -> Self {
        Self {
            version: 1,
            active_tab: None,
            explorer_visible: true,
            explorer_width: 260.0,
            panel_visible: true,
            panel_height: 240.0,
            expanded: Vec::new(),
            tabs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SessionTab {
    Table {
        connection: u32,
        database: String,
        schema: String,
        object: String,
        #[serde(rename = "where")]
        where_: String,
        order_by: String,
        pinned: bool,
    },
    Console {
        connection: u32,
        database: String,
        /// Empty when the console has no schema picked.
        schema: String,
        /// `consoles/<console>.sql`
        console: u32,
        autocommit: bool,
        pinned: bool,
    },
}

pub fn session_path(root: &Path, workspace: u32) -> PathBuf {
    root.join("sessions").join(format!("{workspace}.toml"))
}

/// Workspace `workspace`'s session; a missing file is the default (no tabs). A corrupt one is
/// moved to `.bak` and the notice says so.
pub fn load_session(root: &Path, workspace: u32) -> (SessionFile, Option<String>) {
    match super::read_or_backup(&session_path(root, workspace), |t| {
        toml::from_str::<SessionFile>(t)
    }) {
        Ok(session) => (session.unwrap_or_default(), None),
        Err(notice) => (SessionFile::default(), Some(notice)),
    }
}

pub fn save_session(root: &Path, workspace: u32, session: &SessionFile) -> std::io::Result<()> {
    let text = toml::to_string(session).expect("session serializes");
    super::write_atomic(&session_path(root, workspace), text.as_bytes())
}

/// The largest console id any saved session or `consoles/<id>.sql` file uses, so new consoles
/// never reuse another's file.
pub fn max_console_id(root: &Path) -> u32 {
    let files = |dir: &str| {
        std::fs::read_dir(root.join(dir))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
    };
    let in_sessions = files("sessions")
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|text| toml::from_str::<SessionFile>(&text).ok())
        .flat_map(|s| s.tabs)
        .filter_map(|t| match t {
            SessionTab::Console { console, .. } => Some(console),
            SessionTab::Table { .. } => None,
        });
    let on_disk = files("consoles")
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .filter_map(|p| p.file_stem()?.to_str()?.parse().ok());
    in_sessions.chain(on_disk).max().unwrap_or(0)
}
