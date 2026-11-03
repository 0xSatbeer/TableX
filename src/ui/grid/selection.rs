//! Selecting cells, rows, columns and ranges, and keyboard navigation.

use super::*;

/// Header drag: moves display position `from` to just before display position `before`
/// (`before == order.len()` is the end). Returns whether the order changed.
pub(super) fn move_col(order: &mut Vec<usize>, from: usize, before: usize) -> bool {
    if before == from || before == from + 1 {
        return false;
    }
    let col = order.remove(from);
    order.insert(if before > from { before - 1 } else { before }, col);
    true
}

/// Head cell after a navigation key, or `None` for other keys. `hidden` is in display order;
/// every horizontal move skips hidden columns.
pub(super) fn nav(
    key: &str,
    secondary: bool,
    shift: bool,
    (r, c): (usize, usize),
    rows: usize,
    hidden: &[bool],
    page: usize,
) -> Option<(usize, usize)> {
    let cols = hidden.len();
    // Nearest visible column left or right of `c`, or `c` at the edge.
    let step = |c: usize, right: bool| {
        let visible = |&i: &usize| !hidden[i];
        if right {
            (c + 1..cols).find(visible)
        } else {
            (0..c).rev().find(visible)
        }
        .unwrap_or(c)
    };
    let first = if hidden[0] { step(0, true) } else { 0 };
    let last = if hidden[cols - 1] {
        step(cols - 1, false)
    } else {
        cols - 1
    };
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
pub(super) fn band_cols(col: Option<usize>, hidden: &[bool]) -> (usize, usize) {
    let edge = |key| nav(key, false, false, (0, 0), 1, hidden, 0).map_or(0, |(_, c)| c);
    col.map_or_else(|| (edge("home"), edge("end")), |c| (c, c))
}

/// Anchor, head and extra ranges, in display positions.
pub(super) type Selection = (
    (usize, usize),
    (usize, usize),
    Vec<(Range<usize>, Range<usize>)>,
);

/// Selection after a header drop: a real move (`Some(first visible display column)`) resets it
/// to row 0 of that column, a no-op drop (`None`) keeps it.
pub(super) fn selection_after_drop(reset_col: Option<usize>, selection: Selection) -> Selection {
    reset_col.map_or(selection, |c| ((0, c), (0, c), Vec::new()))
}

/// A selected block: row range, display column range.
pub(super) type Block = (Range<usize>, Range<usize>);

impl Grid {
    /// Cell mouse-down: click selects (or extends, or adds a range); a double-click edits just that
    /// cell (Enter or F2 edits a whole selection).
    pub(super) fn cell_down(
        &mut self,
        cell: (usize, usize),
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The grid's own focus tracking would take focus back from the editor on this press.
        if self.editing.as_ref().is_some_and(|e| e.cell == cell) {
            return window.prevent_default();
        }
        // Text commits now rather than on the blur, which lands a frame later; a list or calendar cancels.
        let typed = match &self.editing {
            Some(Editing {
                editor: Editor::Text(input),
                ..
            }) => Some(input.clone()),
            _ => None,
        };
        if !self.stop_editing(typed.is_some(), window, cx) {
            // Invalid text stays in its cell, focused, until it is fixed or Esc drops it.
            window.prevent_default();
            return typed
                .unwrap()
                .update(cx, |input, cx| input.focus(window, cx));
        }
        window.focus(&self.focus, cx);
        let plain = !ev.modifiers.secondary() && !ev.modifiers.shift;
        if ev.click_count == 2 && plain {
            window.prevent_default();
            return self.start_editing(cell, window, cx);
        }
        if ev.modifiers.secondary() {
            self.add_range(cell, cx);
        } else {
            self.select(cell, ev.modifiers.shift, cx);
        }
        self.drag_selecting = true;
    }

    pub(super) fn selection(&self) -> (Range<usize>, Range<usize>) {
        let (a, h) = (self.anchor, self.head);
        (
            a.0.min(h.0)..a.0.max(h.0) + 1,
            a.1.min(h.1)..a.1.max(h.1) + 1,
        )
    }

    pub fn selection_key(&self) -> SelectionKey {
        let (rows, cols) = self.selection();
        let cols = cols
            .filter_map(|c| self.col_order.get(c).copied())
            .filter(|&s| !self.hidden[s])
            .collect();
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
        crate::ui::statusbar::aggregate(rows.len(), cols.len(), values)
    }

    pub(super) fn select(&mut self, cell: (usize, usize), extend: bool, cx: &mut Context<Self>) {
        self.notice = None;
        self.head = cell;
        if !extend {
            self.anchor = cell;
            self.extra.clear();
        }
        cx.notify();
    }

    /// Header drop: display column `from` moves before `before`. A real move clears the selection.
    pub(super) fn drop_col(&mut self, from: usize, before: usize, cx: &mut Context<Self>) {
        let moved = move_col(&mut self.col_order, from, before);
        let reset_col = moved.then(|| band_cols(None, &self.display_hidden()).0);
        let selection = (self.anchor, self.head, std::mem::take(&mut self.extra));
        (self.anchor, self.head, self.extra) = selection_after_drop(reset_col, selection);
        cx.notify();
    }

    /// Ctrl+click: keep the current range and start a new one at `cell`.
    pub(super) fn add_range(&mut self, cell: (usize, usize), cx: &mut Context<Self>) {
        self.extra.push(self.selection());
        (self.anchor, self.head) = (cell, cell);
        cx.notify();
    }

    pub(super) fn is_selected(&self, r: usize, c: usize) -> bool {
        let (rows, cols) = self.selection();
        std::iter::once((&rows, &cols))
            .chain(self.extra.iter().map(|(rs, cs)| (rs, cs)))
            .any(|(rs, cs)| rs.contains(&r) && cs.contains(&c))
    }

    /// Whole row (gutter click) or whole column (header click); shift extends from the anchor.
    pub(super) fn select_band(
        &mut self,
        row: Option<usize>,
        col: Option<usize>,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        let (first_col, last_col) = band_cols(col, &self.display_hidden());
        let head = (
            row.unwrap_or(self.source.row_count().saturating_sub(1)),
            last_col,
        );
        if !extend {
            self.anchor = (row.unwrap_or(0), first_col);
            self.extra.clear();
        }
        self.head = head;
        cx.notify();
    }

    pub(super) fn on_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self.source.row_count();
        if self.on_picker_key(&ev.keystroke.key, window, cx) {
            return cx.stop_propagation();
        }
        if ev.keystroke.key == "f5" && self.is_live() {
            self.load(cx);
            return cx.stop_propagation();
        }
        if self.editing.is_some() && ev.keystroke.key == "escape" {
            self.stop_editing(false, window, cx);
            return;
        }
        let m = ev.keystroke.modifiers;
        if self.is_live() && self.focus.is_focused(window) {
            match ev.keystroke.key.as_str() {
                "insert" if m.alt => return self.add_row(cx),
                "delete" if m.secondary() => return self.delete_rows(cx),
                _ => {}
            }
        }
        // Keys typed into the filter inputs bubble up here; they are not grid navigation.
        if rows == 0 || self.widths.is_empty() || !self.focus.is_focused(window) {
            return;
        }
        match ev.keystroke.key.as_str() {
            "c" if m.secondary() => return self.copy_as(if self.copy_csv { csv } else { tsv }, cx),
            "v" if m.secondary() => return self.paste(cx),
            "enter" if m.secondary() && self.is_live() => return self.submit(window, cx),
            "enter" | "f2" if !m.secondary() && !m.shift => {
                return self.start_editing(self.head, window, cx);
            }
            "r" if m.secondary() && m.shift => {
                self.record = !self.record;
                return cx.notify();
            }
            _ => {}
        }
        let page =
            (self.scroll.0.borrow().base_handle.bounds().size.height / self.row_h()) as usize;
        let Some(next) = nav(
            &ev.keystroke.key,
            m.secondary(),
            m.shift,
            self.head,
            rows,
            &self.display_hidden(),
            page,
        ) else {
            return;
        };
        self.select(next, m.shift && ev.keystroke.key != "tab", cx);
        self.reveal_head();
        cx.stop_propagation();
    }
}
