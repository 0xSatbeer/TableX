//! Added and deleted rows, pending until Submit like the cell edits.

use super::*;

/// `base` under `added` blank rows: new rows wait at the top until Submit.
struct WithAdded {
    base: Rc<dyn GridSource>,
    added: usize,
}

impl GridSource for WithAdded {
    fn row_count(&self) -> usize {
        self.added + self.base.row_count()
    }
    fn columns(&self) -> &[ColumnMeta] {
        self.base.columns()
    }
    fn row(&self, ix: usize) -> Row {
        match ix.checked_sub(self.added) {
            Some(ix) => self.base.row(ix),
            None => vec![Value::Null; self.columns().len()].into(),
        }
    }
}

impl Grid {
    /// Add row (Alt+Insert): a blank row at the top, selected.
    pub(super) fn add_row(&mut self, cx: &mut Context<Self>) {
        if self.read_only().is_some() || self.all.columns().is_empty() {
            return;
        }
        self.edits = self
            .edits
            .drain()
            .map(|((r, c), v)| ((r + 1, c), v))
            .collect();
        self.deleted = self.deleted.iter().map(|r| r + 1).collect();
        self.set_added(self.added + 1, cx);
        let col = self.display_hidden().iter().position(|h| !h).unwrap_or(0);
        (self.anchor, self.head) = ((0, col), (0, col));
    }

    /// Delete row (Ctrl+Delete): marks the selected rows; an added row just goes away.
    pub(super) fn delete_rows(&mut self, cx: &mut Context<Self>) {
        if self.read_only().is_some() {
            return;
        }
        let shown = self.source.row_count();
        let rows: BTreeSet<usize> = ranges_in_order(&self.extra, self.selection())
            .into_iter()
            .flat_map(|(rows, _)| rows)
            .filter(|&r| r < shown)
            .map(|r| self.abs_row(r))
            .collect();
        let mut added = self.added;
        // Highest first, so removing a row never moves one still to visit.
        for &r in rows.iter().rev() {
            if r >= self.added {
                self.deleted.insert(r);
                continue;
            }
            added -= 1;
            self.edits = self
                .edits
                .drain()
                .filter(|&((at, _), _)| at != r)
                .map(|((at, c), v)| ((if at > r { at - 1 } else { at }, c), v))
                .collect();
            self.deleted = self.deleted.iter().map(|d| d - 1).collect();
        }
        self.set_added(added, cx);
    }

    /// Edits, deletions or added rows wait for Submit.
    pub(super) fn has_changes(&self) -> bool {
        !self.edits.is_empty() || !self.deleted.is_empty() || self.added > 0
    }

    pub(super) fn set_added(&mut self, added: usize, cx: &mut Context<Self>) {
        self.added = added;
        self.all = if added == 0 {
            self.loaded.clone()
        } else {
            Rc::new(WithAdded {
                base: self.loaded.clone(),
                added,
            })
        };
        self.revision += 1;
        // The server sorted these rows; the header keeps showing which way.
        let sort = self.sort;
        self.set_page(self.page, self.page_size, cx);
        self.sort = sort;
    }
}
