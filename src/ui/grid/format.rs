//! Cell display text, alignment, the inspector text, and sort order.

use super::*;

/// Display cap per cell; the cell clips visually, this only bounds shaping cost.
pub(super) const MAX_CHARS: usize = 120;

#[derive(Debug, PartialEq)]
pub(super) enum CellKind {
    Null,
    Number,
    Bool(bool),
    Text,
}

/// Column types whose values align right, so a NULL in them lines up with the numbers.
pub(super) fn is_numeric_type(ty: &str) -> bool {
    let ty = ty.to_lowercase();
    if ty.ends_with("[]") || ty.ends_with("range") {
        return false;
    }
    const PREFIXES: [&str; 8] = [
        "bigint", "smallint", "numeric", "decimal", "double", "float", "real", "money",
    ];
    (ty.starts_with("int") && !ty.starts_with("interval"))
        || PREFIXES.iter().any(|p| ty.starts_with(p))
}

/// Whether a cell is right-aligned: numbers always, NULLs only in numeric columns.
pub(super) fn aligns_right(kind: &CellKind, data_type: &str) -> bool {
    *kind == CellKind::Number || (*kind == CellKind::Null && is_numeric_type(data_type))
}

pub(super) fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    }
}

pub(super) fn human_size(n: usize) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{} KB", n / 1024),
        n => format!("{} MB", n / (1024 * 1024)),
    }
}

/// One-line display text for a cell.
pub(super) fn format_cell(value: &Value) -> (String, CellKind) {
    match value {
        Value::Null => ("<null>".into(), CellKind::Null),
        Value::Bool(b) => (
            (if *b { "true" } else { "false" }).into(),
            CellKind::Bool(*b),
        ),
        Value::Int(_) | Value::Float(_) => (value.to_string(), CellKind::Number),
        Value::Bytes(b) => {
            let head: String = b.iter().take(4).map(|byte| format!("{byte:02X}")).collect();
            let more = if b.len() > 4 { "…" } else { "" };
            (
                format!("0x{head}{more} ({})", human_size(b.len())),
                CellKind::Text,
            )
        }
        Value::Json(s) => (truncate(s, 60), CellKind::Text),
        Value::Text(s) => {
            let mut lines = s.lines();
            let first = truncate(lines.next().unwrap_or_default(), MAX_CHARS);
            let text = if lines.next().is_some() {
                format!("{first} ↵")
            } else {
                first
            };
            (text, CellKind::Text)
        }
        _ => (value.to_string(), CellKind::Text),
    }
}

/// Full value for the inspector: pretty JSON, hex dump for bytes, raw text otherwise.
pub(super) fn inspect_text(value: &Value) -> String {
    match value {
        Value::Json(s) => pretty_json(s),
        Value::Bytes(b) => b
            .chunks(16)
            .enumerate()
            .map(|(i, line)| {
                let hex: Vec<String> = line.iter().map(|byte| format!("{byte:02x}")).collect();
                format!("{:08x}  {}", i * 16, hex.join(" "))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        v => v.to_string(),
    }
}

/// Re-indents JSON text with two spaces. Assumes valid JSON (it came from the database).
pub(super) fn pretty_json(s: &str) -> String {
    let (mut out, mut depth, mut in_str, mut escaped) = (String::new(), 0usize, false, false);
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    };
    for c in s.chars() {
        if in_str {
            out.push(c);
            (in_str, escaped) = (escaped || c != '"', !escaped && c == '\\');
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '{' | '[' => {
                depth += 1;
                out.push(c);
                newline(&mut out, depth);
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

/// Sort order: NULLs last, numbers numerically across int/float, other mixed kinds by variant.
pub(super) fn cmp_values(a: &Value, b: &Value) -> Ordering {
    use Value::*;
    let num = |v: &Value| match v {
        Int(n) => Some(*n as f64),
        Float(x) => Some(*x),
        _ => None,
    };
    let rank = |v: &Value| match v {
        Bool(_) => 0,
        Int(_) | Float(_) => 1,
        Timestamp(_) | TimestampTz { .. } => 2,
        Text(_) | Json(_) => 3,
        Bytes(_) => 4,
        Null => 5,
    };
    match (a, b) {
        (Bool(x), Bool(y)) => x.cmp(y),
        (Timestamp(x), Timestamp(y)) => x.cmp(y),
        (TimestampTz { micros: x, .. }, TimestampTz { micros: y, .. }) => x.cmp(y),
        (Text(x) | Json(x), Text(y) | Json(y)) => x.cmp(y),
        (Bytes(x), Bytes(y)) => x.cmp(y),
        _ => match (num(a), num(b)) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            _ => rank(a).cmp(&rank(b)),
        },
    }
}
