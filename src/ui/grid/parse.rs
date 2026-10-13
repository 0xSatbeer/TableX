//! Typed and pasted text read back as cell values.

use super::*;

/// Fields of TSV text as `tsv` writes it: quoted fields may hold tabs, newlines and doubled quotes.
pub(super) fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() => quoted = true,
            '\t' if !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted && chars.peek() == Some(&'\n') => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            ch => field.push(ch),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// A typed or pasted cell as a value of `column`, or why it can't be one. Empty is NULL, as `tsv` copies it.
// shortcut: arrays, time, interval, inet and other types are not checked; the database rejects them on submit.
pub(super) fn parse_value(text: &str, column: &ColumnMeta) -> Result<Value, String> {
    let ty = column.data_type.to_lowercase();
    let s = text;
    if s.is_empty() {
        return Ok(Value::Null);
    }
    if !column.enum_values.is_empty() {
        return match column.enum_values.iter().find(|v| ***v == *s) {
            Some(v) => Ok(Value::Text(v.clone())),
            None => Err(format!("Not one of: {}", column.enum_values.join(", "))),
        };
    }
    if ty.ends_with("[]") {
        return Ok(Value::Text(s.into()));
    }
    let int_bits = match ty.as_str() {
        t if t.starts_with("smallint") || t == "int2" => Some(16),
        t if t.starts_with("bigint") || t == "int8" => Some(64),
        t if t.starts_with("int") && !t.starts_with("interval") => Some(32),
        _ => None,
    };
    match () {
        _ if let Some(bits) = int_bits => {
            let n: i64 = s
                .trim()
                .parse()
                .map_err(|_| "Not a whole number".to_string())?;
            let max = i64::MAX >> (64 - bits);
            if n > max || n < -max - 1 {
                return Err(format!("Too big for {ty}: use {} to {max}", -max - 1));
            }
            Ok(Value::Int(n))
        }
        _ if is_numeric_type(&ty) => s
            .trim()
            .parse()
            .map(Value::Float)
            .map_err(|_| "Not a number".into()),
        _ if ty.starts_with("bool") => match s.trim().to_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "on" | "1" => Ok(Value::Bool(true)),
            "false" | "f" | "no" | "n" | "off" | "0" => Ok(Value::Bool(false)),
            _ => Err("Use true or false".into()),
        },
        _ if ty.starts_with("json") => match serde_json::from_str::<serde_json::Value>(s) {
            Ok(_) => Ok(Value::Json(s.into())),
            Err(e) => Err(format!("Not valid JSON: {e}")),
        },
        _ if ty.starts_with("timestamp") => parse_timestamp(s, has_zone(&ty)).ok_or_else(|| {
            format!(
                "Not a date and time: use {}",
                if has_zone(&ty) {
                    "2026-10-12 09:30:00+00"
                } else {
                    "2026-10-12 09:30:00"
                }
            )
        }),
        _ if ty == "date" => match NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d") {
            Ok(d) => Ok(Value::Text(d.format("%Y-%m-%d").to_string().into())),
            Err(_) => Err("Not a date: use 2026-10-12".into()),
        },
        _ if ty == "uuid" => {
            let hex: String = s.trim().chars().filter(|&c| c != '-').collect();
            let dashes_ok = s.trim().len() == 32
                || s.trim()
                    .char_indices()
                    .filter(|&(_, c)| c == '-')
                    .map(|(i, _)| i)
                    .eq([8, 13, 18, 23]);
            if hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit()) && dashes_ok {
                Ok(Value::Text(s.trim().to_lowercase().into()))
            } else {
                Err("Not a UUID: use 123e4567-e89b-12d3-a456-426614174000".into())
            }
        }
        _ if ty == "bytea" => {
            let hex = s
                .trim()
                .strip_prefix("\\x")
                .ok_or("Use hex bytes like \\x0a1b")?;
            if hex.len() % 2 != 0 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("Use hex bytes like \\x0a1b: two hex digits per byte".into());
            }
            Ok(Value::Bytes(
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect(),
            ))
        }
        _ => Ok(Value::Text(s.into())),
    }
}

/// Whether a timestamp type keeps a zone: `timestamptz`, `timestamp with time zone`.
pub(super) fn has_zone(ty: &str) -> bool {
    ty.starts_with("timestamptz") || ty.contains("with time zone")
}

/// Timestamp text as the grid shows it (`2026-10-12 09:30:00.5+05:30`), with or without
/// seconds, a `T`, or a time at all. A zoned column reads text without an offset as UTC.
pub(super) fn parse_timestamp(s: &str, zoned: bool) -> Option<Value> {
    const FORMATS: [&str; 3] = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
    ];
    let s = s.trim();
    if zoned
        && let Some(at) = FORMATS
            .iter()
            .find_map(|f| DateTime::parse_from_str(s, &format!("{f}%#z")).ok())
    {
        return Some(Value::TimestampTz {
            micros: at.timestamp_micros(),
            offset_secs: at.offset().local_minus_utc(),
        });
    }
    let at = FORMATS
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .or_else(|| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .map(|d| d.and_time(NaiveTime::MIN))
        })?;
    Some(timestamp_value(at, zoned.then_some(0)))
}

/// A timestamp cell as the wall-clock time it shows, and the zone offset it shows it in.
pub(super) fn wall_time(value: &Value) -> Option<(NaiveDateTime, i32)> {
    let (micros, offset) = match *value {
        Value::Timestamp(micros) => (micros, 0),
        Value::TimestampTz {
            micros,
            offset_secs,
        } => (micros, offset_secs),
        _ => return None,
    };
    Some((
        DateTime::from_timestamp_micros(micros + offset as i64 * 1_000_000)?.naive_utc(),
        offset,
    ))
}

/// Wall-clock `at` as a cell: `timestamp`, or with `offset` a `timestamptz` shown in that zone.
pub(super) fn timestamp_value(at: NaiveDateTime, offset: Option<i32>) -> Value {
    let micros = at.and_utc().timestamp_micros();
    offset.map_or(Value::Timestamp(micros), |offset_secs| Value::TimestampTz {
        micros: micros - offset_secs as i64 * 1_000_000,
        offset_secs,
    })
}
