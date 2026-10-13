//! Header cells: name, type, sort control, and column dragging.

use super::*;

/// Above this the sort belongs to the query (QX-011 passes it to ORDER BY), not the client.
pub(super) const CLIENT_SORT_MAX: usize = 100_000;

/// What a header cell shows for one source column.
#[derive(Debug, PartialEq)]
pub(super) struct HeaderModel {
    pub(super) name: String,
    pub(super) data_type: String,
    pub(super) pk: bool,
}

pub(super) fn header_model(c: &ColumnMeta) -> HeaderModel {
    HeaderModel {
        name: c.name.to_string(),
        data_type: c.data_type.to_string(),
        pk: c.is_pk,
    }
}

/// Header height: two lines (name, type) plus vertical padding.
pub(super) fn header_height(line: Pixels, pad: Pixels) -> Pixels {
    line * 2. + pad
}

/// Whether the header sort control works for a source of this size.
#[derive(Debug, PartialEq)]
pub(super) enum SortAvail {
    Enabled,
    Disabled(&'static str),
}

pub(super) fn sort_availability(rows: usize) -> SortAvail {
    if rows <= CLIENT_SORT_MAX {
        SortAvail::Enabled
    } else {
        SortAvail::Disabled("Too many rows to sort here; use ORDER BY")
    }
}

/// A header cell being dragged to a new display position.
#[derive(Clone)]
pub(super) struct DragColumn {
    grid: EntityId,
    from: usize,
    name: SharedString,
}

impl Render for DragColumn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(theme::space_8())
            .py(theme::space_4())
            .rounded(theme::radius_sm())
            .border_1()
            .border_color(theme.border)
            .bg(theme.table_head)
            .text_size(theme::ui_sm())
            .text_color(theme.foreground)
            .child(self.name.clone())
    }
}

impl Grid {
    /// Header sort button: ascending, descending, off.
    pub(super) fn cycle_sort(&mut self, c: usize, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            Some((sc, true)) if sc == c => Some((c, false)),
            Some((sc, false)) if sc == c => None,
            _ => Some((c, true)),
        };
        self.order = self.sort.map(|(c, asc)| {
            let keys: Vec<Value> = (0..self.source.row_count())
                .map(|r| self.page_row(r)[c].clone())
                .collect();
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

    pub(super) fn render_header(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let offsets = self.offsets();
        let (scroll_x, view) = self.viewport(window);
        let cols = visible_cols(&offsets, scroll_x, view, OVERSCAN);
        let columns = self.source.columns();
        div()
            .relative()
            .flex_none()
            .h(header_height(theme::badge_line(), theme::space_8()))
            .w_full()
            .overflow_hidden()
            .bg(theme.table_head)
            .border_b_1()
            .border_color(theme.border)
            .text_color(theme.table_head_foreground)
            .children(
                cols.map(|c| (c, self.col_order[c]))
                    .filter(|&(_, s)| !self.hidden[s])
                    .map(|(c, s)| {
                        h_flex()
                            .id(("col", s))
                            .absolute()
                            .left(self.gutter() + offsets[c] - scroll_x)
                            .w(self.widths[s])
                            .h_full()
                            .border_r_1()
                            .border_color(theme.table_row_border)
                            // Drag a header and drop it on another: the column moves before that one.
                            .on_drag(
                                DragColumn {
                                    grid: cx.entity_id(),
                                    from: c,
                                    name: columns[s].name.clone().into(),
                                },
                                |drag, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.new(|_| drag.clone())
                                },
                            )
                            .drag_over::<DragColumn>(|d, _, _, cx| {
                                d.border_l_2().border_color(cx.theme().drag_border)
                            })
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
                                        cx.listener(
                                            move |this, ev: &MouseDownEvent, window, cx| {
                                                window.focus(&this.focus, cx);
                                                this.select_band(
                                                    None,
                                                    Some(c),
                                                    ev.modifiers.shift,
                                                    cx,
                                                );
                                            },
                                        ),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(move |this, _, _, _| this.menu_col = s),
                                    )
                                    .px(theme::space_8())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child({
                                        let m = header_model(&columns[s]);
                                        h_flex()
                                            .gap(theme::space_4())
                                            .h(theme::badge_line())
                                            .text_size(theme::ui_sm())
                                            .text_color(theme.foreground)
                                            .when(m.pk, |d| {
                                                d.child(
                                                    Icon::new(IconName::KeyRound)
                                                        .size_3()
                                                        .text_color(theme.muted_foreground),
                                                )
                                            })
                                            .child({
                                                let name = SharedString::from(m.name);
                                                div()
                                                    .debug_selector(|| format!("header-name-{s}"))
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(theme::truncating(
                                                        ("header-name", s),
                                                        StyledText::new(name.clone()),
                                                        name,
                                                    ))
                                            })
                                    })
                                    .child(
                                        div()
                                            .debug_selector(|| format!("header-type-{s}"))
                                            .h(theme::badge_line())
                                            .font_family(theme.mono_font_family.clone())
                                            .text_size(theme::ui_sm())
                                            .text_color(theme.muted_foreground)
                                            .truncate()
                                            .child({
                                                let data_type = SharedString::from(
                                                    header_model(&columns[s]).data_type,
                                                );
                                                theme::truncating(
                                                    ("header-type", s),
                                                    StyledText::new(data_type.clone()),
                                                    data_type,
                                                )
                                            }),
                                    )
                                    .context_menu({
                                        let focus = self.focus.clone();
                                        move |menu, _, _| {
                                            menu.action_context(focus.clone())
                                                .menu("Hide column", Box::new(HideColumn))
                                                .menu("Show all columns", Box::new(ShowAllColumns))
                                        }
                                    }),
                            )
                            .child({
                                let avail = sort_availability(self.source.row_count());
                                let glyph = match self.sort {
                                    Some((sc, true)) if sc == s => "↑",
                                    Some((sc, false)) if sc == s => "↓",
                                    _ => "↕",
                                };
                                div()
                                    .id(("sort", s))
                                    .px(theme::space_4())
                                    .text_size(theme::ui_sm())
                                    // Drag must not start from the sort control.
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .map(|d| match avail {
                                        SortAvail::Enabled => d
                                            .when(glyph == "↕", |d| {
                                                d.text_color(theme::text_disabled(theme))
                                            })
                                            .hover(|d| d.text_color(theme.foreground))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.cycle_sort(s, cx)
                                            })),
                                        SortAvail::Disabled(why) => d
                                            .text_color(theme::text_disabled(theme))
                                            .tooltip(move |window, cx| {
                                                Tooltip::new(why).build(window, cx)
                                            }),
                                    })
                                    .child(glyph)
                            })
                            // Resize grip on the right edge.
                            .child(
                                div()
                                    .id(("resize", s))
                                    .w(theme::space_4())
                                    .h_full()
                                    .cursor_col_resize()
                                    .hover(|d| d.bg(theme.primary))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                            if ev.click_count == 2 {
                                                this.autofit(s);
                                            } else {
                                                this.resizing =
                                                    Some((s, ev.position.x, this.widths[s]));
                                            }
                                            cx.notify();
                                            cx.stop_propagation();
                                        }),
                                    ),
                            )
                    }),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .w(self.gutter())
                    .h_full()
                    .bg(theme.table_head)
                    .border_r_1()
                    .border_color(theme.border),
            )
    }
}
