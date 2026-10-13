//! Copy formats (TSV, CSV, JSON, Markdown, SQL) and copying selected ranges.

use super::*;

/// Spreadsheet-pasteable TSV. NULL copies as an empty field; fields with tabs, newlines or
/// quotes are quoted with doubled quotes, which Sheets/Excel/LibreOffice all read back.
pub(super) fn tsv(_: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let mut out = String::new();
    for row in rows {
        let fields: Vec<String> = row
            .iter()
            .map(|v| match v {
                Value::Null => String::new(),
                v => {
                    let s = v.to_string();
                    if s.contains(['\t', '\n', '\r', '"']) {
                        format!("\"{}\"", s.replace('"', "\"\""))
                    } else {
                        s
                    }
                }
            })
            .collect();
        out.push_str(&fields.join("\t"));
        out.push('\n');
    }
    out
}

/// Source columns a copy of display positions `display` emits, in display order, hidden ones skipped.
pub(super) fn copy_cols(order: &[usize], hidden: &[bool], display: Range<usize>) -> Vec<usize> {
    order[display]
        .iter()
        .copied()
        .filter(|&s| !hidden[s])
        .collect()
}

/// `items` at the indices `ix`, in that order.
pub(super) fn pick<T: Clone>(items: &[T], ix: &[usize]) -> Vec<T> {
    ix.iter().map(|&i| items[i].clone()).collect()
}

/// Columns and rows of one selected block, hidden columns skipped, in display order.
pub(super) fn block_data<R: AsRef<[Value]>>(
    block: &Block,
    order: &[usize],
    hidden: &[bool],
    columns: &[ColumnMeta],
    row: &impl Fn(usize) -> R,
) -> (Vec<ColumnMeta>, Vec<Vec<Value>>) {
    let ix = copy_cols(order, hidden, block.1.clone());
    (
        pick(columns, &ix),
        block
            .0
            .clone()
            .map(|r| pick(row(r).as_ref(), &ix))
            .collect(),
    )
}

/// Every range in the order it was added: the extras, then the active one.
pub(super) fn ranges_in_order(extra: &[Block], active: Block) -> Vec<Block> {
    extra
        .iter()
        .cloned()
        .chain(std::iter::once(active))
        .collect()
}

/// One `format` block per range, separated by a blank line.
pub(super) fn copy_ranges<R: AsRef<[Value]>>(
    format: fn(&[ColumnMeta], &[Vec<Value>]) -> String,
    ranges: &[Block],
    order: &[usize],
    hidden: &[bool],
    columns: &[ColumnMeta],
    row: impl Fn(usize) -> R,
) -> String {
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
pub(super) fn copy_in_list<R: AsRef<[Value]>>(
    ranges: &[Block],
    order: &[usize],
    hidden: &[bool],
    columns: &[ColumnMeta],
    row: impl Fn(usize) -> R,
) -> String {
    let firsts: Vec<Vec<Value>> = ranges
        .iter()
        .flat_map(|r| block_data(r, order, hidden, columns, &row).1)
        .filter_map(|mut values| (!values.is_empty()).then(|| vec![values.swap_remove(0)]))
        .collect();
    sql_in_list(&[], &firsts)
}

pub(super) fn csv(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let field = |s: String| {
        if s.contains([',', '\n', '\r', '"']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s
        }
    };
    let header = cols.iter().map(|c| field(c.name.to_string()));
    let mut out = header.collect::<Vec<_>>().join(",") + "\n";
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .map(|v| {
                if *v == Value::Null {
                    String::new()
                } else {
                    field(v.to_string())
                }
            })
            .collect();
        out += &(line.join(",") + "\n");
    }
    out
}

pub(super) fn json_string(s: &str) -> String {
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

pub(super) fn json_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) | Value::Int(_) => v.to_string(),
        Value::Float(x) if x.is_finite() => v.to_string(),
        Value::Json(s) => s.to_string(),
        v => json_string(&v.to_string()),
    }
}

/// Array of objects keyed by column name.
pub(super) fn json(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let objects: Vec<String> = rows
        .iter()
        .map(|row| {
            let fields: Vec<String> = cols
                .iter()
                .zip(row)
                .map(|(c, v)| format!("{}: {}", json_string(&c.name), json_value(v)))
                .collect();
            format!("  {{{}}}", fields.join(", "))
        })
        .collect();
    format!("[\n{}\n]\n", objects.join(",\n"))
}

pub(super) fn markdown(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let cell = |s: String| s.replace('|', "\\|").replace('\n', " ");
    let line = |cells: Vec<String>| format!("| {} |\n", cells.join(" | "));
    let mut out = line(cols.iter().map(|c| cell(c.name.to_string())).collect());
    out += &line(cols.iter().map(|_| "---".to_string()).collect());
    for row in rows {
        out += &line(row.iter().map(|v| cell(v.to_string())).collect());
    }
    out
}

pub(super) fn sql_literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        Value::Int(_) => v.to_string(),
        Value::Float(x) if x.is_finite() => v.to_string(),
        v => format!("'{}'", v.to_string().replace('\'', "''")),
    }
}

// ponytail: the grid does not know its table name yet; QX-011 passes the real one
pub(super) fn sql_insert(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let names: Vec<&str> = cols.iter().map(|c| c.name.as_ref()).collect();
    rows.iter()
        .map(|row| {
            let values: Vec<String> = row.iter().map(sql_literal).collect();
            format!(
                "INSERT INTO table_name ({}) VALUES ({});\n",
                names.join(", "),
                values.join(", ")
            )
        })
        .collect()
}

/// Values of the first selected column as `(a, b, c)`.
pub(super) fn sql_in_list(_: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let values: Vec<String> = rows
        .iter()
        .filter_map(|row| row.first())
        .map(sql_literal)
        .collect();
    format!("({})", values.join(", "))
}

impl Grid {
    pub(super) fn copy_as(
        &self,
        format: fn(&[ColumnMeta], &[Vec<Value>]) -> String,
        cx: &mut Context<Self>,
    ) {
        let ranges = ranges_in_order(&self.extra, self.selection());
        let text = copy_ranges(
            format,
            &ranges,
            &self.col_order,
            &self.hidden,
            self.source.columns(),
            |r| self.row(r),
        );
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    pub(super) fn copy_in_list(&self, cx: &mut Context<Self>) {
        let ranges = ranges_in_order(&self.extra, self.selection());
        let text = copy_in_list(
            &ranges,
            &self.col_order,
            &self.hidden,
            self.source.columns(),
            |r| self.row(r),
        );
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}
