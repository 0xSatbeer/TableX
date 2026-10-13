//! The filter field: key routing and fuzzy matching of rows.

use super::*;

/// The nodes to show: the expanded tree, or the filtered full tree while a filter is typed.
pub(super) fn tree_nodes(
    ws: &Workspace,
    expanded: &HashSet<String>,
    loads: &HashMap<String, Load>,
    filter: &str,
) -> Vec<FlatNode> {
    if filter.is_empty() {
        flatten(ws, expanded, loads, false)
    } else {
        filter_nodes(flatten(ws, expanded, &HashMap::new(), true), filter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Focus {
    Tree,
    Field,
}

/// What a keystroke does to the filter field.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum KeyRoute {
    /// Not ours: normal tree / input handling.
    Pass,
    /// Tree-focused printable key: focus the field and append this text.
    ToField(String),
    /// Tree-focused Backspace with text in the field: drop its last character.
    PopField,
    ClearField,
    FocusTree,
    FocusFirstMatch,
}

pub(super) fn route_key(
    focus: Focus,
    key: &str,
    key_char: Option<&str>,
    m: Modifiers,
    field_empty: bool,
    has_match: impl FnOnce() -> bool,
    renaming: bool,
) -> KeyRoute {
    if focus == Focus::Field {
        return match key {
            "escape" if field_empty => KeyRoute::FocusTree,
            "escape" => KeyRoute::ClearField,
            "down"
                if !field_empty
                    && !m.shift
                    && !m.control
                    && !m.alt
                    && !m.platform
                    && has_match() =>
            {
                KeyRoute::FocusFirstMatch
            }
            _ => KeyRoute::Pass,
        };
    }
    if renaming {
        return KeyRoute::Pass;
    }
    let printable = key_char.filter(|c| !c.chars().any(char::is_control) && *c != " ");
    match key {
        "escape" if !field_empty => KeyRoute::ClearField,
        "backspace" if !field_empty => KeyRoute::PopField,
        // "*" stays the expand-all key while there is no filter.
        _ => match printable {
            Some(c) if !m.control && !m.alt && !m.platform && !(c == "*" && field_empty) => {
                KeyRoute::ToField(c.into())
            }
            _ => KeyRoute::Pass,
        },
    }
}

/// The single muted row shown when a filter matches nothing.
pub(super) fn no_match_text(filter: &str) -> String {
    format!("No objects match \"{filter}\"")
}

/// Index of the first node matching a non-empty filter.
pub(super) fn first_match(nodes: &[FlatNode], filter: &str) -> Option<usize> {
    nodes
        .iter()
        .position(|n| !filter.is_empty() && fuzzy_match(&n.label, filter).is_some())
}

/// Byte ranges of the characters in `label` that the filter matched, for highlighting.
pub(super) fn highlight_ranges(label: &str, filter: &str) -> Vec<Range<usize>> {
    fuzzy_match(label, filter)
        .unwrap_or_default()
        .into_iter()
        .map(|at| at..at + label[at..].chars().next().map_or(1, char::len_utf8))
        .collect()
}

/// Byte offsets of a case-insensitive subsequence match of `query` in `label`.
pub(super) fn fuzzy_match(label: &str, query: &str) -> Option<Vec<usize>> {
    let mut hits = Vec::new();
    let mut chars = label.char_indices();
    for q in query.chars().flat_map(char::to_lowercase) {
        let (at, _) = chars.find(|(_, c)| c.to_lowercase().eq(std::iter::once(q)))?;
        hits.push(at);
    }
    Some(hits)
}

/// Keeps matching nodes and their ancestors, dropping the rest. Relies on preorder: walking
/// backwards, the first node shallower than a kept one is its parent.
pub(super) fn filter_nodes(nodes: Vec<FlatNode>, query: &str) -> Vec<FlatNode> {
    let mut want: Option<usize> = None;
    let mut keep: Vec<FlatNode> = nodes
        .into_iter()
        .rev()
        .filter(|n| {
            let keep = fuzzy_match(&n.label, query).is_some() || want.is_some_and(|w| n.depth < w);
            if keep {
                want = Some(n.depth);
            }
            keep
        })
        .collect();
    keep.reverse();
    keep
}

/// Index of the nearest node above `ix` with a smaller depth.
pub(super) fn parent_of(nodes: &[FlatNode], ix: usize) -> Option<usize> {
    let depth = nodes[ix].depth;
    nodes[..ix].iter().rposition(|n| n.depth < depth)
}
