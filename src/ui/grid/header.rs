//! Header cells: name (type in its tooltip), key icon, sort control, and column dragging.

use super::*;

/// Above this the sort belongs to the query (QX-011 passes it to ORDER BY), not the client.
pub(super) const CLIENT_SORT_MAX: usize = 100_000;

/// What a header cell shows for one source column.
#[derive(Debug, PartialEq)]
pub(super) struct HeaderModel {
    pub(super) name: String,
    pub(super) data_type: String,
    pub(super) pk: bool,
    pub(super) fk: bool,
}

/// `key` and `fks` name a table tab's primary and foreign key columns; result sets pass none.
pub(super) fn header_model(c: &ColumnMeta, key: &[String], fks: &[String]) -> HeaderModel {
    let named = |cols: &[String]| cols.iter().any(|k| **k == *c.name);
    HeaderModel {
        name: c.name.to_string(),
        data_type: c.data_type.to_string(),
        pk: c.is_pk || named(key),
        fk: c.is_fk || named(fks),
    }
}

/// The header icon of a column that is no key: what kind of value it holds.
pub(super) fn type_icon(c: &ColumnMeta) -> Lucide {
    let ty = c.data_type.to_lowercase();
    let has = |p: &[&str]| p.iter().any(|p| ty.starts_with(p));
    if ty.ends_with("[]") || ty.starts_with('_') {
        Lucide::Brackets
    } else if !c.enum_values.is_empty() {
        Lucide::List
    } else if has(&["json"]) {
        Lucide::Braces
    } else if is_numeric_type(&ty) || has(&["serial", "bigserial", "smallserial", "oid"]) {
        Lucide::Hash
    } else if has(&["bool"]) {
        Lucide::ToggleLeft
    } else if has(&["timestamp", "datetime"]) {
        Lucide::CalendarClock
    } else if has(&["date"]) {
        Lucide::Calendar
    } else if has(&["time", "interval"]) {
        Lucide::Clock
    } else if has(&["bytea", "blob", "binary", "varbinary"]) {
        Lucide::Binary
    } else if has(&["uuid"]) {
        Lucide::FingerprintPattern
    } else if has(&["inet", "cidr", "macaddr"]) {
        Lucide::Network
    } else {
        Lucide::Type
    }
}

/// Whether the header sort control works for a source of this size.
#[derive(Debug, PartialEq)]
pub(super) enum SortAvail {
    Enabled,
    Disabled(&'static str),
}

pub(super) fn sort_availability(rows: usize, data_type: &str) -> SortAvail {
    if data_type.to_lowercase().starts_with("json") {
        SortAvail::Disabled("JSON columns can't be sorted")
    } else if rows <= CLIENT_SORT_MAX {
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
    /// A table tab's primary and foreign key columns; none for a result set.
    pub(super) fn key_columns(&self) -> (&[String], &[String]) {
        self.live.as_ref().map_or((&[], &[]), |l| {
            (l.key.as_deref().unwrap_or_default(), &l.fks)
        })
    }

    /// Header sort button: ascending, descending, off.
    pub(super) fn cycle_sort(&mut self, c: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            Some((sc, true)) if sc == c => Some((c, false)),
            Some((sc, false)) if sc == c => None,
            _ => Some((c, true)),
        };
        if self.is_live() {
            return self.sort_live(window, cx);
        }
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
        let (key, fks) = self.key_columns();
        div()
            .relative()
            .flex_none()
            // One row tall, like the rows under it.
            .h(self.row_h())
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
                            .group(SharedString::from(format!("col-{s}")))
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
                                    .px(theme::space_6())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child({
                                        let m = header_model(&columns[s], key, fks);
                                        let tip = SharedString::from(m.data_type);
                                        h_flex()
                                            .id(("header-tip", s))
                                            .debug_selector(|| format!("header-tip-{s}"))
                                            .tooltip(move |window, cx| {
                                                let tip = tip.clone();
                                                Tooltip::element(move |_, _| {
                                                    div()
                                                        .debug_selector(|| "type-tip".into())
                                                        .child(tip.clone())
                                                })
                                                .build(window, cx)
                                            })
                                            .gap(theme::space_4())
                                            .font_family(theme.mono_font_family.clone())
                                            .text_size(self.mono_px())
                                            .text_color(theme.foreground)
                                            // Every column has an icon; keys get theirs instead of the type's.
                                            .when(m.pk, |d| {
                                                d.child(
                                                    Icon::new(IconName::KeyRound)
                                                        .size_3()
                                                        .text_color(theme.warning),
                                                )
                                            })
                                            .when(m.fk, |d| {
                                                d.child(
                                                    Icon::new(IconName::KeyRound)
                                                        .size_3()
                                                        .text_color(theme.info),
                                                )
                                            })
                                            .when(!m.pk && !m.fk, |d| {
                                                d.child(
                                                    Icon::new(type_icon(&columns[s]))
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
                                    .context_menu({
                                        let focus = self.focus.clone();
                                        move |menu, _, _| {
                                            menu.action_context(focus.clone())
                                                .min_w(theme::menu_sm())
                                                .menu_with_icon(
                                                    "Hide column",
                                                    Icon::new(Lucide::EyeOff),
                                                    Box::new(HideColumn),
                                                )
                                                .menu_with_icon(
                                                    "Show all columns",
                                                    Icon::new(Lucide::Eye),
                                                    Box::new(ShowAllColumns),
                                                )
                                        }
                                    }),
                            )
                            .child({
                                let avail = sort_availability(
                                    self.source.row_count(),
                                    &columns[s].data_type,
                                );
                                let glyph = match self.sort {
                                    Some((sc, true)) if sc == s => "↑",
                                    Some((sc, false)) if sc == s => "↓",
                                    _ => "↕",
                                };
                                div()
                                    .id(("sort", s))
                                    .debug_selector(|| format!("sort-{s}"))
                                    // Fixed width: autofit reserves exactly this much beside the header text.
                                    .flex_none()
                                    .w(theme::space_16())
                                    .flex()
                                    .justify_center()
                                    .text_size(theme::ui_sm())
                                    // Drag must not start from the sort control.
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .map(|d| match avail {
                                        SortAvail::Enabled => d
                                            // An unsorted column shows its control only while the header is hovered.
                                            .when(glyph == "↕", |d| {
                                                d.text_color(transparent_black()).group_hover(
                                                    SharedString::from(format!("col-{s}")),
                                                    |d| d.text_color(theme.muted_foreground),
                                                )
                                            })
                                            .hover(|d| d.text_color(theme.foreground))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.cycle_sort(s, window, cx)
                                            })),
                                        SortAvail::Disabled(why) => d
                                            .text_color(transparent_black())
                                            .group_hover(
                                                SharedString::from(format!("col-{s}")),
                                                |d| d.text_color(theme::text_disabled(theme)),
                                            )
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
