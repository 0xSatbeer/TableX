//! Config files under the `ConfigDir` (C5): atomic writes and `.bak` recovery.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

mod config;
mod keychain;
mod workspaces;

pub use config::*;
pub use keychain::*;
pub use workspaces::*;

/// `queryx/` under the platform config dir; every config file lives below it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigDir {
    pub root: PathBuf,
}

impl ConfigDir {
    /// `env` reads an environment variable.
    pub fn resolve(env: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        // Empty or relative values count as unset, as the XDG spec says.
        let abs = |k: &str| env(k).map(PathBuf::from).filter(|p| p.is_absolute());
        let config = if cfg!(target_os = "macos") {
            abs("HOME").map(|h| h.join("Library/Application Support"))
        } else if cfg!(windows) {
            abs("APPDATA")
        } else {
            abs("XDG_CONFIG_HOME").or_else(|| abs("HOME").map(|h| h.join(".config")))
        };
        config.map(|c| Self {
            root: c.join("queryx"),
        })
    }
}

/// Writes `<path>.tmp`, then renames it over `path`, so `path` is never left half written.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(path, ".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// `path` parsed by `parse`; `Ok(None)` when it doesn't exist. A file that can't be read or
/// parsed is renamed `<path>.bak` and the error is the notice to show.
pub fn read_or_backup<T, E>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<T, E>,
) -> Result<Option<T>, String> {
    let parsed = match std::fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        read => read.ok().and_then(|text| parse(&text).ok()),
    };
    if parsed.is_some() {
        return Ok(parsed);
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match std::fs::rename(path, with_suffix(path, ".bak")) {
        Ok(()) => Err(format!(
            "{name} could not be read; it was saved as {name}.bak and QueryX started empty."
        )),
        Err(e) => Err(format!(
            "{name} could not be read, nor saved as {name}.bak ({e}); QueryX started empty."
        )),
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

#[cfg(test)]
mod tests;
