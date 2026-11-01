//! Pasting and setting values over the selected cells.

use super::*;

/// Every cell of `ranges` in a visible column, once, as (display row, display column).
pub(super) fn fill_targets(ranges: &[Block], hidden: &[bool]) -> Vec<(usize, usize)> {
    let mut cells: Vec<_> = ranges
        .iter()
        .flat_map(|(rs, cs)| {
            rs.clone()
                .flat_map(move |r| cs.clone().map(move |c| (r, c)))
        })
        .filter(|&(_, c)| !hidden[c])
        .collect();
    cells.sort_unstable();
    cells.dedup();
    cells
}

/// Where a pasted `block` lands: one value fills every selected cell, a bigger block goes from
/// `top_left` over visible columns, clipped at the last row and column.
pub(super) fn paste_targets(
    block: &[Vec<String>],
    ranges: &[Block],
    top_left: (usize, usize),
    rows: usize,
    hidden: &[bool],
) -> Vec<((usize, usize), String)> {
    if let [row] = block
        && let [value] = row.as_slice()
    {
        return fill_targets(ranges, hidden)
            .into_iter()
            .map(|cell| (cell, value.clone()))
            .collect();
    }
    let cols: Vec<usize> = (top_left.1..hidden.len()).filter(|&c| !hidden[c]).collect();
    block
        .iter()
        .zip(top_left.0..rows)
        .flat_map(|(fields, r)| {
            fields
                .iter()
                .zip(&cols)
                .map(move |(v, &c)| ((r, c), v.clone()))
        })
        .collect()
}

/// Whether an edit puts back the loaded value. Postgres `numeric` loads as text but edits as a float.
pub(super) fn unchanged(was: &Value, now: &Value) -> bool {
    was == now || matches!((was, now), (Value::Text(t), Value::Float(f)) if t.parse() == Ok(*f))
}

impl Grid {
    /// Stores `cells` (display row, display column, value) as pending edits.
    pub(super) fn set_values(
        &mut self,
        cells: Vec<((usize, usize), Value)>,
        cx: &mut Context<Self>,
    ) {
        if self.read_only().is_some() {
            return;
        }
        self.notice = None;
        for ((r, c), value) in cells {
            let (at, s) = (self.abs_row(r), self.col_order[c]);
            // Back to the loaded value: no longer an edit. A new row's cell keeps it, since blank means DEFAULT.
            if at >= self.added && unchanged(&self.all.row(at)[s], &value) {
                self.edits.remove(&(at, s));
            } else {
                self.edits.insert((at, s), value);
            }
        }
        self.revision += 1;
        cx.notify();
    }

    /// Ctrl+V: clipboard TSV into the selection; a multi-cell paste then selects what it wrote.
    /// One field the column can't hold stops the whole paste, and that cell shows why.
    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        let ranges = ranges_in_order(&self.extra, self.selection());
        let (rows, cols) = self.selection();
        let block = parse_tsv(&text);
        let cells = paste_targets(
            &block,
            &ranges,
            (rows.start, cols.start),
            self.source.row_count(),
            &self.display_hidden(),
        );
        let values: Result<Vec<_>, _> = cells
            .iter()
            .map(|&(cell, ref text)| {
                parse_value(text, &self.source.columns()[self.col_order[cell.1]])
                    .map(|v| (cell, v))
                    .map_err(|why| (cell, why))
            })
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
    pub(super) fn selected_cells(&self) -> Vec<(usize, usize)> {
        fill_targets(
            &ranges_in_order(&self.extra, self.selection()),
            &self.display_hidden(),
        )
    }
}
