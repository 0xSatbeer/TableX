//! Copy formats (TSV, CSV, JSON, Markdown, SQL) and copying selected ranges.

use super::*;

pub(super) use crate::export::sql_literal;
use crate::export::{Format, Writer};

/// `rows` as `format`, with its header unless `header` is false (TSV copies paste as bare cells).
fn text(format: Format, header: bool, cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    let mut out = Writer::new(format, Vec::new(), cols.to_vec(), "table_name");
    if header {
        out.header().expect("writing to memory");
    }
    out.rows(rows).expect("writing to memory");
    String::from_utf8(out.finish().expect("writing to memory")).expect("values are UTF-8")
}

/// Spreadsheet-pasteable TSV without a header. NULL copies as an empty field.
pub(super) fn tsv(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    text(Format::Tsv, false, cols, rows)
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
    text(Format::Csv, true, cols, rows)
}

/// Array of objects keyed by column name.
pub(super) fn json(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    text(Format::Json, true, cols, rows)
}

pub(super) fn markdown(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    text(Format::Markdown, true, cols, rows)
}

// ponytail: the grid does not know its table name yet; QX-011 passes the real one
pub(super) fn sql_insert(cols: &[ColumnMeta], rows: &[Vec<Value>]) -> String {
    text(Format::SqlInsert, true, cols, rows)
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
