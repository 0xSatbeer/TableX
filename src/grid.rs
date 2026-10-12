//! Our own data grid (decision D3): rows virtualized by `uniform_list`, columns by
//! prefix-summed widths against the horizontal scroll offset.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Timelike as _};
use gpui_kit::component::calendar::{Calendar, CalendarEvent, CalendarState, Date};
use gpui_kit::component::time_field::{TimeField, TimeFieldState, TimePrecision};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::{ActiveTheme as _, Icon, Disableable as _, Sizable as _, button::{Button, ButtonVariants as _}, h_flex, input::{Input, InputEvent, InputState}, menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem}, tooltip::Tooltip, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{ColumnMeta, ResultSet, Row, Value};
use crate::{mock, theme};

pub trait GridSource {
    fn row_count(&self) -> usize;
    fn columns(&self) -> &[ColumnMeta];
    /// One row by value. Cells are Arc-backed, so cloning a stored row is cheap.
    fn row(&self, ix: usize) -> Row;
}

impl GridSource for ResultSet {
    fn row_count(&self) -> usize {
        self.rows.len()
    }
    fn columns(&self) -> &[ColumnMeta] {
        &self.columns
    }
    fn row(&self, ix: usize) -> Row {
        self.rows[ix].clone()
    }
}

/// `big_events`, generated per visible row instead of stored.
pub struct BigEvents(Vec<ColumnMeta>);

impl BigEvents {
    pub fn new() -> Self {
        Self(mock::big_events_columns())
    }
}

impl GridSource for BigEvents {
    fn row_count(&self) -> usize {
        mock::BIG_EVENTS_ROWS
    }
    fn columns(&self) -> &[ColumnMeta] {
        &self.0
    }
    fn row(&self, ix: usize) -> Row {
        mock::big_events_row(ix)
    }
}

/// One page of `all`: rows `start..start + len`.
struct Page {
    all: Rc<dyn GridSource>,
    start: usize,
    len: usize,
}

impl Page {
    fn new(all: Rc<dyn GridSource>, page: usize, size: usize) -> Self {
        let start = (page * size).min(all.row_count());
        Self { len: size.min(all.row_count() - start), all, start }
    }
}

impl GridSource for Page {
    fn row_count(&self) -> usize {
        self.len
    }
    fn columns(&self) -> &[ColumnMeta] {
        self.all.columns()
    }
    fn row(&self, ix: usize) -> Row {
        self.all.row(self.start + ix)
    }
}

const PAGE_SIZES: [usize; 5] = [100, 500, 1_000, 5_000, 10_000];

/// Pages needed for `rows` at `size` per page; an empty result still has one.
fn page_count(rows: usize, size: usize) -> usize {
    rows.div_ceil(size).max(1)
}

/// `1234567` as "1,234,567".
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

const DEFAULT_WIDTH: Pixels = px(160.);
const MIN_WIDTH: Pixels = px(40.);
/// Opening width cap: one long JSON column shouldn't push the rest off screen.
const MAX_OPEN_WIDTH: Pixels = px(320.);
const OVERSCAN: usize = 2;
const AUTOFIT_SAMPLE: usize = 60;
/// Display cap per cell; the cell clips visually, this only bounds shaping cost.
const MAX_CHARS: usize = 120;

/// `offsets[i]` is the left edge of column `i`; the last entry is the total width.
fn prefix_sums(widths: &[Pixels]) -> Vec<Pixels> {
    let mut acc = Pixels::ZERO;
    std::iter::once(Pixels::ZERO).chain(widths.iter().map(|w| {
        acc += *w;
        acc
    })).collect()
}

/// Widths in display order from per-source `widths` and `hidden`; hidden columns are zero wide.
fn display_widths(order: &[usize], widths: &[Pixels], hidden: &[bool]) -> Vec<Pixels> {
    order.iter().map(|&s| if hidden[s] { Pixels::ZERO } else { widths[s] }).collect()
}

/// Columns intersecting `[scroll_x, scroll_x + viewport)`, widened by `overscan` each side.
fn visible_cols(offsets: &[Pixels], scroll_x: Pixels, viewport: Pixels, overscan: usize) -> Range<usize> {
    let n = offsets.len() - 1;
    let start = offsets[1..].partition_point(|&end| end <= scroll_x);
    let end = offsets[..n].partition_point(|&left| left < scroll_x + viewport);
    start.saturating_sub(overscan)..(end + overscan).min(n)
}

#[derive(Debug, PartialEq)]
enum CellKind {
    Null,
    Number,
    Bool(bool),
    Text,
}

/// Column types whose values align right, so a NULL in them lines up with the numbers.
fn is_numeric_type(ty: &str) -> bool {
    let ty = ty.to_lowercase();
    if ty.ends_with("[]") || ty.ends_with("range") {
        return false;
    }
    const PREFIXES: [&str; 8] = ["bigint", "smallint", "numeric", "decimal", "double", "float", "real", "money"];
    (ty.starts_with("int") && !ty.starts_with("interval")) || PREFIXES.iter().any(|p| ty.starts_with(p))
}

/// Whether a cell is right-aligned: numbers always, NULLs only in numeric columns.
fn aligns_right(kind: &CellKind, data_type: &str) -> bool {
    *kind == CellKind::Number || (*kind == CellKind::Null && is_numeric_type(data_type))
}

/// Grid row height for a pane's mono font size: GRID_ROW at MONO, in proportion otherwise.
fn row_height(mono: Pixels) -> Pixels {
    theme::GRID_ROW * (mono / theme::MONO)
}

/// Width of the row-number gutter: monospace digits plus padding.
fn gutter_width(rows: usize, mono: Pixels) -> Pixels {
    mono * 0.6 * gutter_digits(rows) as f32 + theme::space_12()
}

/// Digits the row-number gutter must fit: the row count's, but never fewer than 3.
fn gutter_digits(rows: usize) -> usize {
    (rows.max(1).ilog10() as usize + 1).max(3)
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    }
}

fn human_size(n: usize) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{} KB", n / 1024),
        n => format!("{} MB", n / (1024 * 1024)),
    }
}

/// One-line display text for a cell.
fn format_cell(value: &Value) -> (String, CellKind) {
    match value {
        Value::Null => ("NULL".into(), CellKind::Null),
        Value::Bool(b) => ((if *b { "true" } else { "false" }).into(), CellKind::Bool(*b)),
        Value::Int(_) | Value::Float(_) => (value.to_string(), CellKind::Number),
        Value::Bytes(b) => {
            let head: String = b.iter().take(4).map(|byte| format!("{byte:02X}")).collect();
            let more = if b.len() > 4 { "…" } else { "" };
            (format!("0x{head}{more} ({})", human_size(b.len())), CellKind::Text)
        }
        Value::Json(s) => (truncate(s, 60), CellKind::Text),
        Value::Text(s) => {
            let mut lines = s.lines();
            let first = truncate(lines.next().unwrap_or_default(), MAX_CHARS);
            let text = if lines.next().is_some() { format!("{first} ↵") } else { first };
            (text, CellKind::Text)
        }
        _ => (value.to_string(), CellKind::Text),
    }
}

/// Spreadsheet-pasteable TSV. NULL copies as an empty field; fields with tabs, newlines or
/// quotes are quoted with doubled quotes, which Sheets/Excel/LibreOffice all read back.
fn tsv(_: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let mut out = String::new();
    for row in rows {
        let fields: Vec<String> = row
            .iter()
            .map(|v| match v {
                Value::Null => String::new(),
                v => {
                    let s = v.to_string();
                    if s.contains(['\t', '\n', '\r', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
                }
            })
            .collect();
        out.push_str(&fields.join("\t"));
        out.push('\n');
    }
    out
}

/// Header drag: moves display position `from` to just before display position `before`
/// (`before == order.len()` is the end). Returns whether the order changed.
fn move_col(order: &mut Vec<usize>, from: usize, before: usize) -> bool {
    if before == from || before == from + 1 {
        return false;
    }
    let col = order.remove(from);
    order.insert(if before > from { before - 1 } else { before }, col);
    true
}

/// Source columns a copy of display positions `display` emits, in display order, hidden ones skipped.
fn copy_cols(order: &[usize], hidden: &[bool], display: Range<usize>) -> Vec<usize> {
    order[display].iter().copied().filter(|&s| !hidden[s]).collect()
}

/// `items` at the indices `ix`, in that order.
fn pick<T: Clone>(items: &[T], ix: &[usize]) -> Vec<T> {
    ix.iter().map(|&i| items[i].clone()).collect()
}

/// Head cell after a navigation key, or `None` for other keys. `hidden` is in display order;
/// every horizontal move skips hidden columns.
fn nav(key: &str, secondary: bool, shift: bool, (r, c): (usize, usize), rows: usize, hidden: &[bool], page: usize) -> Option<(usize, usize)> {
    let cols = hidden.len();
    // Nearest visible column left or right of `c`, or `c` at the edge.
    let step = |c: usize, right: bool| {
        let visible = |&i: &usize| !hidden[i];
        if right { (c + 1..cols).find(visible) } else { (0..c).rev().find(visible) }.unwrap_or(c)
    };
    let first = if hidden[0] { step(0, true) } else { 0 };
    let last = if hidden[cols - 1] { step(cols - 1, false) } else { cols - 1 };
    Some(match key {
        "up" => (r.saturating_sub(1), c),
        "down" => ((r + 1).min(rows - 1), c),
        "left" => (r, step(c, false)),
        "right" => (r, step(c, true)),
        "pageup" => (r.saturating_sub(page.max(1)), c),
        "pagedown" => ((r + page.max(1)).min(rows - 1), c),
        "home" if secondary => (0, first),
        "end" if secondary => (rows - 1, last),
        "home" => (r, first),
        "end" => (r, last),
        "tab" => (r, step(c, !shift)),
        _ => return None,
    })
}

/// Display columns (anchor, head) of a band: the clicked column, or for a whole row the first and last visible.
fn band_cols(col: Option<usize>, hidden: &[bool]) -> (usize, usize) {
    let edge = |key| nav(key, false, false, (0, 0), 1, hidden, 0).map_or(0, |(_, c)| c);
    col.map_or_else(|| (edge("home"), edge("end")), |c| (c, c))
}

/// Anchor, head and extra ranges, in display positions.
type Selection = ((usize, usize), (usize, usize), Vec<(Range<usize>, Range<usize>)>);

/// Selection after a header drop: a real move (`Some(first visible display column)`) resets it
/// to row 0 of that column, a no-op drop (`None`) keeps it.
fn selection_after_drop(reset_col: Option<usize>, selection: Selection) -> Selection {
    reset_col.map_or(selection, |c| ((0, c), (0, c), Vec::new()))
}

/// A selected block: row range, display column range.
type Block = (Range<usize>, Range<usize>);

/// Columns and rows of one selected block, hidden columns skipped, in display order.
fn block_data<R: AsRef<[Value]>>(block: &Block, order: &[usize], hidden: &[bool], columns: &[ColumnMeta], row: &impl Fn(usize) -> R) -> (Vec<ColumnMeta>, Vec<Vec<Value>>) {
    let ix = copy_cols(order, hidden, block.1.clone());
    (pick(columns, &ix), block.0.clone().map(|r| pick(row(r).as_ref(), &ix)).collect())
}

/// Every range in the order it was added: the extras, then the active one.
fn ranges_in_order(extra: &[Block], active: Block) -> Vec<Block> {
    extra.iter().cloned().chain(std::iter::once(active)).collect()
}

/// One `format` block per range, separated by a blank line.
fn copy_ranges<R: AsRef<[Value]>>(format: fn(&[ColumnMeta], &[Vec<Value>]) -> String, ranges: &[Block], order: &[usize], hidden: &[bool], columns: &[ColumnMeta], row: impl Fn(usize) -> R) -> String {
    let blocks: Vec<String> = ranges
        .iter()
        .map(|r| {
            let (cols, rows) = block_data(r, order, hidden, columns, &row);
            format(&cols, &rows)
        })
        .collect();
    blocks.join("\n")
}

/// Values of every range's first column as one list.
fn copy_in_list<R: AsRef<[Value]>>(ranges: &[Block], order: &[usize], hidden: &[bool], columns: &[ColumnMeta], row: impl Fn(usize) -> R) -> String {
    let firsts: Vec<Vec<Value>> = ranges
        .iter()
        .flat_map(|r| block_data(r, order, hidden, columns, &row).1)
        .filter_map(|mut values| (!values.is_empty()).then(|| vec![values.swap_remove(0)]))
        .collect();
    sql_in_list(&[], &firsts)
}

/// Fields of TSV text as `tsv` writes it: quoted fields may hold tabs, newlines and doubled quotes.
fn parse_tsv(text: &str) -> Vec<Vec<String>> {
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
fn parse_value(text: &str, column: &ColumnMeta) -> Result<Value, String> {
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
            let n: i64 = s.trim().parse().map_err(|_| "Not a whole number".to_string())?;
            let max = i64::MAX >> (64 - bits);
            if n > max || n < -max - 1 {
                return Err(format!("Too big for {ty}: use {} to {max}", -max - 1));
            }
            Ok(Value::Int(n))
        }
        _ if is_numeric_type(&ty) => s.trim().parse().map(Value::Float).map_err(|_| "Not a number".into()),
        _ if ty.starts_with("bool") => match s.trim().to_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "on" | "1" => Ok(Value::Bool(true)),
            "false" | "f" | "no" | "n" | "off" | "0" => Ok(Value::Bool(false)),
            _ => Err("Use true or false".into()),
        },
        _ if ty.starts_with("json") => match serde_json::from_str::<serde_json::Value>(s) {
            Ok(_) => Ok(Value::Json(s.into())),
            Err(e) => Err(format!("Not valid JSON: {e}")),
        },
        _ if ty.starts_with("timestamp") => {
            parse_timestamp(s, has_zone(&ty)).ok_or_else(|| format!("Not a date and time: use {}", if has_zone(&ty) { "2026-10-12 09:30:00+00" } else { "2026-10-12 09:30:00" }))
        }
        _ if ty == "date" => match NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d") {
            Ok(d) => Ok(Value::Text(d.format("%Y-%m-%d").to_string().into())),
            Err(_) => Err("Not a date: use 2026-10-12".into()),
        },
        _ if ty == "uuid" => {
            let hex: String = s.trim().chars().filter(|&c| c != '-').collect();
            let dashes_ok = s.trim().len() == 32 || s.trim().char_indices().filter(|&(_, c)| c == '-').map(|(i, _)| i).eq([8, 13, 18, 23]);
            if hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit()) && dashes_ok { Ok(Value::Text(s.trim().to_lowercase().into())) } else { Err("Not a UUID: use 123e4567-e89b-12d3-a456-426614174000".into()) }
        }
        _ if ty == "bytea" => {
            let hex = s.trim().strip_prefix("\\x").ok_or("Use hex bytes like \\x0a1b")?;
            if hex.len() % 2 != 0 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("Use hex bytes like \\x0a1b: two hex digits per byte".into());
            }
            Ok(Value::Bytes((0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()))
        }
        _ => Ok(Value::Text(s.into())),
    }
}

/// Whether a timestamp type keeps a zone: `timestamptz`, `timestamp with time zone`.
fn has_zone(ty: &str) -> bool {
    ty.starts_with("timestamptz") || ty.contains("with time zone")
}

/// Timestamp text as the grid shows it (`2026-10-12 09:30:00.5+05:30`), with or without
/// seconds, a `T`, or a time at all. A zoned column reads text without an offset as UTC.
fn parse_timestamp(s: &str, zoned: bool) -> Option<Value> {
    const FORMATS: [&str; 3] = ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M"];
    let s = s.trim();
    if zoned && let Some(at) = FORMATS.iter().find_map(|f| DateTime::parse_from_str(s, &format!("{f}%#z")).ok()) {
        return Some(Value::TimestampTz { micros: at.timestamp_micros(), offset_secs: at.offset().local_minus_utc() });
    }
    let at = FORMATS
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .or_else(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().map(|d| d.and_time(NaiveTime::MIN)))?;
    Some(timestamp_value(at, zoned.then_some(0)))
}

/// A timestamp cell as the wall-clock time it shows, and the zone offset it shows it in.
fn wall_time(value: &Value) -> Option<(NaiveDateTime, i32)> {
    let (micros, offset) = match *value {
        Value::Timestamp(micros) => (micros, 0),
        Value::TimestampTz { micros, offset_secs } => (micros, offset_secs),
        _ => return None,
    };
    Some((DateTime::from_timestamp_micros(micros + offset as i64 * 1_000_000)?.naive_utc(), offset))
}

/// Wall-clock `at` as a cell: `timestamp`, or with `offset` a `timestamptz` shown in that zone.
fn timestamp_value(at: NaiveDateTime, offset: Option<i32>) -> Value {
    let micros = at.and_utc().timestamp_micros();
    offset.map_or(Value::Timestamp(micros), |offset_secs| Value::TimestampTz { micros: micros - offset_secs as i64 * 1_000_000, offset_secs })
}

/// How a cell of this column is edited.
#[derive(Debug, PartialEq)]
enum EditorKind {
    Text,
    /// Pick one of these labels: an enum's, or a boolean's true / false.
    Options(Vec<Arc<str>>),
    Date,
    /// Date and time; `zoned` for timestamptz.
    DateTime { zoned: bool },
}

fn editor_kind(meta: &ColumnMeta) -> EditorKind {
    let ty = meta.data_type.to_lowercase();
    match () {
        _ if ty.ends_with("[]") => EditorKind::Text,
        _ if !meta.enum_values.is_empty() => EditorKind::Options(meta.enum_values.to_vec()),
        _ if ty.starts_with("bool") => EditorKind::Options(vec!["true".into(), "false".into()]),
        _ if ty == "date" => EditorKind::Date,
        _ if ty.starts_with("timestamp") => EditorKind::DateTime { zoned: has_zone(&ty) },
        _ => EditorKind::Text,
    }
}

/// Every cell of `ranges` in a visible column, once, as (display row, display column).
fn fill_targets(ranges: &[Block], hidden: &[bool]) -> Vec<(usize, usize)> {
    let mut cells: Vec<_> = ranges.iter().flat_map(|(rs, cs)| rs.clone().flat_map(move |r| cs.clone().map(move |c| (r, c)))).filter(|&(_, c)| !hidden[c]).collect();
    cells.sort_unstable();
    cells.dedup();
    cells
}

/// Where a pasted `block` lands: one value fills every selected cell, a bigger block goes from
/// `top_left` over visible columns, clipped at the last row and column.
fn paste_targets(block: &[Vec<String>], ranges: &[Block], top_left: (usize, usize), rows: usize, hidden: &[bool]) -> Vec<((usize, usize), String)> {
    if let [row] = block
        && let [value] = row.as_slice()
    {
        return fill_targets(ranges, hidden).into_iter().map(|cell| (cell, value.clone())).collect();
    }
    let cols: Vec<usize> = (top_left.1..hidden.len()).filter(|&c| !hidden[c]).collect();
    block
        .iter()
        .zip(top_left.0..rows)
        .flat_map(|(fields, r)| fields.iter().zip(&cols).map(move |(v, &c)| ((r, c), v.clone())))
        .collect()
}

actions!(grid, [CopyCsv, CopyJson, CopyMarkdown, CopySqlInsert, CopySqlInList, HideColumn, ShowAllColumns, Paste, SetValue]);

fn csv(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let field = |s: String| if s.contains([',', '\n', '\r', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s };
    let header = cols.iter().map(|c| field(c.name.to_string()));
    let mut out = header.collect::<Vec<_>>().join(",") + "\n";
    for row in rows {
        let line: Vec<String> =
            row.iter().map(|v| if *v == Value::Null { String::new() } else { field(v.to_string()) }).collect();
        out += &(line.join(",") + "\n");
    }
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if c.is_control() => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

fn json_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) | Value::Int(_) => v.to_string(),
        Value::Float(x) if x.is_finite() => v.to_string(),
        Value::Json(s) => s.to_string(),
        v => json_string(&v.to_string()),
    }
}

/// Array of objects keyed by column name.
fn json(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let objects: Vec<String> = rows
        .iter()
        .map(|row| {
            let fields: Vec<String> =
                cols.iter().zip(row).map(|(c, v)| format!("{}: {}", json_string(&c.name), json_value(v))).collect();
            format!("  {{{}}}", fields.join(", "))
        })
        .collect();
    format!("[\n{}\n]\n", objects.join(",\n"))
}

fn markdown(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let cell = |s: String| s.replace('|', "\\|").replace('\n', " ");
    let line = |cells: Vec<String>| format!("| {} |\n", cells.join(" | "));
    let mut out = line(cols.iter().map(|c| cell(c.name.to_string())).collect());
    out += &line(cols.iter().map(|_| "---".to_string()).collect());
    for row in rows {
        out += &line(row.iter().map(|v| cell(v.to_string())).collect());
    }
    out
}

fn sql_literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        Value::Int(_) => v.to_string(),
        Value::Float(x) if x.is_finite() => v.to_string(),
        v => format!("'{}'", v.to_string().replace('\'', "''")),
    }
}

// ponytail: the grid does not know its table name yet; QX-011 passes the real one
fn sql_insert(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let names: Vec<&str> = cols.iter().map(|c| c.name.as_ref()).collect();
    rows.iter()
        .map(|row| {
            let values: Vec<String> = row.iter().map(sql_literal).collect();
            format!("INSERT INTO table_name ({}) VALUES ({});\n", names.join(", "), values.join(", "))
        })
        .collect()
}

/// Values of the first selected column as `(a, b, c)`.
fn sql_in_list(_: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let values: Vec<String> = rows.iter().filter_map(|row| row.first()).map(sql_literal).collect();
    format!("({})", values.join(", "))
}

/// Full value for the inspector: pretty JSON, hex dump for bytes, raw text otherwise.
fn inspect_text(value: &Value) -> String {
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
fn pretty_json(s: &str) -> String {
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
fn cmp_values(a: &Value, b: &Value) -> Ordering {
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

/// Above this the sort belongs to the query (QX-011 passes it to ORDER BY), not the client.
const CLIENT_SORT_MAX: usize = 100_000;

/// What a header cell shows for one source column.
#[derive(Debug, PartialEq)]
struct HeaderModel {
    name: String,
    data_type: String,
    pk: bool,
}

fn header_model(c: &ColumnMeta) -> HeaderModel {
    HeaderModel { name: c.name.to_string(), data_type: c.data_type.to_string(), pk: c.is_pk }
}

/// Header height: two lines (name, type) plus vertical padding.
fn header_height(line: Pixels, pad: Pixels) -> Pixels {
    line * 2. + pad
}

/// Whether the header sort control works for a source of this size.
#[derive(Debug, PartialEq)]
enum SortAvail {
    Enabled,
    Disabled(&'static str),
}

fn sort_availability(rows: usize) -> SortAvail {
    if rows <= CLIENT_SORT_MAX {
        SortAvail::Enabled
    } else {
        SortAvail::Disabled("Too many rows to sort here; use ORDER BY")
    }
}

/// A header cell being dragged to a new display position.
#[derive(Clone)]
struct DragColumn {
    grid: EntityId,
    from: usize,
    name: SharedString,
}

impl Render for DragColumn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(theme::space_8())
            .py(theme::space_4())
            .rounded(theme::radius_sm())
            .border_1()
            .border_color(theme.border)
            .bg(theme.table_head)
            .text_size(theme::ui_sm())
            .text_color(theme.foreground)
            .child(self.name.clone())
    }
}

pub struct Grid {
    /// The current page of `all`; everything but paging works on it.
    source: Rc<Page>,
    all: Rc<dyn GridSource>,
    page: usize,
    page_size: usize,
    widths: Vec<Pixels>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// Selection is the rectangle between anchor and head, as (row, col).
    anchor: (usize, usize),
    head: (usize, usize),
    /// (column, mouse x at drag start, width at drag start)
    resizing: Option<(usize, Pixels, Pixels)>,
    /// (column, ascending) and the display-to-source row permutation it produced.
    sort: Option<(usize, bool)>,
    order: Option<Vec<usize>>,
    /// Earlier ranges kept by Ctrl+click, as (rows, cols). Copy uses only the active range.
    extra: Vec<(Range<usize>, Range<usize>)>,
    /// Transposed one-row view of the head row (Ctrl+Shift+R).
    record: bool,
    hidden: Vec<bool>,
    /// Display position to source column. Widths, hidden and sort are keyed by source column,
    /// so they travel with it; selection and navigation work in display positions.
    col_order: Vec<usize>,
    /// Column the header context menu was opened on.
    menu_col: usize,
    /// This pane's mono font size, before UI scale (Ctrl+= / Ctrl+- / Ctrl+0).
    mono: Pixels,
    /// What Ctrl+C copies as, picked in the toolbar.
    copy_csv: bool,
    /// WHERE and ORDER BY inputs; made on first render, since InputState needs the window.
    filter: Option<(Entity<InputState>, Entity<InputState>)>,
    /// Share of the filter bar the WHERE input takes, and (mouse x, share) while its edge is dragged.
    where_share: f32,
    filter_drag: Option<(Pixels, f32)>,
    /// Pending cell values by (row in `all`, source column), shown over the source until submitted.
    edits: HashMap<(usize, usize), Value>,
    /// Bumped on every edit, so the status bar recomputes an unchanged selection.
    revision: usize,
    /// The in-cell editor, while open (double-click, Enter, F2).
    editing: Option<Editing>,
    /// A refused paste: the cell that couldn't take its field, and why. Cleared by the next selection.
    notice: Option<((usize, usize), String)>,
    /// The multi-cell selection a click on one of its cells collapsed, so a double-click can restore it.
    before_click: Option<Selection>,
    /// A left button pressed on a cell is still down, so moving over cells extends the selection.
    drag_selecting: bool,
}

/// An open cell editor: the display cell it sits on and every cell its value goes to.
struct Editing {
    editor: Editor,
    cell: (usize, usize),
    // Fixed at open, since the click that blurs (and commits) it moves the selection first.
    targets: Vec<(usize, usize)>,
    /// Why the typed text can't be saved, shown under the cell until it is fixed.
    error: Option<String>,
}

enum Editor {
    /// Typed in place of the cell text.
    Text(Entity<InputState>),
    /// A list under the cell; `highlighted` is the row Enter picks.
    Options { values: Vec<Arc<str>>, current: Option<usize>, highlighted: usize },
    /// A calendar under the cell, with a time field for timestamps. `original` is the cell's
    /// value, whose sub-second part survives when the time is left as it was.
    Date { calendar: Entity<CalendarState>, time: Option<(Entity<TimeFieldState>, Option<i32>)>, original: Value },
}

pub type SelectionKey = (Range<usize>, Vec<usize>, Option<(usize, bool)>, usize);

impl Grid {
    pub fn new(source: Rc<dyn GridSource>, cx: &mut Context<Self>) -> Self {
        let n = source.columns().len();
        let mut grid = Self {
            source: Rc::new(Page::new(source.clone(), 0, PAGE_SIZES[1])),
            all: source,
            page: 0,
            page_size: PAGE_SIZES[1],
            widths: vec![DEFAULT_WIDTH; n],
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            anchor: (0, 0),
            head: (0, 0),
            resizing: None,
            sort: None,
            order: None,
            extra: Vec::new(),
            record: false,
            hidden: vec![false; n],
            col_order: (0..n).collect(),
            menu_col: 0,
            mono: theme::MONO,
            copy_csv: false,
            filter: None,
            where_share: 0.6,
            filter_drag: None,
            edits: HashMap::new(),
            revision: 0,
            editing: None,
            notice: None,
            before_click: None,
            drag_selecting: false,
        };
        // Open fitted to the first rows, so timestamps and UUIDs don't arrive clipped.
        for c in 0..n {
            grid.autofit(c);
            grid.widths[c] = grid.widths[c].min(MAX_OPEN_WIDTH);
        }
        grid
    }

    /// Shows page `page` at `size` rows per page; selection and sort start over.
    fn set_page(&mut self, page: usize, size: usize, cx: &mut Context<Self>) {
        (self.page_size, self.page) = (size, page.min(page_count(self.all.row_count(), size) - 1));
        self.source = Rc::new(Page::new(self.all.clone(), self.page, size));
        (self.anchor, self.head, self.sort, self.order) = ((0, 0), (0, 0), None, None);
        self.extra.clear();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Row at display position `r`, after sorting.
    fn row(&self, r: usize) -> Row {
        self.page_row(self.order.as_ref().map_or(r, |o| o[r]))
    }

    /// Page row `ix` with pending edits over it.
    fn page_row(&self, ix: usize) -> Row {
        let mut row = self.source.row(ix);
        if !self.edits.is_empty() {
            let at = self.source.start + ix;
            for (c, v) in row.iter_mut().enumerate() {
                if let Some(edit) = self.edits.get(&(at, c)) {
                    *v = edit.clone();
                }
            }
        }
        row
    }

    /// Row in `all` shown at display position `r`.
    fn abs_row(&self, r: usize) -> usize {
        self.source.start + self.order.as_ref().map_or(r, |o| o[r])
    }

    /// Stores `cells` (display row, display column, typed text) as pending edits.
    /// Stores `cells` (display row, display column, value) as pending edits.
    fn set_values(&mut self, cells: Vec<((usize, usize), Value)>, cx: &mut Context<Self>) {
        self.notice = None;
        for ((r, c), value) in cells {
            self.edits.insert((self.abs_row(r), self.col_order[c]), value);
        }
        self.revision += 1;
        cx.notify();
    }

    /// Ctrl+V: clipboard TSV into the selection; a multi-cell paste then selects what it wrote.
    /// One field the column can't hold stops the whole paste, and that cell shows why.
    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        let ranges = ranges_in_order(&self.extra, self.selection());
        let (rows, cols) = self.selection();
        let block = parse_tsv(&text);
        let cells = paste_targets(&block, &ranges, (rows.start, cols.start), self.source.row_count(), &self.display_hidden());
        let values: Result<Vec<_>, _> = cells
            .iter()
            .map(|&(cell, ref text)| parse_value(text, &self.source.columns()[self.col_order[cell.1]]).map(|v| (cell, v)).map_err(|why| (cell, why)))
            .collect();
        let values = match values {
            Ok(values) => values,
            Err((cell, why)) => {
                self.select(cell, false, cx);
                self.reveal_head();
                self.notice = Some((cell, format!("Nothing pasted. {why}")));
                return;
            }
        };
        let fill = matches!(block.as_slice(), [row] if row.len() == 1);
        if !fill && let (Some(first), Some(last)) = (cells.first(), cells.last()) {
            (self.anchor, self.head) = (first.0, last.0);
            self.extra.clear();
        }
        self.set_values(values, cx);
    }

    /// Selected cells in visible columns, once each.
    fn selected_cells(&self) -> Vec<(usize, usize)> {
        fill_targets(&ranges_in_order(&self.extra, self.selection()), &self.display_hidden())
    }

    /// Opens the editor for display cell `cell`'s type, showing its value; it writes to every selected cell.
    fn start_editing(&mut self, cell: (usize, usize), window: &mut Window, cx: &mut Context<Self>) {
        let meta = &self.source.columns()[self.col_order[cell.1]];
        let value = self.row(cell.0)[self.col_order[cell.1]].clone();
        let text = if value == Value::Null { String::new() } else { value.to_string() };
        let editor = match editor_kind(meta) {
            EditorKind::Text => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("NULL"));
                input.update(cx, |input, cx| {
                    input.set_value(text, window, cx);
                    input.focus(window, cx);
                });
                cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, window, cx| match ev {
                    // Clicking elsewhere keeps what was typed, as in a spreadsheet.
                    InputEvent::PressEnter { .. } | InputEvent::Blur => _ = this.stop_editing(true, window, cx),
                    // Once a reason shows, it follows the typing, so it goes away when the text is fixed.
                    InputEvent::Change => {
                        let text = input.read(cx).value();
                        if let Some(editing) = &this.editing
                            && editing.error.is_some()
                        {
                            let error = parse_value(&text, &this.source.columns()[this.col_order[editing.cell.1]]).err();
                            this.editing.as_mut().unwrap().error = error;
                            cx.notify();
                        }
                    }
                    _ => {}
                })
                .detach();
                Editor::Text(input)
            }
            EditorKind::Options(values) => {
                let current = values.iter().position(|v| **v == *text);
                Editor::Options { highlighted: current.unwrap_or(0), current, values }
            }
            kind => {
                let zoned = matches!(kind, EditorKind::DateTime { zoned: true });
                let shown = wall_time(&value).or_else(|| NaiveDate::parse_from_str(&text, "%Y-%m-%d").ok().map(|d| (d.and_time(NaiveTime::MIN), 0)));
                let calendar = cx.new(|cx| CalendarState::new(window, cx));
                calendar.update(cx, |c, cx| c.set_date(Date::Single(shown.map(|(at, _)| at.date())), window, cx));
                let time = (kind != EditorKind::Date).then(|| {
                    let field = cx.new(|cx| TimeFieldState::new(window, cx).precision(TimePrecision::Second));
                    field.update(cx, |f, cx| f.set_time(shown.map_or(NaiveTime::MIN, |(at, _)| at.time()), window, cx));
                    (field, zoned.then(|| shown.map_or(0, |(_, offset)| offset)))
                });
                if time.is_none() {
                    // A date is one click: picking the day is the whole edit.
                    cx.subscribe_in(&calendar, window, |this, _, CalendarEvent::Selected(_), window, cx| _ = this.stop_editing(true, window, cx)).detach();
                }
                Editor::Date { calendar, time, original: value }
            }
        };
        self.notice = None;
        self.editing = Some(Editing { editor, cell, targets: self.selected_cells(), error: None });
        cx.notify();
    }

    /// The value an editor holds: `Ok(None)` for a calendar with no day picked, `Err` for text
    /// that isn't a value of the column.
    fn editor_value(&self, editor: &Editor, column: &ColumnMeta, cx: &App) -> Result<Option<Value>, String> {
        let day = |calendar: &Entity<CalendarState>| calendar.read(cx).date().start();
        Ok(Some(match editor {
            Editor::Text(input) => parse_value(&input.read(cx).value(), column)?,
            Editor::Options { values, highlighted, .. } => parse_value(&values[*highlighted], column)?,
            Editor::Date { calendar, time: None, .. } => match day(calendar) {
                Some(day) => Value::Text(day.format("%Y-%m-%d").to_string().into()),
                None => return Ok(None),
            },
            Editor::Date { calendar, time: Some((field, offset)), original } => {
                let Some(day) = day(calendar) else { return Ok(None) };
                let mut at = day.and_time(field.read(cx).time());
                // The field edits whole seconds; keep the original's fraction if they match.
                if let Some((was, _)) = wall_time(original).filter(|(was, _)| was.with_nanosecond(0) == Some(at)) {
                    at = was;
                }
                timestamp_value(at, *offset)
            }
        }))
    }

    /// Closes the editor, writing its value to its targets when `commit`. A value the column
    /// can't hold keeps it open with the reason under the cell; returns whether it closed.
    fn stop_editing(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(editing) = &self.editing else { return true };
        if commit {
            let column = &self.source.columns()[self.col_order[editing.cell.1]];
            match self.editor_value(&editing.editor, column, cx) {
                Err(why) => {
                    self.editing.as_mut().unwrap().error = Some(why);
                    cx.notify();
                    return false;
                }
                Ok(Some(value)) => {
                    let cells = editing.targets.iter().map(|&cell| (cell, value.clone())).collect();
                    self.set_values(cells, cx);
                }
                Ok(None) => {}
            }
        }
        self.editing = None;
        // Focus inside the editor goes back to the grid; focus the user moved elsewhere stays.
        if self.focus.contains_focused(window, cx) {
            window.focus(&self.focus, cx);
        }
        cx.notify();
        true
    }

    /// Keys while a list or calendar is open: ↑↓ move through the list, Enter sets, Esc cancels.
    /// Returns whether the key was the editor's.
    fn on_picker_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(Editing { editor, .. }) = &mut self.editing else { return false };
        if matches!(editor, Editor::Text(_)) {
            return false;
        }
        match (key, editor) {
            ("escape", _) => _ = self.stop_editing(false, window, cx),
            ("enter", _) => _ = self.stop_editing(true, window, cx),
            ("up", Editor::Options { highlighted, .. }) => *highlighted = highlighted.saturating_sub(1),
            ("down", Editor::Options { highlighted, values, .. }) => *highlighted = (*highlighted + 1).min(values.len() - 1),
            // The grid stays still under an open list; the calendar's own keys still reach it.
            (_, Editor::Options { .. }) => {}
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Cell mouse-down: click selects (or extends, or adds a range); a double-click edits, keeping
    /// a multi-cell selection the first click collapsed when the cell was part of it.
    fn cell_down(&mut self, cell: (usize, usize), ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The grid's own focus tracking would take focus back from the editor on this press.
        if self.editing.as_ref().is_some_and(|e| e.cell == cell) {
            return window.prevent_default();
        }
        // Text commits now rather than on the blur, which lands a frame later; a list or calendar cancels.
        let typed = match &self.editing {
            Some(Editing { editor: Editor::Text(input), .. }) => Some(input.clone()),
            _ => None,
        };
        if !self.stop_editing(typed.is_some(), window, cx) {
            // Invalid text stays in its cell, focused, until it is fixed or Esc drops it.
            window.prevent_default();
            return typed.unwrap().update(cx, |input, cx| input.focus(window, cx));
        }
        window.focus(&self.focus, cx);
        let plain = !ev.modifiers.secondary() && !ev.modifiers.shift;
        if ev.click_count == 2 && plain {
            if let Some((anchor, head, extra)) = self.before_click.take() {
                (self.anchor, self.head, self.extra) = (anchor, head, extra);
            }
            window.prevent_default();
            return self.start_editing(cell, window, cx);
        }
        let multi = self.selected_cells().len() > 1 && self.is_selected(cell.0, cell.1);
        self.before_click = (multi && plain).then(|| (self.anchor, self.head, self.extra.clone()));
        if ev.modifiers.secondary() {
            self.add_range(cell, cx);
        } else {
            self.select(cell, ev.modifiers.shift, cx);
        }
        self.drag_selecting = true;
    }

    /// Header sort button: ascending, descending, off.
    fn cycle_sort(&mut self, c: usize, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            Some((sc, true)) if sc == c => Some((c, false)),
            Some((sc, false)) if sc == c => None,
            _ => Some((c, true)),
        };
        self.order = self.sort.map(|(c, asc)| {
            let keys: Vec<Value> = (0..self.source.row_count()).map(|r| self.page_row(r)[c].clone()).collect();
            let mut order: Vec<usize> = (0..keys.len()).collect();
            // NULLs stay last in both directions.
            order.sort_by(|&x, &y| match (&keys[x], &keys[y]) {
                (Value::Null, _) | (_, Value::Null) => cmp_values(&keys[x], &keys[y]),
                (a, b) if asc => cmp_values(a, b),
                (a, b) => cmp_values(b, a),
            });
            order
        });
        cx.notify();
    }

    fn selection(&self) -> (Range<usize>, Range<usize>) {
        let (a, h) = (self.anchor, self.head);
        (a.0.min(h.0)..a.0.max(h.0) + 1, a.1.min(h.1)..a.1.max(h.1) + 1)
    }

    /// Active range as display rows and visible source columns, plus the sort that maps those rows.
    /// Rows in the whole result, across pages.
    #[cfg(test)]
    pub fn row_count(&self) -> usize {
        self.all.row_count()
    }

    /// Whether the grid has no columns at all, as a table tab has before its data can be read.
    pub fn is_blank(&self) -> bool {
        self.all.columns().is_empty()
    }

    pub fn selection_key(&self) -> SelectionKey {
        let (rows, cols) = self.selection();
        let cols = cols.filter_map(|c| self.col_order.get(c).copied()).filter(|&s| !self.hidden[s]).collect();
        (rows, cols, self.sort, self.revision)
    }

    /// Status-bar aggregate of the active range, hidden columns skipped.
    // shortcut: Ctrl+click ranges are left out (one shape can't describe them); add them if users ask.
    pub fn selection_summary(&self) -> Option<String> {
        if self.source.row_count() == 0 {
            return None;
        }
        let (rows, cols, ..) = self.selection_key();
        let values = rows.clone().flat_map(|r| {
            let row = self.row(r);
            cols.iter().map(|&c| row[c].clone()).collect::<Vec<_>>()
        });
        crate::statusbar::aggregate(rows.len(), cols.len(), values)
    }

    fn select(&mut self, cell: (usize, usize), extend: bool, cx: &mut Context<Self>) {
        self.notice = None;
        self.head = cell;
        if !extend {
            self.anchor = cell;
            self.extra.clear();
        }
        cx.notify();
    }

    /// Header drop: display column `from` moves before `before`. A real move clears the selection.
    fn drop_col(&mut self, from: usize, before: usize, cx: &mut Context<Self>) {
        let moved = move_col(&mut self.col_order, from, before);
        let reset_col = moved.then(|| band_cols(None, &self.display_hidden()).0);
        let selection = (self.anchor, self.head, std::mem::take(&mut self.extra));
        (self.anchor, self.head, self.extra) = selection_after_drop(reset_col, selection);
        cx.notify();
    }

    /// Ctrl+click: keep the current range and start a new one at `cell`.
    fn add_range(&mut self, cell: (usize, usize), cx: &mut Context<Self>) {
        self.extra.push(self.selection());
        (self.anchor, self.head) = (cell, cell);
        cx.notify();
    }

    fn is_selected(&self, r: usize, c: usize) -> bool {
        let (rows, cols) = self.selection();
        std::iter::once((&rows, &cols))
            .chain(self.extra.iter().map(|(rs, cs)| (rs, cs)))
            .any(|(rs, cs)| rs.contains(&r) && cs.contains(&c))
    }

    /// Whole row (gutter click) or whole column (header click); shift extends from the anchor.
    fn select_band(&mut self, row: Option<usize>, col: Option<usize>, extend: bool, cx: &mut Context<Self>) {
        let (first_col, last_col) = band_cols(col, &self.display_hidden());
        let head = (row.unwrap_or(self.source.row_count().saturating_sub(1)), last_col);
        if !extend {
            self.anchor = (row.unwrap_or(0), first_col);
            self.extra.clear();
        }
        self.head = head;
        cx.notify();
    }

    /// Width that fits the header and the rows currently in view.
    fn autofit(&mut self, c: usize) {
        let top = (-self.scroll.0.borrow().base_handle.offset().y / self.row_h()) as usize;
        let rows = top..(top + AUTOFIT_SAMPLE).min(self.source.row_count());
        let meta = &self.source.columns()[c];
        let chars = rows
            .map(|r| format_cell(&self.row(r)[c]).0.chars().count())
            .chain([meta.name.chars().count(), meta.data_type.chars().count()])
            .max()
            .unwrap_or(0);
        // ponytail: advance estimated as 0.6em (mono); Inter headers can be under-sized, measure with the text system if that shows
        self.widths[c] = (self.mono_px() * 0.6 * chars as f32 + theme::space_16() + theme::space_4()).max(MIN_WIDTH);
    }

    fn gutter(&self) -> Pixels {
        gutter_width(self.all.row_count(), self.mono_px())
    }

    /// Scroll so the head cell is in view, both axes.
    fn reveal_head(&self) {
        self.scroll.scroll_to_item(self.head.0, ScrollStrategy::Nearest);
        let base = self.scroll.0.borrow().base_handle.clone();
        let offsets = self.offsets();
        let (left, right) = (offsets[self.head.1], offsets[self.head.1 + 1]);
        let view = base.bounds().size.width - self.gutter();
        let mut offset = base.offset();
        if left < -offset.x {
            offset.x = -left;
        } else if right > -offset.x + view {
            offset.x = -(right - view);
        }
        base.set_offset(offset);
    }

    fn display_hidden(&self) -> Vec<bool> {
        self.col_order.iter().map(|&s| self.hidden[s]).collect()
    }

    /// Display-order left edges, hidden columns at zero width.
    fn offsets(&self) -> Vec<Pixels> {
        prefix_sums(&display_widths(&self.col_order, &self.widths, &self.hidden))
    }

    fn copy_as(&self, format: fn(&[ColumnMeta], &[Vec<Value>]) -> String, cx: &mut Context<Self>) {
        let ranges = ranges_in_order(&self.extra, self.selection());
        let text = copy_ranges(format, &ranges, &self.col_order, &self.hidden, self.source.columns(), |r| self.row(r));
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn copy_in_list(&self, cx: &mut Context<Self>) {
        let ranges = ranges_in_order(&self.extra, self.selection());
        let text = copy_in_list(&ranges, &self.col_order, &self.hidden, self.source.columns(), |r| self.row(r));
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.source.row_count();
        if self.on_picker_key(&ev.keystroke.key, window, cx) {
            return cx.stop_propagation();
        }
        if self.editing.is_some() && ev.keystroke.key == "escape" {
            self.stop_editing(false, window, cx);
            return;
        }
        // Keys typed into the filter inputs bubble up here; they are not grid navigation.
        if rows == 0 || self.widths.is_empty() || !self.focus.is_focused(window) {
            return;
        }
        let m = ev.keystroke.modifiers;
        match ev.keystroke.key.as_str() {
            "c" if m.secondary() => return self.copy_as(if self.copy_csv { csv } else { tsv }, cx),
            "v" if m.secondary() => return self.paste(cx),
            "enter" | "f2" if !m.secondary() && !m.shift => return self.start_editing(self.head, window, cx),
            "r" if m.secondary() && m.shift => {
                self.record = !self.record;
                return cx.notify();
            }
            _ => {}
        }
        let page = (self.scroll.0.borrow().base_handle.bounds().size.height / self.row_h()) as usize;
        let Some(next) = nav(&ev.keystroke.key, m.secondary(), m.shift, self.head, rows, &self.display_hidden(), page) else { return };
        self.select(next, m.shift && ev.keystroke.key != "tab", cx);
        self.reveal_head();
        cx.stop_propagation();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (page, size, last) = (self.page, self.page_size, page_count(self.all.row_count(), self.page_size) - 1);
        let page_button = |id: &'static str, icon: IconName, tip: &'static str, to: usize| {
            Button::new(id).ghost().small().icon(icon).tooltip(tip).accessibility_label(tip).disabled(to == page).on_click(cx.listener(move |this, _, _, cx| this.set_page(to, size, cx)))
        };
        let (start, end) = (self.source.start, self.source.start + self.source.len);
        let range = if end == 0 { "No rows".to_string() } else { format!("{}–{} of {}", grouped(start + 1), grouped(end), grouped(self.all.row_count())) };
        // shortcut: no query backend yet, so these stay disabled until results come from a live connection.
        let pending = |id: &'static str, icon: IconName, tip: &'static str| {
            Button::new(id).ghost().small().icon(icon).tooltip(format!("{tip} (needs a live connection)")).accessibility_label(tip).disabled(true)
        };
        let divider = || div().flex_none().w(px(1.)).h(theme::space_16()).mx(theme::space_4()).bg(theme.border);
        let (grid, current) = (cx.entity().downgrade(), self.copy_csv);
        let pager = grid.clone();
        h_flex()
            .flex_none()
            .px(theme::space_4())
            .py(theme::space_2())
            .gap(theme::space_2())
            .border_b_1()
            .border_color(theme.border)
            .text_size(theme::ui())
            .child(page_button("grid-first", IconName::ChevronFirst, "First page", 0))
            .child(page_button("grid-prev", IconName::ChevronLeft, "Previous page", page.saturating_sub(1)))
            .child(
                Button::new("grid-page-size").ghost().small().label(range).dropdown_caret(true).tooltip("Rows per page").dropdown_menu(move |menu, _, _| {
                    PAGE_SIZES.into_iter().fold(menu.label("Rows per page"), |menu, n| {
                        let pager = pager.clone();
                        menu.item(PopupMenuItem::new(grouped(n)).checked(n == size).on_click(move |_, _, cx| {
                            // Stay on the page holding the current first row.
                            _ = pager.update(cx, |this, cx| this.set_page(start / n, n, cx));
                        }))
                    })
                }),
            )
            .child(page_button("grid-next", IconName::ChevronRight, "Next page", (page + 1).min(last)))
            .child(page_button("grid-last", IconName::ChevronLast, "Last page", last))
            .child(divider())
            .child(pending("grid-refresh", IconName::RefreshCw, "Refresh"))
            .child(pending("grid-stop", IconName::Square, "Stop query"))
            .child(divider())
            .child(pending("grid-add-row", IconName::Plus, "Add row"))
            .child(pending("grid-delete-row", IconName::Minus, "Delete row"))
            .child(pending("grid-submit", IconName::ArrowUp, "Submit changes"))
            .child(div().flex_1())
            .child(
                Button::new("grid-copy-format")
                    .ghost()
                    .small()
                    .label(if self.copy_csv { "Copy as CSV" } else { "Copy as TSV" })
                    .dropdown_caret(true)
                    .tooltip("Format Ctrl+C copies")
                    .dropdown_menu(move |menu, _, _| {
                        [("TSV (tab-separated)", false), ("CSV (comma-separated)", true)].into_iter().fold(menu.label("Ctrl+C copies as"), |menu, (name, is_csv)| {
                            let grid = grid.clone();
                            menu.item(PopupMenuItem::new(name).checked(current == is_csv).on_click(move |_, _, cx| {
                                _ = grid.update(cx, |this, cx| {
                                    this.copy_csv = is_csv;
                                    cx.notify();
                                });
                            }))
                        })
                    }),
            )
    }

    // shortcut: no query backend yet, so WHERE / ORDER BY are typed but not applied; wire to the query once drivers land.
    fn render_filter_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (where_input, order_input) = self
            .filter
            .get_or_insert_with(|| {
                let mut input = |placeholder: &'static str, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
                (input("WHERE", cx), input("ORDER BY", cx))
            })
            .clone();
        let theme = cx.theme();
        let icon = |name: IconName| Icon::new(name).xsmall().text_color(theme.muted_foreground);
        h_flex()
            .flex_none()
            .items_stretch()
            .border_b_1()
            .border_color(theme.border)
            .text_size(theme::ui())
            .child(div().w(relative(self.where_share)).min_w_0().child(Input::new(&where_input).small().bordered(false).prefix(icon(IconName::Funnel))))
            .child(
                div()
                    .id("filter-split")
                    .flex_none()
                    .w(theme::space_8())
                    .cursor_col_resize()
                    .flex()
                    .justify_center()
                    .child(div().w(px(1.)).h_full().bg(theme.border))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.filter_drag = Some((ev.position.x, this.where_share));
                        cx.stop_propagation();
                    })),
            )
            .child(div().flex_1().min_w_0().child(Input::new(&order_input).small().bordered(false).prefix(icon(IconName::ArrowDownWideNarrow))))
    }

    /// Why a cell's text can't be saved, under that cell.
    fn render_problem(&self, why: String, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let bubble = div()
            .debug_selector(|| "cell-problem".into())
            .occlude()
            .max_w(px(420.))
            .px(theme::space_8())
            .py(theme::space_4())
            .rounded(theme::radius_md())
            .border_1()
            .border_color(theme.danger)
            .bg(theme.popover)
            .text_color(theme.danger)
            .shadow_lg()
            .font_family(theme.font_family.clone())
            .text_size(theme::ui_sm())
            .whitespace_normal()
            .child(why);
        div().absolute().top_full().left_0().child(deferred(anchored().snap_to_window_with_margin(theme::space_8()).child(bubble)).with_priority(1)).into_any_element()
    }

    /// The list or calendar under the cell being edited; `None` for text, which is edited in place.
    fn render_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editing = self.editing.as_ref()?;
        let theme = cx.theme();
        let n = editing.targets.len();
        let panel = v_flex()
            .id("cell-picker")
            .occlude()
            .min_w(self.widths[self.col_order[editing.cell.1]])
            .p(theme::space_4())
            .gap(theme::space_2())
            .rounded(theme::radius_md())
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .shadow_lg()
            .font_family(theme.font_family.clone())
            .text_size(theme::ui())
            .whitespace_normal()
            // Clicking away from a list or calendar picks nothing.
            .on_mouse_down_out(cx.listener(|this, _, window, cx| _ = this.stop_editing(false, window, cx)))
            // Say up front that the pick goes to the whole selection, not just this cell.
            .when(n > 1, |d| d.child(div().px(theme::space_8()).py(theme::space_2()).text_size(theme::ui_sm()).text_color(theme.muted_foreground).child(format!("Set {} cells to", grouped(n)))));
        let panel = match &editing.editor {
            Editor::Text(_) => return None,
            Editor::Options { values, current, highlighted } => panel.max_h(px(280.)).overflow_y_scroll().children(values.iter().enumerate().map(|(i, v)| {
                h_flex()
                    .id(("cell-option", i))
                    .debug_selector(move || format!("cell-option-{i}"))
                    .gap(theme::space_8())
                    .px(theme::space_8())
                    .h(self.row_h())
                    .rounded(theme::radius_sm())
                    .cursor_pointer()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(self.mono_px())
                    .hover(|d| d.bg(theme.accent))
                    .when(i == *highlighted, |d| d.bg(theme.accent).text_color(theme.accent_foreground))
                    .child(div().flex_none().w(theme::space_12()).when(*current == Some(i), |d| d.child(Icon::new(IconName::Check).xsmall())))
                    .child(v.to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(Editing { editor: Editor::Options { highlighted, .. }, .. }) = &mut this.editing {
                            *highlighted = i;
                        }
                        this.stop_editing(true, window, cx);
                    }))
            })),
            Editor::Date { calendar, time, .. } => {
                let picked = calendar.read(cx).date().start().is_some();
                panel.child(Calendar::new(calendar).small().border_0().p_0()).when_some(time.clone(), |d, (field, _)| {
                    d.child(
                        h_flex()
                            .gap(theme::space_8())
                            .pt(theme::space_4())
                            .border_t_1()
                            .border_color(theme.border)
                            .child(div().text_color(theme.muted_foreground).child("Time"))
                            .child(TimeField::new(&field).small())
                            .child(div().flex_1())
                            .child(
                                Button::new("cell-picker-set")
                                    .primary()
                                    .small()
                                    .label(if n > 1 { format!("Set {} cells", grouped(n)) } else { "Set".into() })
                                    .disabled(!picked)
                                    .tooltip(if picked { "Enter" } else { "Pick a day first" })
                                    .on_click(cx.listener(|this, _, window, cx| _ = this.stop_editing(true, window, cx))),
                            ),
                    )
                })
            }
        };
        Some(div().absolute().top_full().left_0().child(deferred(anchored().snap_to_window_with_margin(theme::space_8()).child(panel)).with_priority(1)).into_any_element())
    }

    /// Horizontal scroll position and viewport width of the cell area.
    fn viewport(&self, window: &Window) -> (Pixels, Pixels) {
        let base = self.scroll.0.borrow().base_handle.clone();
        let width = base.bounds().size.width;
        // Bounds are zero before the first layout; the window width over-covers, which is fine.
        let width = if width > Pixels::ZERO { width } else { window.viewport_size().width };
        (-base.offset().x, width - self.gutter())
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        let mut picker = self.render_picker(cx);
        let mut problem = self.editing.as_ref().and_then(|e| Some((e.cell, e.error.clone()?))).or_else(|| self.notice.clone());
        let theme = cx.theme();
        // Unfocused, the selection dims and loses its cursor, so only one pane looks "live".
        let focused = self.focus.is_focused(window);
        let (fill, cursor) = if focused {
            (theme.table_active, Some(theme.table_active_border))
        } else {
            (theme::selected_inactive(theme), None)
        };
        let offsets = self.offsets();
        let (scroll_x, view) = self.viewport(window);
        let cols = visible_cols(&offsets, scroll_x, view, OVERSCAN);
        let total = *offsets.last().unwrap_or(&Pixels::ZERO);
        let editing_cell = self.editing.as_ref().map(|e| e.cell);
        let text_editor = match self.editing.as_ref().map(|e| &e.editor) {
            Some(Editor::Text(input)) => Some(input.clone()),
            _ => None,
        };
        range
            .map(|r| {
                let (row, at) = (self.row(r), self.abs_row(r));
                let cells = cols.clone().map(|c| (c, self.col_order[c])).filter(|&(_, s)| !self.hidden[s]).map(|(c, s)| {
                    let (text, kind) = format_cell(&row[s]);
                    let selected = self.is_selected(r, c);
                    div()
                        .absolute()
                        .left(self.gutter() + offsets[c])
                        .w(self.widths[s])
                        .h_full()
                        .px(theme::space_8())
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .border_r_1()
                        .border_color(theme.table_row_border)
                        .when(aligns_right(&kind, &self.source.columns()[s].data_type), |d| d.justify_end())
                        .when(self.edits.contains_key(&(at, s)), |d| d.bg(theme::grid_edited(theme)))
                        .when(selected, |d| d.bg(fill))
                        .when_some(cursor.filter(|_| self.head == (r, c)), |d, color| d.border_1().border_color(color))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, ev: &MouseDownEvent, window, cx| this.cell_down((r, c), ev, window, cx)),
                        )
                        // Dragging across cells moves the head; the anchor stays where the drag began.
                        .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                            if this.drag_selecting && ev.dragging() && this.head != (r, c) {
                                this.select((r, c), true, cx);
                            }
                        }))
                        .when_some(text_editor.clone().filter(|_| editing_cell == Some((r, c))), |d, input| {
                            d.px_0().child(Input::new(&input).small().bordered(false).w_full().h_full().font_family(theme.mono_font_family.clone()).text_size(self.mono_px()))
                        })
                        .when_some(picker.take_if(|_| editing_cell == Some((r, c))), |d, picker| d.child(picker))
                        .when_some(problem.take_if(|(at, _)| *at == (r, c)), |d, (_, why)| d.border_1().border_color(theme.danger).child(self.render_problem(why, cx)))
                        .when(editing_cell != Some((r, c)) || text_editor.is_none(), |d| d.child(if kind == CellKind::Null {
                            div()
                                .px(theme::space_4())
                                .rounded(theme::radius_sm())
                                .bg(theme.muted)
                                .text_color(theme::grid_null(theme))
                                .child(text)
                        } else if kind == CellKind::Bool(false) {
                            div().text_color(theme.muted_foreground).child(text)
                        } else {
                            div().child(text)
                        }))
                });
                div()
                    .relative()
                    .w(self.gutter() + total)
                    .h(self.row_h())
                    .font_family(theme.mono_font_family.clone())
                    .text_size(self.mono_px())
                    .bg(if r % 2 == 1 { theme.table_even } else { theme.table })
                    .hover(|d| d.bg(theme.table_hover))
                    .border_b_1()
                    .border_color(theme.table_row_border)
                    .children(cells)
                    // Gutter is drawn last and pinned against the horizontal scroll.
                    .child(
                        div()
                            .absolute()
                            .left(scroll_x)
                            .w(self.gutter())
                            .h_full()
                            .pr(theme::space_8())
                            .flex()
                            .items_center()
                            .justify_end()
                            .bg(theme.table_head)
                            .border_r_1()
                            .border_color(theme.border)
                            .text_color(theme.muted_foreground)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                    window.focus(&this.focus, cx);
                                    this.select_band(Some(r), None, ev.modifiers.shift, cx);
                                }),
                            )
                            .child((self.source.start + r + 1).to_string()),
                    )
            })
            .collect()
    }

    /// One row transposed: column name, type, full value.
    fn render_record(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let r = self.head.0;
        let row = self.row(r);
        let name_w = theme::space_24() * 8.;
        div()
            .id("grid-record")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .font_family(theme.mono_font_family.clone())
            .text_size(self.mono_px())
            .child(
                div()
                    .debug_selector(|| "record-header".into())
                    .px(theme::space_8())
                    .h(self.row_h())
                    .flex()
                    .items_center()
                    .text_size(theme::ui_sm())
                    .text_color(theme.muted_foreground)
                    .child(format!("Row {} of {}   (Ctrl+Shift+R back to grid, ↑↓ change row)", self.source.start + r + 1, self.all.row_count())),
            )
            .children(self.source.columns().iter().zip(row.iter()).enumerate().map(|(c, (meta, value))| {
                let (text, kind) = format_cell(value);
                h_flex()
                    .min_h(self.row_h())
                    .px(theme::space_8())
                    .gap(theme::space_8())
                    .border_b_1()
                    .border_color(theme.table_row_border)
                    .when(c % 2 == 1, |d| d.bg(theme.table_even))
                    .child(div().flex_none().w(name_w).overflow_hidden().whitespace_nowrap().child(meta.name.to_string()))
                    .child(
                        div()
                            .flex_none()
                            .w(name_w / 2.)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(theme.muted_foreground)
                            .child(meta.data_type.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(kind == CellKind::Null, |d| d.text_color(theme::grid_null(theme)))
                            .child(if kind == CellKind::Null { text } else { inspect_text(value) }),
                    )
            }))
    }

    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let offsets = self.offsets();
        let (scroll_x, view) = self.viewport(window);
        let cols = visible_cols(&offsets, scroll_x, view, OVERSCAN);
        let columns = self.source.columns();
        div()
            .relative()
            .flex_none()
            .h(header_height(theme::badge_line(), theme::space_8()))
            .w_full()
            .overflow_hidden()
            .bg(theme.table_head)
            .border_b_1()
            .border_color(theme.border)
            .text_color(theme.table_head_foreground)
            .children(cols.map(|c| (c, self.col_order[c])).filter(|&(_, s)| !self.hidden[s]).map(|(c, s)| {
                h_flex()
                    .id(("col", s))
                    .absolute()
                    .left(self.gutter() + offsets[c] - scroll_x)
                    .w(self.widths[s])
                    .h_full()
                    .border_r_1()
                    .border_color(theme.table_row_border)
                    // Drag a header and drop it on another: the column moves before that one.
                    .on_drag(DragColumn { grid: cx.entity_id(), from: c, name: columns[s].name.clone().into() }, |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    })
                    .drag_over::<DragColumn>(|d, _, _, cx| d.border_l_2().border_color(cx.theme().drag_border))
                    .on_drop(cx.listener(move |this, drag: &DragColumn, _, cx| {
                        if drag.grid == cx.entity_id() {
                            this.drop_col(drag.from, c, cx);
                        }
                    }))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .justify_center()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                    window.focus(&this.focus, cx);
                                    this.select_band(None, Some(c), ev.modifiers.shift, cx);
                                }),
                            )
                            .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, _| this.menu_col = s))
                            .px(theme::space_8())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child({
                                let m = header_model(&columns[s]);
                                h_flex()
                                    .gap(theme::space_4())
                                    .h(theme::badge_line())
                                    .text_size(theme::ui_sm())
                                    .text_color(theme.foreground)
                                    .when(m.pk, |d| d.child(Icon::new(IconName::KeyRound).size_3().text_color(theme.muted_foreground)))
                                    .child({
                                        let name = SharedString::from(m.name);
                                        div().debug_selector(|| format!("header-name-{s}")).min_w_0().truncate().child(theme::truncating(("header-name", s), StyledText::new(name.clone()), name))
                                    })
                            })
                            .child(
                                div()
                                    .debug_selector(|| format!("header-type-{s}"))
                                    .h(theme::badge_line())
                                    .font_family(theme.mono_font_family.clone())
                                    .text_size(theme::ui_sm())
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child({
                                        let data_type = SharedString::from(header_model(&columns[s]).data_type);
                                        theme::truncating(("header-type", s), StyledText::new(data_type.clone()), data_type)
                                    }),
                            )
                            .context_menu({
                                let focus = self.focus.clone();
                                move |menu, _, _| {
                                    menu.action_context(focus.clone())
                                        .menu("Hide column", Box::new(HideColumn))
                                        .menu("Show all columns", Box::new(ShowAllColumns))
                                }
                            }),
                    )
                    .child({
                        let avail = sort_availability(self.source.row_count());
                        let glyph = match self.sort {
                            Some((sc, true)) if sc == s => "↑",
                            Some((sc, false)) if sc == s => "↓",
                            _ => "↕",
                        };
                        div()
                            .id(("sort", s))
                            .px(theme::space_4())
                            .text_size(theme::ui_sm())
                            // Drag must not start from the sort control.
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .map(|d| match avail {
                                SortAvail::Enabled => d
                                    .when(glyph == "↕", |d| d.text_color(theme::text_disabled(theme)))
                                    .hover(|d| d.text_color(theme.foreground))
                                    .on_click(cx.listener(move |this, _, _, cx| this.cycle_sort(s, cx))),
                                SortAvail::Disabled(why) => {
                                    d.text_color(theme::text_disabled(theme)).tooltip(move |window, cx| Tooltip::new(why).build(window, cx))
                                }
                            })
                            .child(glyph)
                    })
                    // Resize grip on the right edge.
                    .child(
                        div()
                            .id(("resize", s))
                            .w(theme::space_4())
                            .h_full()
                            .cursor_col_resize()
                            .hover(|d| d.bg(theme.primary))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                    if ev.click_count == 2 {
                                        this.autofit(s);
                                    } else {
                                        this.resizing = Some((s, ev.position.x, this.widths[s]));
                                    }
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                    )
            }))
            .child(div().absolute().left_0().w(self.gutter()).h_full().bg(theme.table_head).border_r_1().border_color(theme.border))
    }
}

impl Grid {
    #[cfg(test)]
    pub fn mono(&self) -> Pixels {
        self.mono
    }

    /// The pane's mono font size at the current UI scale.
    fn mono_px(&self) -> Pixels {
        theme::scaled(self.mono)
    }

    fn row_h(&self) -> Pixels {
        theme::scaled(row_height(self.mono))
    }

    fn zoom(&mut self, step: theme::Zoom, cx: &mut Context<Self>) {
        self.mono = theme::zoom(self.mono, step);
        cx.notify();
    }
}

impl Focusable for Grid {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Grid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &theme::ZoomIn, _, cx| this.zoom(theme::Zoom::In, cx)))
            .on_action(cx.listener(|this, _: &theme::ZoomOut, _, cx| this.zoom(theme::Zoom::Out, cx)))
            .on_action(cx.listener(|this, _: &theme::ZoomReset, _, cx| this.zoom(theme::Zoom::Reset, cx)))
            .on_action(cx.listener(|this, _: &CopyCsv, _, cx| this.copy_as(csv, cx)))
            .on_action(cx.listener(|this, _: &HideColumn, _, cx| {
                // Keep at least one column visible.
                if this.hidden.iter().filter(|h| !**h).count() > 1 {
                    this.hidden[this.menu_col] = true;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ShowAllColumns, _, cx| {
                this.hidden.fill(false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyJson, _, cx| this.copy_as(json, cx)))
            .on_action(cx.listener(|this, _: &CopyMarkdown, _, cx| this.copy_as(markdown, cx)))
            .on_action(cx.listener(|this, _: &CopySqlInsert, _, cx| this.copy_as(sql_insert, cx)))
            .on_action(cx.listener(|this, _: &CopySqlInList, _, cx| this.copy_in_list(cx)))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &SetValue, window, cx| this.start_editing(this.head, window, cx)))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some((c, start_x, start_w)) = this.resizing {
                    this.widths[c] = (start_w + ev.position.x - start_x).max(MIN_WIDTH);
                    cx.notify();
                }
                if let Some((start_x, start_share)) = this.filter_drag {
                    let width = this.scroll.0.borrow().base_handle.bounds().size.width;
                    if width > Pixels::ZERO {
                        this.where_share = (start_share + (ev.position.x - start_x) / width).clamp(0.15, 0.85);
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| (this.resizing, this.filter_drag, this.drag_selecting) = (None, None, false)))
            // Re-render on scroll so the header and column window follow the offset.
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .child(self.render_toolbar(cx))
            .child(self.render_filter_bar(window, cx))
            .when(!self.record, |d| d.child(self.render_header(window, cx)))
            .when(self.record && self.source.row_count() > 0, |d| d.child(self.render_record(cx)))
            .when(!self.record, |d| d.child(
                div().id("grid-body").debug_selector(|| "grid-body".into()).relative().flex_1().min_h_0().child(
                    uniform_list("grid-rows", self.source.row_count(), cx.processor(Self::render_rows))
                        .track_scroll(&self.scroll)
                        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                        .size_full(),
                )
                // Always shown: where you are in the rows and columns is worth seeing at rest.
                .child(Scrollbar::new(&self.scroll).id("grid-scrollbar").mode(ScrollbarMode::Always))
                .context_menu({
                    let focus = self.focus.clone();
                    move |menu, _, _| {
                        menu.action_context(focus.clone())
                            .label("Copy as")
                            .menu("CSV", Box::new(CopyCsv))
                            .menu("JSON", Box::new(CopyJson))
                            .menu("Markdown", Box::new(CopyMarkdown))
                            .menu("SQL INSERT", Box::new(CopySqlInsert))
                            .menu("SQL IN-list", Box::new(CopySqlInList))
                            .separator()
                            .menu("Paste (Ctrl+V)", Box::new(Paste))
                            .menu("Edit selected cells… (Enter)", Box::new(SetValue))
                    }
                }),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{aligns_right, gutter_digits, gutter_width, row_height, is_numeric_type, Block, copy_in_list, copy_ranges, ranges_in_order, GridSource as _, sort_availability, SortAvail, header_height, header_model, HeaderModel, band_cols, selection_after_drop, cmp_values, copy_cols, display_widths, nav, pick, move_col, csv, inspect_text, json, markdown, pretty_json, sql_in_list, sql_insert, CellKind, ColumnMeta, MAX_CHARS, format_cell, mock, prefix_sums, px, tsv, visible_cols, Value};

    #[test]
    fn parse_tsv_reads_back_what_tsv_copies() {
        let cols = [ColumnMeta { name: "a".into(), data_type: "text".into(), is_pk: false, ..Default::default() }];
        let rows = vec![vec![Value::Text("x\ty".into()), Value::Null], vec![Value::Text("say \"hi\"\nbye".into()), Value::Int(3)]];
        let parsed = super::parse_tsv(&tsv(&cols, &rows));
        assert_eq!(parsed, [vec!["x\ty", ""], vec!["say \"hi\"\nbye", "3"]]);
        assert_eq!(super::parse_tsv("1\t2\r\n3\t4"), [vec!["1", "2"], vec!["3", "4"]], "CRLF and no trailing newline");
        assert_eq!(super::parse_tsv("v\n"), [vec!["v"]]);
    }

    #[test]
    fn parse_value_follows_the_column_type() {
        let col = |ty: &str| ColumnMeta { name: "c".into(), data_type: ty.into(), ..Default::default() };
        let ok = |text: &str, ty: &str| super::parse_value(text, &col(ty)).unwrap_or_else(|e| panic!("{text:?} as {ty}: {e}"));
        let err = |text: &str, ty: &str| super::parse_value(text, &col(ty)).expect_err(&format!("{text:?} as {ty} should be refused"));
        assert_eq!(ok("", "integer"), Value::Null, "empty is NULL in any column");
        assert_eq!(ok("42", "integer"), Value::Int(42));
        assert_eq!(ok("4.5", "numeric(10,2)"), Value::Float(4.5));
        assert_eq!(ok("TRUE", "boolean"), Value::Bool(true));
        assert_eq!(ok("0", "boolean"), Value::Bool(false));
        assert_eq!(ok("{\"a\": [1, 2]}", "jsonb"), Value::Json("{\"a\": [1, 2]}".into()));
        assert_eq!(ok("42", "text"), Value::Text("42".into()));
        assert_eq!(ok("2026-10-12", "date"), Value::Text("2026-10-12".into()));
        assert_eq!(ok("\\x0aFF", "bytea"), Value::Bytes([0x0a, 0xff][..].into()));
        assert_eq!(ok("123E4567-e89b-12d3-a456-426614174000", "uuid"), Value::Text("123e4567-e89b-12d3-a456-426614174000".into()));
        assert_eq!(ok("32767", "smallint"), Value::Int(32_767));
        for (text, ty, why) in [
            ("abc", "bigint", "Not a whole number"),
            ("4.5", "integer", "Not a whole number"),
            ("32768", "smallint", "Too big for smallint"),
            ("2147483648", "integer", "Too big for integer"),
            ("12,5", "double precision", "Not a number"),
            ("maybe", "boolean", "Use true or false"),
            ("{\"a\": }", "jsonb", "Not valid JSON"),
            ("{'a': 1}", "json", "Not valid JSON"),
            ("soon", "timestamptz", "Not a date and time"),
            ("2026-02-30", "date", "Not a date"),
            ("123e4567e89b12d3a456", "uuid", "Not a UUID"),
            ("123e4567-e89b12d3-a456-426614174000-", "uuid", "Not a UUID"),
            ("0aff", "bytea", "Use hex bytes"),
            ("\\x0af", "bytea", "two hex digits"),
        ] {
            let got = err(text, ty);
            assert!(got.contains(why), "{text:?} as {ty}: {got:?}, want {why:?}");
        }
        let status = ColumnMeta { enum_values: ["open", "paid"].map(std::sync::Arc::from).into(), ..col("order_status") };
        assert_eq!(super::parse_value("paid", &status), Ok(Value::Text("paid".into())));
        assert_eq!(super::parse_value("Paid", &status), Err("Not one of: open, paid".into()));
    }

    #[test]
    fn paste_fills_one_value_and_lays_out_a_block() {
        use super::paste_targets;
        let block = |rows: &[&[&str]]| rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect::<Vec<Vec<String>>>();
        let one = paste_targets(&block(&[&["x"]]), &[(0..2, 0..2), (1..2, 1..3)], (1, 1), 10, &[false, false, true]);
        let cells: Vec<_> = one.iter().map(|(c, _)| *c).collect();
        assert_eq!(cells, [(0, 0), (0, 1), (1, 0), (1, 1)], "every selected visible cell once, hidden column 2 skipped");
        let grid = paste_targets(&block(&[&["a", "b", "c"], &["d", "e", "f"], &["g", "h", "i"]]), &[(1..2, 0..1)], (1, 0), 3, &[false, true, false, false]);
        let got: Vec<_> = grid.iter().map(|((r, c), v)| (*r, *c, v.as_str())).collect();
        assert_eq!(got, [(1, 0, "a"), (1, 2, "b"), (1, 3, "c"), (2, 0, "d"), (2, 2, "e"), (2, 3, "f")], "skips hidden column 1, clipped at the last row");
    }

    /// The open in-cell text input.
    fn text_input(g: &super::Grid) -> gpui_kit::Entity<gpui_kit::component::input::InputState> {
        match &g.editing.as_ref().expect("an editor is open").editor {
            super::Editor::Text(input) => input.clone(),
            _ => panic!("expected the text editor"),
        }
    }

    #[test]
    fn each_type_gets_its_editor() {
        use super::{editor_kind, EditorKind};
        let meta = |ty: &str| ColumnMeta { name: "c".into(), data_type: ty.into(), ..Default::default() };
        let cases = [
            ("boolean", EditorKind::Options(vec!["true".into(), "false".into()])),
            ("date", EditorKind::Date),
            ("timestamp", EditorKind::DateTime { zoned: false }),
            ("timestamp without time zone", EditorKind::DateTime { zoned: false }),
            ("timestamptz", EditorKind::DateTime { zoned: true }),
            ("timestamp with time zone", EditorKind::DateTime { zoned: true }),
            ("timestamptz[]", EditorKind::Text),
            ("text", EditorKind::Text),
            ("integer", EditorKind::Text),
        ];
        for (ty, kind) in cases {
            assert_eq!(editor_kind(&meta(ty)), kind, "{ty}");
        }
        let status = ColumnMeta { enum_values: ["open", "paid"].map(std::sync::Arc::from).into(), ..meta("order_status") };
        assert_eq!(editor_kind(&status), EditorKind::Options(vec!["open".into(), "paid".into()]));
    }

    #[test]
    fn timestamps_read_back_what_the_grid_shows() {
        use super::{parse_value, timestamp_value, wall_time};
        let values = [
            Value::Timestamp(1_760_000_000_123_456),
            Value::TimestampTz { micros: 1_760_000_000_000_000, offset_secs: 19_800 },
            Value::TimestampTz { micros: 1_760_000_000_000_000, offset_secs: -3_600 },
        ];
        for v in values {
            let ty = if matches!(v, Value::Timestamp(_)) { "timestamp" } else { "timestamptz" };
            let column = ColumnMeta { data_type: ty.into(), ..Default::default() };
            assert_eq!(parse_value(&v.to_string(), &column), Ok(v.clone()), "{v}");
            let (at, offset) = wall_time(&v).unwrap();
            assert_eq!(timestamp_value(at, (ty == "timestamptz").then_some(offset)), v, "wall time round trip of {v}");
        }
        let at = |text: &str, ty: &str| parse_value(text, &ColumnMeta { data_type: ty.into(), ..Default::default() }).unwrap().to_string();
        assert_eq!(at("2026-10-12", "timestamp"), "2026-10-12 00:00:00", "a bare date is midnight");
        assert_eq!(at("2026-10-12 09:30", "timestamptz"), "2026-10-12 09:30:00+00", "no offset reads as UTC");
    }

    #[gpui_kit::test]
    fn lists_and_calendars_edit_booleans_enums_and_timestamps(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        use super::Editor;
        cx.update(gpui_kit::init);
        let (window, grid) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|cx| super::Grid::new(std::rc::Rc::new(super::BigEvents::new()), cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        let cell = |cx: &mut gpui_kit::VisualTestContext, r: usize, c: usize| grid.read_with(cx, |g, _| g.row(r)[c].clone());
        let open_at = |cx: &mut gpui_kit::VisualTestContext, cell: (usize, usize)| {
            grid.update_in(cx, |g, window, cx| {
                window.focus(&g.focus, cx);
                (g.anchor, g.head) = (cell, cell);
            });
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
        };
        let (ok, event, occurred, logged) = (5, 3, 1, 11);

        // Boolean: a true/false list; ↓ then Enter flips it.
        let was = cell(&mut cx, 0, ok);
        open_at(&mut cx, (0, ok));
        assert!(matches!(grid.read_with(&cx, |g, _| g.editing.as_ref().map(|e| matches!(e.editor, Editor::Options { .. }))), Some(true)), "booleans open a list");
        assert!(cx.debug_bounds("cell-option-1").is_some(), "the list is drawn");
        let key = if was == Value::Bool(true) { "down" } else { "up" };
        cx.simulate_keystrokes(&format!("{key} enter"));
        assert_eq!(cell(&mut cx, 0, ok), Value::Bool(was != Value::Bool(true)), "↑↓ Enter picks the other value");
        assert_eq!(grid.read_with(&cx, |g, _| g.head), (0, ok), "the arrows moved the list, not the grid");

        // Enum: clicking a label sets it, on every selected cell.
        grid.update(&mut cx, |g, _| (g.anchor, g.head) = ((0, event), (2, event)));
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let purchase = cx.debug_bounds("cell-option-3").expect("enum labels listed").center();
        cx.simulate_click(purchase, Default::default());
        for r in 0..3 {
            assert_eq!(cell(&mut cx, r, event), Value::Text("purchase".into()), "row {r}");
        }

        // Esc closes a list without writing.
        open_at(&mut cx, (5, event));
        let before = cell(&mut cx, 5, event);
        cx.simulate_keystrokes("down escape");
        assert!(grid.read_with(&cx, |g, _| g.editing.is_none()));
        assert_eq!(cell(&mut cx, 5, event), before);

        // Timestamps: a calendar opened on the cell's day; Enter keeps the exact value, sub-seconds included.
        for col in [occurred, logged] {
            let was = cell(&mut cx, 4, col);
            open_at(&mut cx, (4, col));
            let day = grid.read_with(&cx, |g, cx| match &g.editing.as_ref().unwrap().editor {
                Editor::Date { calendar, time, .. } => {
                    assert!(time.is_some(), "timestamps also edit the time");
                    calendar.read(cx).date().start()
                }
                _ => panic!("timestamps open a calendar"),
            });
            assert_eq!(day, super::wall_time(&was).map(|(at, _)| at.date()));
            cx.simulate_keystrokes("enter");
            assert_eq!(cell(&mut cx, 4, col), was, "column {col}");
        }

        // A changed time is written in the cell's own zone.
        open_at(&mut cx, (6, occurred));
        grid.update_in(&mut cx, |g, window, cx| {
            let Editor::Date { time: Some((field, _)), .. } = &g.editing.as_ref().unwrap().editor else { unreachable!() };
            field.update(cx, |f, cx| f.set_time(chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap(), window, cx));
        });
        cx.simulate_keystrokes("enter");
        let written = cell(&mut cx, 6, occurred);
        assert!(matches!(written, Value::TimestampTz { offset_secs: 0, .. }) && written.to_string().ends_with(" 09:30:00+00"), "{written}");
    }

    #[gpui_kit::test]
    fn invalid_values_are_never_saved(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let (window, grid) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|cx| super::Grid::new(std::rc::Rc::new(super::BigEvents::new()), cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        let cell = |cx: &mut gpui_kit::VisualTestContext, r: usize, c: usize| grid.read_with(cx, |g, _| g.row(r)[c].clone());
        let error = |cx: &mut gpui_kit::VisualTestContext| grid.read_with(cx, |g, _| g.editing.as_ref().and_then(|e| e.error.clone()));
        let (amount, payload) = (4, 6);
        let type_in = |cx: &mut gpui_kit::VisualTestContext, text: &str| grid.update_in(cx, |g, window, cx| text_input(g).update(cx, |i, cx| i.set_value(text.to_string(), window, cx)));
        grid.update_in(&mut cx, |g, window, cx| {
            window.focus(&g.focus, cx);
            (g.anchor, g.head) = ((2, payload), (2, payload));
        });
        let was = cell(&mut cx, 2, payload);

        cx.simulate_keystrokes("enter");
        type_in(&mut cx, "{\"v\": ");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(error(&mut cx).is_some_and(|e| e.starts_with("Not valid JSON")), "Enter on broken JSON says why: {:?}", error(&mut cx));
        assert!(cx.debug_bounds("cell-problem").is_some(), "the reason shows under the cell");
        assert_eq!(cell(&mut cx, 2, payload), was, "and nothing is saved");
        let elsewhere = grid.read_with(&cx, |g, _| g.gutter() + g.offsets()[0] + px(5.));
        let body = cx.debug_bounds("grid-body").unwrap().origin;
        cx.simulate_click(gpui_kit::point(body.x + elsewhere, body.y + grid.read_with(&cx, |g, _| g.row_h()) * 8.5), Default::default());
        assert!(grid.read_with(&cx, |g, _| g.editing.is_some() && g.head == (2, payload)), "clicking another cell can't leave it either");
        cx.simulate_input("1}");
        assert_eq!(error(&mut cx), None, "typing the fix clears the reason");
        cx.simulate_keystrokes("enter");
        assert_eq!(cell(&mut cx, 2, payload), Value::Json("{\"v\": 1}".into()));

        // Esc always leaves, dropping the bad text.
        grid.update(&mut cx, |g, _| (g.anchor, g.head) = ((3, amount), (3, amount)));
        let was = cell(&mut cx, 3, amount);
        cx.simulate_keystrokes("enter");
        type_in(&mut cx, "12,5");
        cx.simulate_keystrokes("enter");
        assert!(error(&mut cx).is_some());
        cx.simulate_keystrokes("escape");
        assert!(grid.read_with(&cx, |g, _| g.editing.is_none()));
        assert_eq!(cell(&mut cx, 3, amount), was);

        // A paste with one bad field writes nothing and points at that field.
        let before: Vec<_> = (0..2).map(|r| cell(&mut cx, r, amount)).collect();
        grid.update(&mut cx, |g, _| (g.anchor, g.head) = ((0, amount), (0, amount)));
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("1.5\nlots\n".into()));
        cx.simulate_keystrokes("ctrl-v");
        assert_eq!((0..2).map(|r| cell(&mut cx, r, amount)).collect::<Vec<_>>(), before, "nothing pasted");
        assert_eq!(grid.read_with(&cx, |g, _| (g.head, g.notice.clone())), ((1, amount), Some(((1, amount), "Nothing pasted. Not a number".into()))));
    }

    #[gpui_kit::test]
    fn paste_and_set_value_edit_the_selected_cells(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let (window, grid) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|cx| super::Grid::new(std::rc::Rc::new(mock::wide_table(20)), cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        let cell = |cx: &mut gpui_kit::VisualTestContext, r: usize, c: usize| grid.read_with(cx, |g, _| g.row(r)[c].clone());
        grid.update_in(&mut cx, |g, window, cx| {
            window.focus(&g.focus, cx);
            (g.anchor, g.head) = ((0, 0), (1, 1));
        });
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("7".into()));
        cx.simulate_keystrokes("ctrl-v");
        assert_eq!([cell(&mut cx, 0, 0), cell(&mut cx, 1, 1)], [Value::Int(7), Value::Int(7)], "one value fills the selection");
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("1\t2\n3\t4\n".into()));
        grid.update(&mut cx, |g, _| (g.anchor, g.head) = ((5, 2), (5, 2)));
        cx.simulate_keystrokes("ctrl-v");
        assert_eq!([cell(&mut cx, 5, 2), cell(&mut cx, 6, 3)], [Value::Int(1), Value::Int(4)], "a block pastes from the top-left");
        assert_eq!(grid.read_with(&cx, |g, _| (g.anchor, g.head)), ((5, 2), (6, 3)), "and selects what it wrote");

        cx.simulate_keystrokes("enter");
        cx.simulate_input("99");
        cx.simulate_keystrokes("escape");
        assert!(grid.read_with(&cx, |g, _| g.editing.is_none()), "Esc closes the input");
        assert_eq!(cell(&mut cx, 5, 2), Value::Int(1), "Esc applies nothing");
        cx.simulate_keystrokes("enter");
        grid.update_in(&mut cx, |g, window, cx| text_input(g).update(cx, |i, cx| i.set_value("", window, cx)));
        cx.simulate_keystrokes("enter");
        assert!(grid.read_with(&cx, |g, _| g.editing.is_none()), "Enter applies and closes");
        assert_eq!([cell(&mut cx, 5, 2), cell(&mut cx, 5, 3), cell(&mut cx, 6, 2), cell(&mut cx, 6, 3)], [Value::Null, Value::Null, Value::Null, Value::Null], "empty sets every selected cell NULL");
    }

    #[gpui_kit::test]
    fn dragging_across_cells_selects_the_rectangle(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let (window, grid) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|cx| super::Grid::new(std::rc::Rc::new(mock::wide_table(20)), cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let (gutter, row_h, offsets) = grid.read_with(&cx, |g, _| (g.gutter(), g.row_h(), g.offsets()));
        let body = cx.debug_bounds("grid-body").expect("grid body drawn").origin;
        let at = |r: usize, c: usize| gpui_kit::point(body.x + gutter + offsets[c] + px(5.), body.y + row_h * (r as f32 + 0.5));
        cx.simulate_mouse_down(at(1, 0), gpui_kit::MouseButton::Left, Default::default());
        cx.simulate_mouse_move(at(3, 2), Some(gpui_kit::MouseButton::Left), Default::default());
        cx.simulate_mouse_up(at(3, 2), gpui_kit::MouseButton::Left, Default::default());
        assert_eq!(grid.read_with(&cx, |g, _| g.selection()), (1..4, 0..3), "anchor at the press, head at the release");
        cx.simulate_mouse_move(at(5, 4), None, Default::default());
        assert_eq!(grid.read_with(&cx, |g, _| g.selection()), (1..4, 0..3), "moving with no button pressed changes nothing");
    }

    #[gpui_kit::test]
    fn double_click_edits_in_the_cell_and_fills_the_selection(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let (window, grid) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|cx| super::Grid::new(std::rc::Rc::new(mock::wide_table(20)), cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let (gutter, row_h, offsets) = grid.read_with(&cx, |g, _| (g.gutter(), g.row_h(), g.offsets()));
        let body = cx.debug_bounds("grid-body").expect("grid body drawn").origin;
        let at = |r: usize, c: usize| gpui_kit::point(body.x + gutter + offsets[c] + px(5.), body.y + row_h * (r as f32 + 0.5));
        let double_click = |cx: &mut gpui_kit::VisualTestContext, r, c| {
            for click_count in [1, 2] {
                cx.simulate_event(gpui_kit::MouseDownEvent { button: gpui_kit::MouseButton::Left, position: at(r, c), modifiers: Default::default(), click_count, first_mouse: false });
                cx.simulate_event(gpui_kit::MouseUpEvent { button: gpui_kit::MouseButton::Left, position: at(r, c), modifiers: Default::default(), click_count });
            }
        };
        let cell = |cx: &mut gpui_kit::VisualTestContext, r: usize, c: usize| grid.read_with(cx, |g, _| g.row(r)[c].clone());
        let original = cell(&mut cx, 3, 3);

        grid.update(&mut cx, |g, _| (g.anchor, g.head) = ((0, 0), (1, 1)));
        double_click(&mut cx, 1, 0);
        assert_eq!(grid.read_with(&cx, |g, _| (g.selection(), g.editing.as_ref().map(|e| e.cell))), ((0..2, 0..2), Some((1, 0))), "double-click inside keeps the selection and edits that cell");
        grid.update_in(&mut cx, |g, window, cx| text_input(g).update(cx, |i, cx| i.set_value("5", window, cx)));
        cx.simulate_keystrokes("enter");
        assert_eq!([0, 1].map(|r| [0, 1].map(|c| cell(&mut cx, r, c))), [[Value::Int(5), Value::Int(5)], [Value::Int(5), Value::Int(5)]], "the typed value fills every selected cell");

        double_click(&mut cx, 3, 3);
        cx.simulate_input("8");
        assert_eq!(cell(&mut cx, 3, 3), original, "nothing is written while typing");
        cx.simulate_click(at(6, 0), Default::default());
        cx.run_until_parked();
        assert!(grid.read_with(&cx, |g, _| g.editing.is_none()), "clicking another cell closes the editor");
        assert_eq!(cell(&mut cx, 3, 3), Value::Int(format!("{original}8").parse().unwrap()), "and keeps what was typed, only in the edited cell");
        assert_eq!(cell(&mut cx, 6, 0), grid.read_with(&cx, |g, _| g.source.row(6)[0].clone()), "the clicked cell is untouched");
    }

    #[test]
    fn grouped_puts_commas_every_three_digits() {
        for (n, s) in [(0, "0"), (999, "999"), (1_000, "1,000"), (10_000, "10,000"), (1_000_000, "1,000,000")] {
            assert_eq!(super::grouped(n), s);
        }
    }

    #[test]
    fn pages_cover_every_row_once() {
        use super::{page_count, Page};
        let all: std::rc::Rc<dyn super::GridSource> = std::rc::Rc::new(mock::wide_table(1_050));
        assert_eq!(page_count(1_050, 500), 3);
        assert_eq!(page_count(0, 500), 1, "an empty result still has a page");
        let last = Page::new(all.clone(), 2, 500);
        assert_eq!((last.start, last.len), (1_000, 50));
        assert_eq!(last.row(0), all.row(1_000));
        let past = Page::new(all, 9, 500);
        assert_eq!(past.len, 0, "a page past the end is empty, not a panic");
    }

    #[test]
    fn visible_cols_follow_scroll_offset() {
        let offsets = prefix_sums(&[px(100.); 300]);
        assert_eq!(visible_cols(&offsets, px(0.), px(250.), 0), 0..3);
        assert_eq!(visible_cols(&offsets, px(100.), px(200.), 0), 1..3, "edge-aligned scroll skips col 0");
        assert_eq!(visible_cols(&offsets, px(150.), px(200.), 2), 0..6);
        assert_eq!(visible_cols(&offsets, px(29_900.), px(500.), 2), 297..300, "clamped at the end");
        let hidden = prefix_sums(&[px(100.), px(0.), px(100.), px(100.)]);
        assert_eq!(visible_cols(&hidden, px(100.), px(100.), 0), 2..3, "zero-width hidden column at the left edge is skipped");
    }

    #[test]
    fn move_col_drops_before_target() {
        // a=0, b=1, c=2, d=3
        let cases: [(&[usize], usize, usize, &[usize]); 5] = [
            (&[0, 1, 2, 3], 0, 3, &[1, 2, 0, 3]), // drag a before d
            (&[0, 1, 2], 2, 0, &[2, 0, 1]),       // drag c to start
            (&[0, 1, 2], 0, 3, &[1, 2, 0]),       // drag a past the last column
            (&[0, 1, 2, 3], 3, 1, &[0, 3, 1, 2]), // drag d left into the middle
            (&[2, 0, 1], 0, 2, &[0, 2, 1]),       // already permuted order
        ];
        for (start, from, before, expected) in cases {
            let mut order = start.to_vec();
            move_col(&mut order, from, before);
            assert_eq!(order, expected, "{start:?} move {from} before {before}");
        }
    }

    #[test]
    fn widths_hidden_and_sort_travel_with_the_column() {
        // Per source column a..d: widths 10..40, c hidden, sorted on b.
        let (widths, hidden, sorted) = ([px(10.), px(20.), px(30.), px(40.)], [false, false, true, false], 1);
        // (from, before) -> (order, display widths, display position of the sorted column, selection cleared)
        let cases: [((usize, usize), [usize; 4], [f32; 4], usize, bool); 3] = [
            ((0, 3), [1, 2, 0, 3], [20., 0., 10., 40.], 0, true),
            ((1, 0), [1, 0, 2, 3], [20., 10., 0., 40.], 0, true),
            ((3, 0), [3, 0, 1, 2], [40., 10., 20., 0.], 2, true),
        ];
        for ((from, before), order, expected, sort_at, cleared) in cases {
            let mut o = vec![0, 1, 2, 3];
            let changed = move_col(&mut o, from, before);
            assert_eq!(o, order, "move {from} before {before}");
            assert_eq!(display_widths(&o, &widths, &hidden), expected.map(px), "widths after move {from} before {before}");
            assert_eq!(o.iter().position(|&s| s == sorted), Some(sort_at), "sort still on b after move {from} before {before}");
            assert_eq!(changed, cleared, "selection cleared after move {from} before {before}");
        }
    }

    #[test]
    fn drop_on_own_position_keeps_order_and_selection() {
        // (from, before): before itself, before its right neighbour, and the last column at the end.
        for (from, before) in [(0, 0), (0, 1), (2, 2), (2, 3), (3, 3), (3, 4)] {
            let mut order = vec![0, 1, 2, 3];
            let changed = move_col(&mut order, from, before);
            assert_eq!(order, [0, 1, 2, 3], "order after move {from} before {before}");
            assert!(!changed, "selection kept after move {from} before {before}");
        }
    }

    fn abc() -> (Vec<ColumnMeta>, Vec<Vec<Value>>) {
        let cols = ["a", "b", "c"].map(|n| ColumnMeta { name: n.into(), data_type: "text".into(), is_pk: false, ..Default::default() }).to_vec();
        (cols, vec![["a0", "b0", "c0"].map(|v| Value::Text(v.into())).to_vec()])
    }

    /// What `Grid::copy_as` emits for display positions `display` of row 0.
    fn copy(format: fn(&[ColumnMeta], &[Vec<Value>]) -> String, order: &[usize], hidden: &[bool], display: std::ops::Range<usize>) -> String {
        let (cols, rows) = abc();
        let ix = copy_cols(order, hidden, display);
        format(&pick(&cols, &ix), &rows.iter().map(|r| pick(r, &ix)).collect::<Vec<_>>())
    }

    #[test]
    fn copy_skips_hidden_columns() {
        // (hidden, selected display positions, TSV)
        let cases: [([bool; 3], std::ops::Range<usize>, &str); 5] = [
            ([false, true, false], 0..3, "a0\tc0\n"), // select row 0 with b hidden
            ([false, false, false], 0..3, "a0\tb0\tc0\n"),
            ([true, false, true], 0..3, "b0\n"),
            ([false, true, false], 1..3, "c0\n"),
            ([false, true, false], 1..2, "\n"), // only the hidden column selected
        ];
        for (hidden, display, expected) in cases {
            assert_eq!(copy(tsv, &[0, 1, 2], &hidden, display.clone()), expected, "hidden {hidden:?}, selected {display:?}");
        }
        assert_eq!(copy(csv, &[0, 1, 2], &[false, true, false], 0..3), "a,c\na0,c0\n", "every format skips hidden");
    }

    #[test]
    fn copy_follows_display_order() {
        // (display order over a, b, c, hidden per source, selected display positions, CSV)
        let cases: [([usize; 3], [bool; 3], std::ops::Range<usize>, &str); 4] = [
            ([2, 0, 1], [false, true, false], 0..3, "c,a\nc0,a0\n"), // display order [c,a]
            ([2, 0, 1], [false, false, false], 0..2, "c,a\nc0,a0\n"),
            ([2, 1, 0], [false, false, false], 0..3, "c,b,a\nc0,b0,a0\n"),
            ([1, 2, 0], [true, false, false], 1..3, "c\nc0\n"), // hidden looked up by source, not position
        ];
        for (order, hidden, display, expected) in cases {
            assert_eq!(copy(csv, &order, &hidden, display.clone()), expected, "order {order:?}, hidden {hidden:?}, selected {display:?}");
        }
    }

    #[test]
    fn ctrl_home_end_skip_hidden_columns() {
        // (key, hidden in display order, from, expected head) over 10 rows
        let cases: [(&str, [bool; 3], (usize, usize), (usize, usize)); 6] = [
            ("end", [false, true, false], (0, 0), (9, 2)), // b hidden -> last row, column c
            ("end", [false, false, true], (0, 0), (9, 1)),
            ("end", [false, false, false], (3, 1), (9, 2)),
            ("home", [true, false, false], (5, 2), (0, 1)),
            ("home", [true, true, false], (5, 2), (0, 2)),
            ("home", [false, false, false], (5, 2), (0, 0)),
        ];
        for (key, hidden, from, expected) in cases {
            assert_eq!(nav(key, true, false, from, 10, &hidden, 5), Some(expected), "Ctrl+{key} from {from:?}, hidden {hidden:?}");
        }
    }

    #[test]
    fn horizontal_keys_skip_hidden_columns() {
        // (key, shift, hidden in display order, from, expected head) over 10 rows
        let cases: [(&str, bool, [bool; 4], (usize, usize), (usize, usize)); 9] = [
            ("right", false, [false, true, false, false], (3, 0), (3, 2)),
            ("right", false, [false, false, true, true], (3, 1), (3, 1)), // nothing visible to the right
            ("left", false, [false, true, false, false], (3, 2), (3, 0)),
            ("left", false, [true, true, false, false], (3, 2), (3, 2)), // nothing visible to the left
            ("tab", false, [false, false, true, false], (3, 1), (3, 3)),
            ("tab", true, [false, false, true, false], (3, 3), (3, 1)),
            ("home", false, [true, false, false, false], (3, 3), (3, 1)),
            ("end", false, [false, false, false, true], (3, 0), (3, 2)),
            ("end", false, [false, true, true, true], (3, 0), (3, 0)),
        ];
        for (key, shift, hidden, from, expected) in cases {
            assert_eq!(nav(key, false, shift, from, 10, &hidden, 5), Some(expected), "{key} shift={shift} from {from:?}, hidden {hidden:?}");
        }
    }

    #[test]
    fn real_move_resets_selection_and_own_position_keeps_it() {
        let selection = || ((2, 1), (4, 3), vec![(0..2, 0..1)]);
        // c hidden after the move [c, a, b] -> first visible display column is 1
        let first_visible = band_cols(None, &[true, false, false]).0;
        assert_eq!(selection_after_drop(Some(first_visible), selection()), ((0, 1), (0, 1), vec![]), "real move");
        assert_eq!(selection_after_drop(None, selection()), selection(), "own-position drop");
    }

    #[test]
    fn select_row_skips_hidden_columns() {
        // (clicked column, hidden in display order, expected (anchor, head) columns)
        let cases: [(Option<usize>, [bool; 3], (usize, usize)); 5] = [
            (None, [false, false, true], (0, 1)), // c hidden -> head on b
            (None, [true, false, false], (1, 2)), // a hidden -> anchor on b
            (None, [true, false, true], (1, 1)),
            (None, [false, false, false], (0, 2)),
            (Some(1), [false, true, false], (1, 1)), // a header click selects that column as is
        ];
        for (col, hidden, expected) in cases {
            assert_eq!(band_cols(col, &hidden), expected, "band {col:?}, hidden {hidden:?}");
        }
    }

    #[test]
    fn bools_format_as_words() {
        let cases = [(true, "true", CellKind::Bool(true)), (false, "false", CellKind::Bool(false))];
        for (b, text, kind) in cases {
            assert_eq!(format_cell(&Value::Bool(b)), (text.into(), kind));
        }
    }

    #[test]
    fn null_aligns_with_its_column() {
        let cases = [
            ("bigint", true),
            ("integer", true),
            ("int4", true),
            ("smallint", true),
            ("numeric(12,2)", true),
            ("double precision", true),
            ("real", true),
            ("text", false),
            ("timestamptz", false),
            ("boolean", false),
            ("", false),
            ("interval", false),
            ("point", false),
            ("INTEGER", true),
            ("integer[]", false),
            ("numeric[]", false),
            ("int4range", false),
        ];
        for (ty, right) in cases {
            assert_eq!(is_numeric_type(ty), right, "type {ty:?}");
        }
    }

    #[test]
    fn gutter_fits_the_row_count() {
        let cases = [(0, 3), (9, 3), (999, 3), (1000, 4), (99_999, 5), (100_000, 6), (999_999, 6), (1_000_000, 7)];
        for (rows, digits) in cases {
            assert_eq!(gutter_digits(rows), digits, "{rows} rows");
        }
    }

    #[test]
    fn aligns_right_by_kind_and_type() {
        let cases = [
            (CellKind::Null, "integer", true),
            (CellKind::Null, "INTEGER", true),
            (CellKind::Null, "text", false),
            (CellKind::Null, "integer[]", false),
            (CellKind::Number, "integer", true),
            (CellKind::Number, "text", true),
            (CellKind::Text, "integer", false),
            (CellKind::Bool(true), "integer", false),
        ];
        for (kind, ty, right) in cases {
            assert_eq!(aligns_right(&kind, ty), right, "{kind:?} in {ty:?}");
        }
    }

    #[test]
    fn row_height_scales_with_the_zoomed_font() {
        let cases = [(12.5, 26.), (13.5, 28.08), (11.5, 23.92), (9., 18.72), (24., 49.92)];
        for (mono, want) in cases {
            let got: f32 = row_height(px(mono)).into();
            assert!((got - want).abs() < 0.01, "mono {mono}: row {got}, want {want}");
        }
    }

    #[gpui_kit::test]
    fn record_header_row_follows_the_zoom(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(|cx| gpui_kit::init(cx));
        let (window, grid) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|cx| super::Grid::new(std::rc::Rc::new(super::BigEvents::new()), cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        grid.update(&mut cx, |grid, cx| {
            grid.record = true;
            grid.zoom(crate::theme::Zoom::In, cx);
        });
        cx.run_until_parked();
        let header: f32 = cx.debug_bounds("record-header").expect("record view renders its header").size.height.into();
        let row: f32 = grid.read_with(&cx, |grid, _| grid.row_h()).into();
        // Layout snaps to whole pixels.
        assert!((header - row).abs() < 0.5, "header row {header}px, field rows {row}px at mono 13.5");
    }

    #[test]
    fn gutter_width_is_digits_plus_padding() {
        // Spec rule: digits * one mono digit width (0.6 em) + SPACE_12.
        let cases = [(9, 3.0 * 7.5 + 12.0), (1_000_000, 7.0 * 7.5 + 12.0)];
        for (rows, px_width) in cases {
            let got: f32 = gutter_width(rows, crate::theme::MONO).into();
            assert!((got - px_width).abs() < 0.01, "{rows} rows: {got}");
        }
    }

    #[test]
    fn cells_format_by_type() {
        assert_eq!(format_cell(&Value::Null), ("NULL".into(), CellKind::Null));
        assert_eq!(format_cell(&Value::Int(-5)), ("-5".into(), CellKind::Number));
        assert_eq!(format_cell(&Value::Text("a\nb".into())).0, "a ↵");
        assert_eq!(format_cell(&Value::Bytes(std::sync::Arc::from(&[0u8; 5][..]))).0, "0x00000000… (5 B)");
        let long = format_cell(&Value::Text("é".repeat(500).into())).0;
        assert_eq!(long.chars().count(), MAX_CHARS + 1);
    }

    #[test]
    fn sort_order_is_typed_with_nulls_last() {
        let mut v = vec![Value::Null, Value::Float(2.5), Value::Int(10), Value::Int(-1), Value::Text("a".into())];
        v.sort_by(cmp_values);
        assert_eq!(v, [Value::Int(-1), Value::Float(2.5), Value::Int(10), Value::Text("a".into()), Value::Null]);
    }

    #[test]
    fn inspector_pretty_prints_json_and_dumps_bytes() {
        assert_eq!(pretty_json(r#"{"a": [1,2], "s": "x,{\"y\"}"}"#), "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"s\": \"x,{\\\"y\\\"}\"\n}");
        let bytes = Value::Bytes(std::sync::Arc::from(&[0xABu8; 17][..]));
        assert_eq!(inspect_text(&bytes).lines().nth(1), Some("00000010  ab"));
    }

    #[test]
    fn copy_formats_escape_their_syntax() {
        let cols = [ColumnMeta { name: "a".into(), data_type: "text".into(), is_pk: false, ..Default::default() }, ColumnMeta { name: "b".into(), data_type: "int".into(), is_pk: false, ..Default::default() }];
        let rows = vec![vec![Value::Text("it's, \"x\"|".into()), Value::Null], vec![Value::Text("y".into()), Value::Int(2)]];
        assert_eq!(csv(&cols, &rows), "a,b\n\"it's, \"\"x\"\"|\",\ny,2\n");
        assert_eq!(json(&cols, &rows), "[\n  {\"a\": \"it's, \\\"x\\\"|\", \"b\": null},\n  {\"a\": \"y\", \"b\": 2}\n]\n");
        assert_eq!(markdown(&cols, &rows).lines().nth(2), Some("| it's, \"x\"\\| | NULL |"));
        assert_eq!(sql_insert(&cols, &rows).lines().next(), Some("INSERT INTO table_name (a, b) VALUES ('it''s, \"x\"|', NULL);"));
        assert_eq!(sql_in_list(&cols, &rows), "('it''s, \"x\"|', 'y')");
    }

    #[test]
    fn tsv_quotes_and_blanks_nulls() {
        let rs = mock::edge_cases();
        // Rows 0 (null) and 3 (multiline), both columns.
        let rows = |r: std::ops::Range<usize>| rs.rows[r].iter().map(|row| row[0..2].to_vec()).collect::<Vec<_>>();
        assert_eq!(tsv(&rs.columns, &rows(0..1)), "null\t\n");
        assert_eq!(tsv(&rs.columns, &rows(3..4)), "multiline\t\"first line\nsecond line\n\tindented third\"\n");
    }

    fn col(name: &str, ty: &str, pk: bool) -> ColumnMeta {
        ColumnMeta { name: name.into(), data_type: ty.into(), is_pk: pk, ..Default::default() }
    }

    #[test]
    fn header_model_shows_name_type_and_pk_icon() {
        let m = |n: &str, t: &str, pk| HeaderModel { name: n.into(), data_type: t.into(), pk };
        let cases = [
            (col("id", "bigint", true), m("id", "bigint", true)),
            (col("email", "text", false), m("email", "text", false)),
            (col("", "", false), m("", "", false)),
        ];
        for (c, want) in cases {
            assert_eq!(header_model(&c), want);
        }
        // The icon belongs to the column: a reordered PK keeps it, and so does a hidden one (the model ignores both).
        let cols = [col("a", "text", false), col("id", "bigint", true), col("b", "int", false)];
        let shown: Vec<bool> = [1usize, 0, 2].iter().map(|&s| header_model(&cols[s]).pk).collect();
        assert_eq!(shown, [true, false, false]);
    }

    #[test]
    fn header_height_fits_two_lines() {
        use crate::theme::{badge_line, space_4, space_8};
        assert_eq!(header_height(badge_line(), space_8()), px(40.));
        assert_eq!(header_height(px(16.), px(0.)), px(32.));
        assert_eq!(header_height(px(14.), space_4()), px(32.));
        assert!(header_height(badge_line(), space_8()) >= badge_line() * 2.);
    }

    #[test]
    fn sort_is_available_up_to_client_sort_max() {
        const WHY: &str = "Too many rows to sort here; use ORDER BY";
        let cases = [
            (0, SortAvail::Enabled),
            (1, SortAvail::Enabled),
            (99_999, SortAvail::Enabled),
            (100_000, SortAvail::Enabled),
            (100_001, SortAvail::Disabled(WHY)),
            (1_000_000, SortAvail::Disabled(WHY)),
        ];
        for (rows, want) in cases {
            assert_eq!(sort_availability(rows), want, "{rows} rows");
        }
        assert_eq!(sort_availability(super::BigEvents::new().row_count()), SortAvail::Disabled(WHY), "big_events");
    }

    /// Row r, column c holds "v{r}{c}"; columns a, b, c.
    fn vgrid() -> (Vec<ColumnMeta>, impl Fn(usize) -> Vec<Value>) {
        let cols = ["a", "b", "c"].map(|n| ColumnMeta { name: n.into(), data_type: "text".into(), is_pk: false, ..Default::default() }).to_vec();
        (cols, |r| (0..3).map(|c| Value::Text(format!("v{r}{c}").into())).collect())
    }

    #[test]
    fn multi_range_blocks_are_separated_by_a_blank_line() {
        let (cols, row) = vgrid();
        // (ranges as (rows, display cols), TSV)
        let cases: [(Vec<Block>, &str); 4] = [
            (vec![(0..1, 0..1), (2..3, 1..2)], "v00\n\nv21\n"),
            (vec![(0..1, 0..1)], "v00\n"),
            (vec![(0..2, 0..2), (2..3, 2..3)], "v00\tv01\nv10\tv11\n\nv22\n"), // different widths
            (vec![(0..1, 0..2), (0..1, 1..3)], "v00\tv01\n\nv01\tv02\n"),      // overlapping
        ];
        for (ranges, expected) in cases {
            assert_eq!(copy_ranges(tsv, &ranges, &[0, 1, 2], &[false; 3], &cols, &row), expected, "ranges {ranges:?}");
        }
    }

    #[test]
    fn ranges_are_emitted_in_the_order_added() {
        let (cols, row) = vgrid();
        let (r1, r2): (Block, Block) = ((0..1, 0..1), (2..3, 1..2));
        // Ctrl+click adds: R2 first (kept in `extra`), then R1 is active.
        let ranges = ranges_in_order(&[r2.clone()], r1.clone());
        assert_eq!(ranges, [r2.clone(), r1.clone()]);
        assert_eq!(copy_ranges(tsv, &ranges, &[0, 1, 2], &[false; 3], &cols, &row), "v21\n\nv00\n", "R2 block first");
        // three ranges keep insertion order, not position order
        let r3: Block = (1..2, 2..3);
        assert_eq!(ranges_in_order(&[r2.clone(), r3.clone()], r1.clone()), [r2, r3, r1]);
        assert_eq!(ranges_in_order(&[], (4..5, 0..1)), [(4..5, 0..1)], "no extras: just the active range");
    }

    #[test]
    fn one_range_matches_the_single_range_copy_in_every_format() {
        let (cols, row) = vgrid();
        // display order [c, a, b] with b hidden, rows 0..2: literal output of each format
        let expected = [
            (tsv as fn(&[ColumnMeta], &[Vec<Value>]) -> String, "v02\tv00\nv12\tv10\n"),
            (csv, "c,a\nv02,v00\nv12,v10\n"),
            (markdown, "| c | a |\n| --- | --- |\n| v02 | v00 |\n| v12 | v10 |\n"),
            (json, "[\n  {\"c\": \"v02\", \"a\": \"v00\"},\n  {\"c\": \"v12\", \"a\": \"v10\"}\n]\n"),
            (sql_insert, "INSERT INTO table_name (c, a) VALUES ('v02', 'v00');\nINSERT INTO table_name (c, a) VALUES ('v12', 'v10');\n"),
            (sql_in_list, "('v02', 'v12')"),
        ];
        for (format, want) in expected {
            let got = copy_ranges(format, &[(0..2, 0..3)], &[2, 0, 1], &[false, true, false], &cols, &row);
            assert_eq!(got, want);
        }
    }

    #[test]
    fn in_list_merges_every_range_into_one_list() {
        let cols = ["a", "b", "c"].map(|n| ColumnMeta { name: n.into(), data_type: "text".into(), is_pk: false, ..Default::default() }).to_vec();
        let rows: Vec<Vec<Value>> = vec![
            vec![Value::Int(1), Value::Text("x".into()), Value::Null],
            vec![Value::Int(2), Value::Text("it's".into()), Value::Text("z".into())],
            vec![Value::Int(1), Value::Null, Value::Int(7)],
            vec![Value::Int(5), Value::Text("y".into()), Value::Int(8)],
        ];
        let row = |r: usize| rows[r].clone();
        let id = [0, 1, 2];
        // (ranges, order, hidden, expected); duplicates are kept: the list mirrors the selection, and IN ignores repeats
        let cases: [(Vec<Block>, [usize; 3], [bool; 3], &str); 7] = [
            (vec![(0..2, 0..1), (3..4, 0..1)], id, [false; 3], "(1, 2, 5)"),
            (vec![(0..1, 0..1)], id, [false; 3], "(1)"),
            (vec![(0..1, 1..2), (1..2, 1..2)], id, [false; 3], "('x', 'it''s')"), // strings quoted, quote doubled
            (vec![(0..3, 2..3), (2..3, 1..2)], id, [false; 3], "(NULL, 'z', 7, NULL)"), // NULLs, mixed types
            (vec![(0..1, 0..1), (2..3, 0..1)], id, [false; 3], "(1, 1)"),          // duplicates across ranges kept
            (vec![(0..2, 0..2), (0..1, 1..3)], id, [false; 3], "(1, 2, 'x')"),      // first column of each range only; overlap
            (vec![(0..2, 0..2), (3..4, 0..2)], [1, 0, 2], [true, false, false], "('x', 'it''s', 'y')"), // permuted, a hidden: b leads
        ];
        for (ranges, order, hidden, expected) in cases {
            assert_eq!(copy_in_list(&ranges, &order, &hidden, &cols, row), expected, "ranges {ranges:?}, order {order:?}, hidden {hidden:?}");
        }
    }

    #[gpui_kit::test]
    fn truncated_header_names_and_types_show_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        use crate::model::ResultSet;
        use crate::theme::tests::hover_shows_truncation_tip;
        cx.update(gpui_kit::init);
        let long = "shipment_line_item_carrier_tracking_events_archive_partitioned_by_month_2026_q3x";
        let col = |name: &str, data_type: &str| ColumnMeta { name: name.into(), data_type: data_type.into(), is_pk: false, ..Default::default() };
        let source = ResultSet {
            columns: vec![col("id", "bigint"), col(long, "text"), col("created_at", "timestamp with time zone")],
            rows: vec![vec![Value::Int(1), Value::Text("a".into()), Value::Text("2026-10-10".into())].into()],
        };
        let (window, grid) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|cx| super::Grid::new(std::rc::Rc::new(source), cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        // As if dragged by the user: wide enough for id/bigint, narrow for created_at.
        grid.update(&mut cx, |grid, cx| {
            grid.widths[0] = px(200.);
            grid.widths[2] = px(60.);
            cx.notify();
        });
        cx.run_until_parked();
        // (selector, tooltip shown); ones that fit first, so no earlier tooltip lingers.
        let cases = [("header-name-0", false), ("header-type-0", false), ("header-type-1", false), ("header-name-1", true), ("header-type-2", true)];
        for (selector, tip) in cases {
            assert_eq!(hover_shows_truncation_tip(&mut cx, selector.into()), tip, "{selector}");
        }
    }
}
