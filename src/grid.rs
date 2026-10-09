//! Our own data grid (decision D3): rows virtualized by `uniform_list`, columns by
//! prefix-summed widths against the horizontal scroll offset.

use std::cmp::Ordering;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex, menu::ContextMenuExt as _, v_flex};
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

const GUTTER: Pixels = px(64.);
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
    Text,
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
        Value::Bool(b) => ((if *b { "✓" } else { "✗" }).into(), CellKind::Text),
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

actions!(grid, [CopyCsv, CopyJson, CopyMarkdown, CopySqlInsert, CopySqlInList, HideColumn, ShowAllColumns]);

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
            .px(theme::SPACE_8)
            .py(theme::SPACE_4)
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(theme.border)
            .bg(theme.table_head)
            .text_size(theme::UI_SM)
            .text_color(theme.foreground)
            .child(self.name.clone())
    }
}

pub struct Grid {
    source: Rc<dyn GridSource>,
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
    /// Value inspector for the head cell, toggled by Shift+Enter or double-click.
    inspecting: bool,
    /// Transposed one-row view of the head row (Ctrl+Shift+R).
    record: bool,
    hidden: Vec<bool>,
    /// Display position to source column. Widths, hidden and sort are keyed by source column,
    /// so they travel with it; selection and navigation work in display positions.
    col_order: Vec<usize>,
    /// Column the header context menu was opened on.
    menu_col: usize,
}

impl Grid {
    pub fn new(source: Rc<dyn GridSource>, cx: &mut Context<Self>) -> Self {
        let n = source.columns().len();
        let mut grid = Self {
            source,
            widths: vec![DEFAULT_WIDTH; n],
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            anchor: (0, 0),
            head: (0, 0),
            resizing: None,
            sort: None,
            order: None,
            extra: Vec::new(),
            inspecting: false,
            record: false,
            hidden: vec![false; n],
            col_order: (0..n).collect(),
            menu_col: 0,
        };
        // Open fitted to the first rows, so timestamps and UUIDs don't arrive clipped.
        for c in 0..n {
            grid.autofit(c);
            grid.widths[c] = grid.widths[c].min(MAX_OPEN_WIDTH);
        }
        grid
    }

    /// Row at display position `r`, after sorting.
    fn row(&self, r: usize) -> Row {
        self.source.row(self.order.as_ref().map_or(r, |o| o[r]))
    }

    /// Header sort button: ascending, descending, off.
    fn cycle_sort(&mut self, c: usize, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            Some((sc, true)) if sc == c => Some((c, false)),
            Some((sc, false)) if sc == c => None,
            _ => Some((c, true)),
        };
        self.order = self.sort.map(|(c, asc)| {
            let keys: Vec<Value> = (0..self.source.row_count()).map(|r| self.source.row(r)[c].clone()).collect();
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

    fn select(&mut self, cell: (usize, usize), extend: bool, cx: &mut Context<Self>) {
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
        let top = (-self.scroll.0.borrow().base_handle.offset().y / theme::GRID_ROW) as usize;
        let rows = top..(top + AUTOFIT_SAMPLE).min(self.source.row_count());
        let meta = &self.source.columns()[c];
        let chars = rows
            .map(|r| format_cell(&self.row(r)[c]).0.chars().count())
            .chain([meta.name.chars().count(), meta.data_type.chars().count()])
            .max()
            .unwrap_or(0);
        // ponytail: advance estimated as 0.6em (mono); Inter headers can be under-sized, measure with the text system if that shows
        self.widths[c] = (theme::MONO * 0.6 * chars as f32 + theme::SPACE_16 + theme::SPACE_4).max(MIN_WIDTH);
    }

    /// Scroll so the head cell is in view, both axes.
    fn reveal_head(&self) {
        self.scroll.scroll_to_item(self.head.0, ScrollStrategy::Nearest);
        let base = self.scroll.0.borrow().base_handle.clone();
        let offsets = self.offsets();
        let (left, right) = (offsets[self.head.1], offsets[self.head.1 + 1]);
        let view = base.bounds().size.width - GUTTER;
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
        let (rows, cols) = self.selection();
        let cols = copy_cols(&self.col_order, &self.hidden, cols);
        let rows: Vec<Vec<Value>> = rows.map(|r| pick(&self.row(r), &cols)).collect();
        let text = format(&pick(self.source.columns(), &cols), &rows);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.source.row_count();
        if rows == 0 || self.widths.is_empty() {
            return;
        }
        let m = ev.keystroke.modifiers;
        let page = (self.scroll.0.borrow().base_handle.bounds().size.height / theme::GRID_ROW) as usize;
        match ev.keystroke.key.as_str() {
            "c" if m.secondary() => return self.copy_as(tsv, cx),
            "r" if m.secondary() && m.shift => {
                self.record = !self.record;
                return cx.notify();
            }
            "enter" if m.shift => {
                self.inspecting = !self.inspecting;
                return cx.notify();
            }
            "escape" if self.inspecting => {
                self.inspecting = false;
                return cx.notify();
            }
            _ => {}
        }
        let Some(next) = nav(&ev.keystroke.key, m.secondary(), m.shift, self.head, rows, &self.display_hidden(), page) else { return };
        self.select(next, m.shift && ev.keystroke.key != "tab", cx);
        self.reveal_head();
        cx.stop_propagation();
    }

    /// Horizontal scroll position and viewport width of the cell area.
    fn viewport(&self, window: &Window) -> (Pixels, Pixels) {
        let base = self.scroll.0.borrow().base_handle.clone();
        let width = base.bounds().size.width;
        // Bounds are zero before the first layout; the window width over-covers, which is fine.
        let width = if width > Pixels::ZERO { width } else { window.viewport_size().width };
        (-base.offset().x, width - GUTTER)
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
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
        range
            .map(|r| {
                let row = self.row(r);
                let cells = cols.clone().map(|c| (c, self.col_order[c])).filter(|&(_, s)| !self.hidden[s]).map(|(c, s)| {
                    let (text, kind) = format_cell(&row[s]);
                    let selected = self.is_selected(r, c);
                    div()
                        .absolute()
                        .left(GUTTER + offsets[c])
                        .w(self.widths[s])
                        .h_full()
                        .px(theme::SPACE_8)
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .border_r_1()
                        .border_color(theme.table_row_border)
                        .when(kind == CellKind::Number, |d| d.justify_end())
                        .when(selected, |d| d.bg(fill))
                        .when_some(cursor.filter(|_| self.head == (r, c)), |d, color| d.border_1().border_color(color))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                window.focus(&this.focus, cx);
                                if ev.modifiers.secondary() {
                                    this.add_range((r, c), cx);
                                } else {
                                    this.select((r, c), ev.modifiers.shift, cx);
                                }
                                if ev.click_count == 2 {
                                    this.inspecting = true;
                                }
                            }),
                        )
                        .child(if kind == CellKind::Null {
                            div()
                                .px(theme::SPACE_4)
                                .rounded(theme::RADIUS_SM)
                                .bg(theme.muted)
                                .italic()
                                .text_color(theme::grid_null(theme))
                                .child(text)
                        } else {
                            div().child(text)
                        })
                });
                div()
                    .relative()
                    .w(GUTTER + total)
                    .h(theme::GRID_ROW)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme::MONO)
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
                            .w(GUTTER)
                            .h_full()
                            .px(theme::SPACE_8)
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
                            .child((r + 1).to_string()),
                    )
            })
            .collect()
    }

    /// One row transposed: column name, type, full value.
    fn render_record(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let r = self.head.0;
        let row = self.row(r);
        let name_w = theme::SPACE_24 * 8.;
        div()
            .id("grid-record")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .font_family(theme.mono_font_family.clone())
            .text_size(theme::MONO)
            .child(
                div()
                    .px(theme::SPACE_8)
                    .h(theme::GRID_ROW)
                    .flex()
                    .items_center()
                    .text_size(theme::UI_SM)
                    .text_color(theme.muted_foreground)
                    .child(format!("Row {} of {}   (Ctrl+Shift+R back to grid, ↑↓ change row)", r + 1, self.source.row_count())),
            )
            .children(self.source.columns().iter().zip(row.iter()).enumerate().map(|(c, (meta, value))| {
                let (text, kind) = format_cell(value);
                h_flex()
                    .min_h(theme::GRID_ROW)
                    .px(theme::SPACE_8)
                    .gap(theme::SPACE_8)
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
                            .when(kind == CellKind::Null, |d| d.italic().text_color(theme::grid_null(theme)))
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
            .h(theme::GRID_ROW + theme::SPACE_12)
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
                    .left(GUTTER + offsets[c] - scroll_x)
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
                            .px(theme::SPACE_8)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(div().text_size(theme::UI_SM).text_color(theme.foreground).child(columns[s].name.to_string()))
                            .child(div().text_size(theme::UI_SM).child(columns[s].data_type.to_string()))
                            .context_menu({
                                let focus = self.focus.clone();
                                move |menu, _, _| {
                                    menu.action_context(focus.clone())
                                        .menu("Hide column", Box::new(HideColumn))
                                        .menu("Show all columns", Box::new(ShowAllColumns))
                                }
                            }),
                    )
                    .when(self.source.row_count() <= CLIENT_SORT_MAX, |d| {
                        let glyph = match self.sort {
                            Some((sc, true)) if sc == s => "↑",
                            Some((sc, false)) if sc == s => "↓",
                            _ => "↕",
                        };
                        d.child(
                            div()
                                .id(("sort", s))
                                .px(theme::SPACE_4)
                                .text_size(theme::UI_SM)
                                .when(glyph == "↕", |d| d.text_color(theme::text_disabled(theme)))
                                .hover(|d| d.text_color(theme.foreground))
                                .on_click(cx.listener(move |this, _, _, cx| this.cycle_sort(s, cx)))
                                .child(glyph),
                        )
                    })
                    // Resize grip on the right edge.
                    .child(
                        div()
                            .id(("resize", s))
                            .w(theme::SPACE_4)
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
            .child(div().absolute().left_0().w(GUTTER).h_full().bg(theme.table_head).border_r_1().border_color(theme.border))
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
            .on_action(cx.listener(|this, _: &CopySqlInList, _, cx| this.copy_as(sql_in_list, cx)))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some((c, start_x, start_w)) = this.resizing {
                    this.widths[c] = (start_w + ev.position.x - start_x).max(MIN_WIDTH);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.resizing = None))
            // Re-render on scroll so the header and column window follow the offset.
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .when(!self.record, |d| d.child(self.render_header(window, cx)))
            .when(self.record && self.source.row_count() > 0, |d| d.child(self.render_record(cx)))
            .when(!self.record, |d| d.child(
                div().id("grid-body").flex_1().min_h_0().child(
                    uniform_list("grid-rows", self.source.row_count(), cx.processor(Self::render_rows))
                        .track_scroll(&self.scroll)
                        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                        .size_full(),
                )
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
                    }
                }),
            ))
            .when(self.inspecting && self.source.row_count() > 0, |d| {
                let theme = cx.theme();
                let (r, c) = (self.head.0, self.col_order[self.head.1]);
                let meta = &self.source.columns()[c];
                d.child(
                    v_flex()
                        .flex_none()
                        .h(px(180.))
                        .border_t_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .child(
                            div()
                                .px(theme::SPACE_8)
                                .py(theme::SPACE_4)
                                .text_size(theme::UI_SM)
                                .text_color(theme.muted_foreground)
                                .child(format!("{} · {} · row {}   (Esc closes)", meta.name, meta.data_type, r + 1)),
                        )
                        .child(
                            div()
                                .id("grid-inspector")
                                .flex_1()
                                .overflow_y_scroll()
                                .px(theme::SPACE_8)
                                .font_family(theme.mono_font_family.clone())
                                .text_size(theme::MONO)
                                .child(inspect_text(&self.row(r)[c])),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{header_model, HeaderModel, band_cols, selection_after_drop, cmp_values, copy_cols, display_widths, nav, pick, move_col, csv, inspect_text, json, markdown, pretty_json, sql_in_list, sql_insert, CellKind, ColumnMeta, MAX_CHARS, format_cell, mock, prefix_sums, px, tsv, visible_cols, Value};

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
        let cols = ["a", "b", "c"].map(|n| ColumnMeta { name: n.into(), data_type: "text".into(), is_pk: false }).to_vec();
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
    fn cells_format_by_type() {
        assert_eq!(format_cell(&Value::Null), ("NULL".into(), CellKind::Null));
        assert_eq!(format_cell(&Value::Int(-5)), ("-5".into(), CellKind::Number));
        assert_eq!(format_cell(&Value::Bool(true)).0, "✓");
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
        let cols = [ColumnMeta { name: "a".into(), data_type: "text".into(), is_pk: false }, ColumnMeta { name: "b".into(), data_type: "int".into(), is_pk: false }];
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
        ColumnMeta { name: name.into(), data_type: ty.into(), is_pk: pk }
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
}
