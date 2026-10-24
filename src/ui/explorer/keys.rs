//! Tree keys: escaped name segments joined by `/`, and the status rows under them.

use super::*;

/// The row shown in place of `key`'s children while they load or after their load failed.
pub(super) fn load_row(
    loads: &HashMap<String, Load>,
    key: &str,
) -> Option<(String, NodeKind, String)> {
    match loads.get(key)? {
        Load::Loading => Some((
            format!("{key}/{LOADING}"),
            NodeKind::Loading,
            "Loading…".into(),
        )),
        Load::Refreshing => None,
        Load::Failed(reason) => Some((
            format!("{key}/{ERROR}"),
            NodeKind::LoadError,
            format!("Could not load: {reason}"),
        )),
    }
}

/// `name` as one tree-key segment: `\` and `/` escaped, so a name never splits or merges segments.
pub(super) fn seg(name: &str) -> String {
    name.replace('\\', r"\\").replace('/', r"\/")
}

/// The keys of `key`'s ancestors, outermost first: its prefixes up to each unescaped `/`.
pub(super) fn ancestors(key: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut escaped = false;
    for (i, c) in key.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '/' => out.push(&key[..i]),
            _ => {}
        }
    }
    out
}

/// `key`'s segments with their escapes undone: connection id, database, schema, kind, object, ...
pub(super) fn segments(key: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = key.chars();
    while let Some(c) = chars.next() {
        let last = out.last_mut().expect("never empty");
        match c {
            '\\' => last.extend(chars.next()),
            '/' => out.push(String::new()),
            c => last.push(c),
        }
    }
    out
}

/// Whether `key` is connection `id`'s row or one under it. The id segment is a plain number.
pub(super) fn of_connection(key: &str, id: ConnectionId) -> bool {
    key.split('/').next() == Some(id.0.to_string().as_str())
}

/// The last segment of a status row's key. A backslash never starts an escaped name's segment, so
/// these never clash with a database or schema named "loading" or "error".
pub(super) const LOADING: &str = r"\loading";
pub(super) const ERROR: &str = r"\error";
pub(super) const EMPTY: &str = r"\empty";
