//! What the grid shows: the row source trait, its sources, and paging.

use super::*;

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
pub(super) struct Page {
    all: Rc<dyn GridSource>,
    pub(super) start: usize,
    pub(super) len: usize,
}

impl Page {
    pub(super) fn new(all: Rc<dyn GridSource>, page: usize, size: usize) -> Self {
        let start = (page * size).min(all.row_count());
        Self {
            len: size.min(all.row_count() - start),
            all,
            start,
        }
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

pub(super) const PAGE_SIZES: [usize; 5] = [100, 500, 1_000, 5_000, 10_000];

/// Pages needed for `rows` at `size` per page; an empty result still has one.
pub(super) fn page_count(rows: usize, size: usize) -> usize {
    rows.div_ceil(size).max(1)
}

/// `1234567` as "1,234,567".
pub(super) fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

impl Grid {
    /// Shows page `page` at `size` rows per page; selection and sort start over.
    pub(super) fn set_page(&mut self, page: usize, size: usize, cx: &mut Context<Self>) {
        (self.page_size, self.page) = (size, page.min(page_count(self.all.row_count(), size) - 1));
        self.source = Rc::new(Page::new(self.all.clone(), self.page, size));
        (self.anchor, self.head, self.sort, self.order) = ((0, 0), (0, 0), None, None);
        self.extra.clear();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Row at display position `r`, after sorting.
    pub(super) fn row(&self, r: usize) -> Row {
        self.page_row(self.order.as_ref().map_or(r, |o| o[r]))
    }

    /// Page row `ix` with pending edits over it.
    pub(super) fn page_row(&self, ix: usize) -> Row {
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
    pub(super) fn abs_row(&self, r: usize) -> usize {
        self.source.start + self.order.as_ref().map_or(r, |o| o[r])
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
}
