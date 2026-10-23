//! Pure SQL text helpers: split a script into statements, find the one under the cursor,
//! and tell whether a statement only reads.
#![allow(dead_code, reason = "the console calls these")]

use std::ops::Range;

pub fn split(sql: &str) -> Vec<Range<usize>> {
    let code = mask(sql);
    let mut out = Vec::new();
    let mut start = 0;
    for end in code.match_indices(';').map(|(i, _)| i).chain([sql.len()]) {
        if !code[start..end].trim().is_empty() {
            let lead = sql[start..end].len() - sql[start..end].trim_start().len();
            out.push(start + lead..start + sql[start..end].trim_end().len());
        }
        start = end + 1;
    }
    out
}

pub fn at_cursor(sql: &str, offset: usize) -> Option<Range<usize>> {
    let all = split(sql);
    all.iter()
        .rev()
        .find(|r| r.start <= offset)
        .or(all.first())
        .cloned()
}

pub fn is_read_only(stmt: &str) -> bool {
    let code = mask(stmt).to_ascii_uppercase();
    let words: Vec<&str> = code
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect();
    let has = |k: &[&str]| words.iter().any(|w| k.contains(w));
    match words.first().copied() {
        Some("SELECT" | "SHOW" | "VALUES" | "TABLE") => true,
        Some("WITH") => !has(&["INSERT", "UPDATE", "DELETE", "MERGE"]),
        Some("EXPLAIN") => !has(&["ANALYZE", "ANALYSE"]),
        Some("PRAGMA") => !code.contains('='),
        _ => false,
    }
}

pub fn line_col(sql: &str, byte_offset: usize) -> (usize, usize) {
    let before = &sql[..byte_offset];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (
        before.matches('\n').count() + 1,
        before[line_start..].chars().count() + 1,
    )
}

/// Same byte length as `sql`; quoted text becomes `_` and comments become spaces,
/// so `;`, `=` and keywords found in it are real code.
pub(crate) fn mask(sql: &str) -> String {
    let b = sql.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        let (end, fill) = match b[i] {
            b'\'' | b'"' => (quoted(b, i), b'_'),
            b'$' => match dollar_tag(b, i) {
                Some(tag) => (
                    find(b, i + tag.len(), tag).map_or(b.len(), |j| j + tag.len()),
                    b'_',
                ),
                None => (i + 1, b'$'),
            },
            b'-' if b.get(i + 1) == Some(&b'-') => (find(b, i, b"\n").unwrap_or(b.len()), b' '),
            b'/' if b.get(i + 1) == Some(&b'*') => (block_comment(b, i), b' '),
            _ => (i + 1, b[i]),
        };
        out[i..end].fill(fill);
        i = end;
    }
    String::from_utf8(out).expect("only whole UTF-8 runs are replaced, with ASCII")
}

/// End of the '..' or ".." run opening at `i`; a doubled quote is an escape.
fn quoted(b: &[u8], i: usize) -> usize {
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() {
        if b[j] == q {
            if b.get(j + 1) != Some(&q) {
                return j + 1;
            }
            j += 1;
        }
        j += 1;
    }
    b.len()
}

/// `$tag$` opening at `i`; a digit after `$` is a parameter like `$1`, not a tag.
fn dollar_tag(b: &[u8], i: usize) -> Option<&[u8]> {
    let body = b[i + 1..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
        .count();
    let close = i + 1 + body;
    let starts_ok = body == 0 || !b[i + 1].is_ascii_digit();
    (starts_ok && b.get(close) == Some(&b'$')).then(|| &b[i..=close])
}

/// End of the `/* */` comment opening at `i`, counting nested ones.
fn block_comment(b: &[u8], i: usize) -> usize {
    let mut depth = 0;
    let mut j = i;
    while j + 1 < b.len() {
        match &b[j..j + 2] {
            b"/*" => depth += 1,
            b"*/" => depth -= 1,
            _ => {
                j += 1;
                continue;
            }
        }
        j += 2;
        if depth == 0 {
            return j;
        }
    }
    b.len()
}

fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

#[cfg(test)]
mod tests;
