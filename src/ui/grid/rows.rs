//! Rendering the visible rows, and the transposed one-row record view.

use super::*;

/// The (top, bottom) runs of a column line from 0 to `bottom`, broken for one `row_h` at each
/// row top in `gaps`.
pub(super) fn line_spans(
    mut gaps: Vec<Pixels>,
    row_h: Pixels,
    bottom: Pixels,
) -> Vec<(Pixels, Pixels)> {
    gaps.sort();
    let mut top = Pixels::ZERO;
    let mut spans = Vec::new();
    for end in gaps.into_iter().chain([bottom]) {
        let end = end.min(bottom);
        if end > top {
            spans.push((top, end));
        }
        top = top.max(end + row_h);
    }
    spans
}

impl Grid {
    pub(super) fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let mut picker = self.render_picker(cx);
        let mut problem = self
            .editing
            .as_ref()
            .and_then(|e| Some((e.cell, e.error.clone()?)))
            .or_else(|| self.notice.clone());
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
        let failed_row = self.failed_row();
        let text_editor = match self.editing.as_ref().map(|e| &e.editor) {
            Some(Editor::Text(input)) => Some(input.clone()),
            _ => None,
        };
        range
            .map(|r| {
                let (row, at) = (self.row(r), self.abs_row(r));
                let added = at < self.added;
                let cells = cols
                    .clone()
                    .map(|c| (c, self.col_order[c]))
                    .filter(|&(_, s)| !self.hidden[s])
                    .map(|(c, s)| {
                        let (text, kind) = format_cell(&row[s]);
                        let selected = self.is_selected(r, c);
                        div()
                            .absolute()
                            .left(self.gutter() + offsets[c])
                            .w(self.widths[s])
                            .h_full()
                            .px(theme::space_6())
                            .flex()
                            .items_center()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .when(
                                aligns_right(&kind, &self.source.columns()[s].data_type),
                                |d| d.justify_end(),
                            )
                            .when(self.edits.contains_key(&(at, s)), |d| {
                                d.bg(theme::grid_edited(theme))
                            })
                            .when(selected, |d| d.bg(fill))
                            .when_some(cursor.filter(|_| self.head == (r, c)), |d, color| {
                                d.border_1().border_color(color)
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                    this.cell_down((r, c), ev, window, cx)
                                }),
                            )
                            // Dragging across cells moves the head; the anchor stays where the drag began.
                            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                                if this.drag_selecting && ev.dragging() && this.head != (r, c) {
                                    this.select((r, c), true, cx);
                                }
                            }))
                            .when_some(
                                text_editor.clone().filter(|_| editing_cell == Some((r, c))),
                                |d, input| {
                                    d.px_0().child(
                                        Input::new(&input)
                                            .small()
                                            .bordered(false)
                                            .w_full()
                                            .h_full()
                                            .font_family(theme.mono_font_family.clone())
                                            .text_size(self.mono_px()),
                                    )
                                },
                            )
                            .when_some(
                                picker.take_if(|_| editing_cell == Some((r, c))),
                                |d, picker| d.child(picker),
                            )
                            .when_some(problem.take_if(|(at, _)| *at == (r, c)), |d, (_, why)| {
                                d.border_1()
                                    .border_color(theme.danger)
                                    .child(self.render_problem(why, cx))
                            })
                            .when(
                                (editing_cell != Some((r, c)) || text_editor.is_none())
                                    // Left blank, a new row's cell takes the column default.
                                    && !(added && !self.edits.contains_key(&(at, s))),
                                |d| {
                                    d.when(kind == CellKind::Null, |d| {
                                        d.text_color(theme::grid_null(theme))
                                    })
                                    .when(kind == CellKind::Bool(false), |d| {
                                        d.text_color(theme.muted_foreground)
                                    })
                                    .child(text)
                                },
                            )
                    });
                div()
                    .relative()
                    .w(self.gutter() + total)
                    .h(self.row_h())
                    .font_family(theme.mono_font_family.clone())
                    .text_size(self.mono_px())
                    .text_color(
                        theme
                            .highlight_theme
                            .style
                            .editor_foreground
                            .unwrap_or(theme.foreground),
                    )
                    .bg(if r % 2 == 1 {
                        theme.table_even
                    } else {
                        theme.table
                    })
                    .when(added, |d| d.bg(theme::grid_inserted(theme)))
                    .when(self.deleted.contains(&at), |d| {
                        d.debug_selector(move || format!("deleted-row:{at}"))
                            .bg(theme::grid_deleted(theme))
                            .line_through()
                    })
                    .when(failed_row == Some(at), |d| {
                        d.debug_selector(move || format!("failed-row:{at}"))
                            .bg(theme.danger.opacity(0.14))
                    })
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
                            .child(if added {
                                "+".to_string()
                            } else {
                                (self.source.start + r + 1)
                                    .saturating_sub(self.added)
                                    .to_string()
                            }),
                    )
            })
            .collect()
    }

    /// Column lines over the rows: one quad per visible column instead of a border per cell,
    /// which halved the cost of a scrolled frame. A line breaks where the cursor or a refused
    /// cell draws its own outline, so that outline stays whole.
    pub(super) fn render_col_lines(&self, window: &Window, cx: &App) -> impl IntoElement {
        let offsets = self.offsets();
        let (scroll_x, view) = self.viewport(window);
        let scroll_y = self.scroll.0.borrow().base_handle.offset().y;
        let (gutter, row_h) = (self.gutter(), self.row_h());
        let (bottom_room, right_room) = self.scrollbar_room();
        let outlined: Vec<(usize, usize)> = self
            .focus
            .is_focused(window)
            .then_some(self.head)
            .into_iter()
            .chain(
                self.editing
                    .as_ref()
                    .filter(|e| e.error.is_some())
                    .map(|e| e.cell),
            )
            .chain(self.notice.as_ref().map(|(at, _)| *at))
            .collect();
        let lines: Vec<(Pixels, Vec<usize>)> = visible_cols(&offsets, scroll_x, view, OVERSCAN)
            .filter(|&c| !self.hidden[self.col_order[c]])
            .map(|c| {
                let gaps = outlined.iter().filter(|o| o.1 == c).map(|o| o.0).collect();
                (gutter + offsets[c + 1] - scroll_x - px(1.), gaps)
            })
            .collect();
        let rows_bottom = row_h * self.source.row_count() as f32 + scroll_y;
        let color = cx.theme().table_row_border;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let bottom = rows_bottom.min(bounds.size.height - bottom_room);
                let right = bounds.size.width - right_room;
                for (x, gaps) in lines.iter().filter(|(x, _)| *x >= gutter && *x < right) {
                    let gaps = gaps.iter().map(|&r| row_h * r as f32 + scroll_y);
                    for (top, end) in line_spans(gaps.collect(), row_h, bottom) {
                        let origin = bounds.origin + point(*x, top);
                        window
                            .paint_quad(fill(Bounds::new(origin, size(px(1.), end - top)), color));
                    }
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// One row transposed: column name, type, full value.
    pub(super) fn render_record(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(format!(
                        "Row {} of {}   (Ctrl+Shift+R back to grid, ↑↓ change row)",
                        self.source.start + r + 1,
                        self.all.row_count()
                    )),
            )
            .children(
                self.source
                    .columns()
                    .iter()
                    .zip(row.iter())
                    .enumerate()
                    .map(|(c, (meta, value))| {
                        let (text, kind) = format_cell(value);
                        h_flex()
                            .min_h(self.row_h())
                            .px(theme::space_8())
                            .gap(theme::space_8())
                            .border_b_1()
                            .border_color(theme.table_row_border)
                            .when(c % 2 == 1, |d| d.bg(theme.table_even))
                            .child(
                                div()
                                    .flex_none()
                                    .w(name_w)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(meta.name.to_string()),
                            )
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
                                    .when(kind == CellKind::Null, |d| {
                                        d.text_color(theme::grid_null(theme))
                                    })
                                    .child(if kind == CellKind::Null {
                                        text
                                    } else {
                                        inspect_text(value)
                                    }),
                            )
                    }),
            )
    }
}
