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

/// Grid row height for a pane's mono font size: GRID_ROW at GRID_MONO, in proportion otherwise.
pub(super) fn row_height(mono: Pixels) -> Pixels {
    theme::GRID_ROW * (mono / theme::GRID_MONO)
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
        let cell_chars = rows
            .map(|r| format_cell(&self.row(r)[c]).0.chars().count())
            .max()
            .unwrap_or(0);
        // shortcut: advance estimated as 0.6em (mono); Inter headers can be under-sized, measure with the text system if that shows
        let advance = self.mono_px() * 0.6;
        // Key icons sit before the name; the sort control sits beside it.
        let (key, fks) = self.key_columns();
        let m = super::header::header_model(meta, key, fks);
        let icons = theme::space_16() * (m.pk && m.fk) as u8 as f32 + theme::space_16();
        // The header's padding is space_4 wider than a cell's.
        let header = self.header_px() * 0.6 * meta.name.chars().count() as f32
            + icons
            + theme::space_16()
            + theme::space_4();
        self.widths[c] =
            ((advance * cell_chars as f32).max(header) + theme::space_16() + theme::space_4())
                .max(MIN_WIDTH);
    }

    /// `grid_scroll` bench: one frame of scrolling down and right, wrapping at each end.
    pub fn bench_step(&mut self, cx: &mut Context<Self>) {
        let base = self.scroll.0.borrow().base_handle.clone();
        let (o, max) = (base.offset(), base.max_offset());
        let next = |v: Pixels, step: Pixels, max: Pixels| {
            if step - v > max {
                Pixels::ZERO
            } else {
                v - step
            }
        };
        base.set_offset(point(next(o.x, px(7.), max.x), next(o.y, px(37.), max.y)));
        cx.notify();
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

    /// Padding (bottom, right) that keeps the last row and column clear of the scrollbars. A bar
    /// shows only when its axis overflows, so an axis that fits gets none.
    pub(super) fn scrollbar_room(&self) -> (Pixels, Pixels) {
        // shortcut: reads last frame's bounds, so the frame right after a resize can be off; measure in prepaint if it shows.
        let size = self.scroll.0.borrow().base_handle.bounds().size;
        let bar = Scrollbar::width();
        let wide_by = self.gutter() + *self.offsets().last().unwrap_or(&Pixels::ZERO) - size.width;
        let tall_by = self.row_h() * self.source.row_count() as f32 - size.height;
        // Each bar's room eats into the other axis, so one overflow can tip the other.
        let wide = wide_by > Pixels::ZERO || (tall_by > Pixels::ZERO && wide_by > -bar);
        let tall = tall_by > Pixels::ZERO || (wide && tall_by > -bar);
        let room = |on: bool| if on { bar } else { Pixels::ZERO };
        (room(wide), room(tall))
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

    /// The header stands a little taller than a row, in slightly larger text.
    pub(super) fn header_h(&self) -> Pixels {
        self.row_h() * 1.2
    }

    pub(super) fn header_px(&self) -> Pixels {
        self.mono_px() * 1.1
    }

    pub(super) fn zoom(&mut self, step: theme::Zoom, cx: &mut Context<Self>) {
        self.mono = match step {
            theme::Zoom::Reset => theme::GRID_MONO,
            step => theme::zoom(self.mono, step),
        };
        cx.notify();
    }
}
