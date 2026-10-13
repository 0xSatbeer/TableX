use crate::model::Value;

/// Larger selections show their shape only, so the bar never walks millions of cells.
const AGGREGATE_MAX: usize = 1_000_000;

/// Status-bar readout for a `rows` × `cols` selection; `None` for one cell or none.
pub fn aggregate(
    rows: usize,
    cols: usize,
    values: impl IntoIterator<Item = Value>,
) -> Option<String> {
    if rows * cols <= 1 {
        return None;
    }
    let plural = |n: usize, one: &str| {
        if n == 1 {
            format!("{n} {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    let shape = format!("{} × {}", plural(rows, "row"), plural(cols, "col"));
    if rows.saturating_mul(cols) > AGGREGATE_MAX {
        return Some(format!("{shape} · aggregate skipped (too many cells)"));
    }
    // Ints sum exactly in i128 (no i64 overflow); floats join only when there is one.
    let (mut count, mut numeric, mut ints, mut floats) = (0, 0, 0i128, None);
    for v in values {
        match v {
            Value::Null => continue,
            Value::Int(n) => (numeric, ints) = (numeric + 1, ints + n as i128),
            Value::Float(x) => (numeric, floats) = (numeric + 1, Some(floats.unwrap_or(0.0) + x)),
            _ => {}
        }
        count += 1;
    }
    let mut out = format!("{shape} · count {count}");
    if numeric > 0 {
        let total = ints as f64 + floats.unwrap_or(0.0);
        let sum = if floats.is_some() {
            decimal(total)
        } else {
            ints.to_string()
        };
        out += &format!(" · sum {sum} · avg {}", decimal(total / numeric as f64));
    }
    Some(out)
}

/// At most 4 decimals, trailing zeros trimmed: `0.75`, `0.3333`, `4`.
fn decimal(x: f64) -> String {
    let s = format!("{x:.4}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        &s
    };
    if s == "-0" { "0".into() } else { s.into() }
}
