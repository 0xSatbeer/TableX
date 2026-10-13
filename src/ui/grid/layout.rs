//! Column widths, the visible column window, row and gutter sizes, and zoom.

use super::*;

pub(super) const DEFAULT_WIDTH: Pixels = px(160.);

pub(super) const MIN_WIDTH: Pixels = px(40.);

/// Opening width cap: one long JSON column shouldn't push the rest off screen.
pub(super) const MAX_OPEN_WIDTH: Pixels = px(320.);

pub(super) const OVERSCAN: usize = 2;

pub(super) const AUTOFIT_SAMPLE: usize = 60;

/// `offsets[i]` is the left edge of column `i`; the last entry is the total width.
pub(super) fn prefix_sums(widths: &[Pixels]) -> Vec<Pixels> {
    let mut acc = Pixels::ZERO;
    std::iter::once(Pixels::ZERO)
        .chain(widths.iter().map(|w| {
            acc += *w;
            acc
        }))
        .collect()
}

/// Widths in display order from per-source `widths` and `hidden`; hidden columns are zero wide.
pub(super) fn display_widths(order: &[usize], widths: &[Pixels], hidden: &[bool]) -> Vec<Pixels> {
    order
        .iter()
        .map(|&s| if hidden[s] { Pixels::ZERO } else { widths[s] })
        .collect()
}

/// Columns intersecting `[scroll_x, scroll_x + viewport)`, widened by `overscan` each side.
pub(super) fn visible_cols(
    offsets: &[Pixels],
    scroll_x: Pixels,
    viewport: Pixels,
    overscan: usize,
) -> Range<usize> {
    let n = offsets.len() - 1;
    let start = offsets[1..].partition_point(|&end| end <= scroll_x);
    let end = offsets[..n].partition_point(|&left| left < scroll_x + viewport);
    start.saturating_sub(overscan)..(end + overscan).min(n)
}

/// Grid row height for a pane's mono font size: GRID_ROW at MONO, in proportion otherwise.
pub(super) fn row_height(mono: Pixels) -> Pixels {
    theme::GRID_ROW * (mono / theme::MONO)
}

/// Width of the row-number gutter: monospace digits plus padding.
pub(super) fn gutter_width(rows: usize, mono: Pixels) -> Pixels {
    mono * 0.6 * gutter_digits(rows) as f32 + theme::space_12()
}

/// Digits the row-number gutter must fit: the row count's, but never fewer than 3.
pub(super) fn gutter_digits(rows: usize) -> usize {
    (rows.max(1).ilog10() as usize + 1).max(3)
}

impl Grid {
    /// Width that fits the header and the rows currently in view.
    pub(super) fn autofit(&mut self, c: usize) {
        let top = (-self.scroll.0.borrow().base_handle.offset().y / self.row_h()) as usize;
        let rows = top..(top + AUTOFIT_SAMPLE).min(self.source.row_count());
        let meta = &self.source.columns()[c];
        let chars = rows
            .map(|r| format_cell(&self.row(r)[c]).0.chars().count())
            .chain([meta.name.chars().count(), meta.data_type.chars().count()])
            .max()
            .unwrap_or(0);
        // ponytail: advance estimated as 0.6em (mono); Inter headers can be under-sized, measure with the text system if that shows
        self.widths[c] =
            (self.mono_px() * 0.6 * chars as f32 + theme::space_16() + theme::space_4())
                .max(MIN_WIDTH);
    }

    pub(super) fn gutter(&self) -> Pixels {
        gutter_width(self.all.row_count(), self.mono_px())
    }

    /// Scroll so the head cell is in view, both axes.
    pub(super) fn reveal_head(&self) {
        self.scroll
            .scroll_to_item(self.head.0, ScrollStrategy::Nearest);
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

    pub(super) fn display_hidden(&self) -> Vec<bool> {
        self.col_order.iter().map(|&s| self.hidden[s]).collect()
    }

    /// Display-order left edges, hidden columns at zero width.
    pub(super) fn offsets(&self) -> Vec<Pixels> {
        prefix_sums(&display_widths(&self.col_order, &self.widths, &self.hidden))
    }

    /// Horizontal scroll position and viewport width of the cell area.
    pub(super) fn viewport(&self, window: &Window) -> (Pixels, Pixels) {
        let base = self.scroll.0.borrow().base_handle.clone();
        let width = base.bounds().size.width;
        // Bounds are zero before the first layout; the window width over-covers, which is fine.
        let width = if width > Pixels::ZERO {
            width
        } else {
            window.viewport_size().width
        };
        (-base.offset().x, width - self.gutter())
    }

    #[cfg(test)]
    pub fn mono(&self) -> Pixels {
        self.mono
    }

    /// The pane's mono font size at the current UI scale.
    pub(super) fn mono_px(&self) -> Pixels {
        theme::scaled(self.mono)
    }

    pub(super) fn row_h(&self) -> Pixels {
        theme::scaled(row_height(self.mono))
    }

    pub(super) fn zoom(&mut self, step: theme::Zoom, cx: &mut Context<Self>) {
        self.mono = theme::zoom(self.mono, step);
        cx.notify();
    }
}
