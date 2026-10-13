//! Inline rename of a connection row.

use super::*;

/// The trimmed name to commit, or the message to show inline. `others` are the other connections'
/// names; uniqueness is case-sensitive.
pub(crate) fn validate_rename(input: &str, others: &[&str]) -> Result<String, String> {
    let name = input.trim();
    if name.is_empty() {
        Err("Name can't be empty".into())
    } else if others.contains(&name) {
        Err(format!("Another connection is already named {name}"))
    } else {
        Ok(name.into())
    }
}

/// Names of every connection except `conn`, which a rename may keep.
pub(super) fn other_names(ws: &Workspace, conn: usize) -> Vec<&str> {
    ws.connections
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != conn)
        .map(|(_, c)| c.name.as_str())
        .collect()
}

/// Whether a key during a rename is consumed; chords pass through to the Shell (Ctrl+1..9).
pub(super) fn rename_swallows(m: Modifiers) -> bool {
    !m.control && !m.alt && !m.platform
}

/// F2 renames connections only.
pub(super) fn can_rename(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Connection(_))
}

impl Rename {
    pub(super) fn key(&mut self, key: &str, key_char: Option<&str>, others: &[&str]) -> RenameKey {
        match key {
            "escape" => return RenameKey::Cancel,
            "enter" => match validate_rename(&self.text, others) {
                Ok(name) => return RenameKey::Commit(name),
                Err(msg) => self.error = Some(msg),
            },
            "backspace" => {
                self.text.pop();
                self.error = None;
            }
            _ => {
                if let Some(ch) = key_char.filter(|c| !c.chars().any(char::is_control)) {
                    self.text.push_str(ch);
                    self.error = None;
                }
            }
        }
        RenameKey::Open
    }
}
