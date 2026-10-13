use gpui_kit::*;

// ponytail: keyword list, not a parser; the log line only needs a hint of structure.
// Swap for the editor's tree-sitter highlighter if rows need full fidelity.
const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "LIKE",
    "AS",
    "ON",
    "JOIN",
    "LEFT",
    "INNER",
    "GROUP",
    "ORDER",
    "BY",
    "LIMIT",
    "OFFSET",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "TABLE",
    "INDEX",
    "DROP",
    "ALTER",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "ASC",
    "DESC",
    "RETURNING",
];

/// Byte ranges of keywords, strings and numbers in `sql`, tagged with their `highlight.syntax` key.
pub(crate) fn sql_spans(sql: &str) -> Vec<(std::ops::Range<usize>, &'static str)> {
    let bytes = sql.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let b = bytes[i];
        if b == b'\'' {
            i = sql[i + 1..]
                .find('\'')
                .map_or(bytes.len(), |end| i + end + 2);
            spans.push((start..i, "string"));
        } else if b.is_ascii_alphanumeric() || b == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &sql[start..i];
            if word.bytes().all(|c| c.is_ascii_digit()) {
                spans.push((start..i, "number"));
            } else if KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(word)) {
                spans.push((start..i, "keyword"));
            }
        } else {
            i += sql[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    spans
}

/// `line` (SQL flattened to one line) with syntax colors.
pub(super) fn sql_line(line: &SharedString, theme: &gpui_kit::component::Theme) -> StyledText {
    let spans: Vec<_> = sql_spans(line)
        .into_iter()
        .filter_map(|(range, key)| Some((range, theme.highlight_theme.style(key)?)))
        .collect();
    StyledText::new(line.clone()).with_highlights(spans)
}
