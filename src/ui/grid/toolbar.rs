//! The toolbar and the WHERE / ORDER BY filter bar.

use super::*;

impl Grid {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live_pager = self
            .render_live_pager(cx)
            .map(IntoElement::into_any_element);
        let theme = cx.theme();
        let (page, size, last) = (
            self.page,
            self.page_size,
            page_count(self.all.row_count(), self.page_size) - 1,
        );
        let page_button = |id: &'static str, icon: IconName, tip: &'static str, to: usize| {
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .tooltip(tip)
                .accessibility_label(tip)
                .disabled(to == page)
                .on_click(cx.listener(move |this, _, _, cx| this.set_page(to, size, cx)))
        };
        let (start, end) = (self.source.start, self.source.start + self.source.len);
        let range = if end == 0 {
            "No rows".to_string()
        } else {
            format!(
                "{}–{} of {}",
                grouped(start + 1),
                grouped(end),
                grouped(self.all.row_count())
            )
        };
        let pending = |id: &'static str, icon: IconName, tip: &'static str| {
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .tooltip(format!("{tip} (needs a live connection)"))
                .accessibility_label(tip)
                .disabled(true)
        };
        let divider = || {
            div()
                .flex_none()
                .w(px(1.))
                .h(theme::space_16())
                .mx(theme::space_4())
                .bg(theme.border)
        };
        let (grid, current) = (cx.entity().downgrade(), self.copy_csv);
        let pager = grid.clone();
        h_flex()
            .flex_none()
            .px(theme::space_4())
            .py(theme::space_2())
            .gap(theme::space_2())
            .border_b_1()
            .border_color(theme.border)
            .text_size(theme::ui())
            .map(|d| match live_pager {
                Some(pager) => d.child(pager),
                None => d
                    .child(page_button(
                        "grid-first",
                        IconName::ChevronFirst,
                        "First page",
                        0,
                    ))
                    .child(page_button(
                        "grid-prev",
                        IconName::ChevronLeft,
                        "Previous page",
                        page.saturating_sub(1),
                    ))
                    .child(local_pager_size(range, size, start, pager))
                    .child(page_button(
                        "grid-next",
                        IconName::ChevronRight,
                        "Next page",
                        (page + 1).min(last),
                    ))
                    .child(page_button(
                        "grid-last",
                        IconName::ChevronLast,
                        "Last page",
                        last,
                    )),
            })
            .child(divider())
            .child(if self.is_live() {
                Button::new("grid-refresh")
                    .ghost()
                    .small()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh (F5)")
                    .accessibility_label("Refresh")
                    .on_click(cx.listener(|this, _, _, cx| this.load(cx)))
            } else {
                pending("grid-refresh", IconName::RefreshCw, "Refresh")
            })
            // A live grid has no stop yet; showing a dead button there would claim it needs a connection.
            .when(!self.is_live(), |d| {
                d.child(pending("grid-stop", IconName::Square, "Stop query"))
            })
            .child(divider())
            .children(if self.is_live() {
                let read_only = self.read_only().is_some();
                let row_button =
                    |id: &'static str,
                     icon,
                     tip: &'static str,
                     key,
                     run: fn(&mut Self, &mut Context<Self>)| {
                        Button::new(id)
                            .ghost()
                            .small()
                            .icon(icon)
                            .tooltip(format!("{tip} ({key})"))
                            .accessibility_label(tip)
                            .disabled(read_only)
                            .on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
                    };
                [
                    row_button(
                        "grid-add-row",
                        IconName::Plus,
                        "Add row",
                        "Alt+Insert",
                        Self::add_row,
                    ),
                    row_button(
                        "grid-delete-row",
                        IconName::Minus,
                        "Delete row",
                        "Ctrl+Delete",
                        Self::delete_rows,
                    ),
                ]
            } else {
                [
                    pending("grid-add-row", IconName::Plus, "Add row"),
                    pending("grid-delete-row", IconName::Minus, "Delete row"),
                ]
            })
            .child(if self.is_live() {
                let idle = !self.has_changes();
                h_flex()
                    .gap(theme::space_2())
                    .child(
                        div().debug_selector(|| "grid-submit".into()).child(
                            Button::new("grid-submit")
                                .ghost()
                                .small()
                                .icon(IconName::ArrowUp)
                                .tooltip("Submit changes (Ctrl+Enter)")
                                .accessibility_label("Submit changes")
                                .disabled(idle)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                ),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "grid-revert".into()).child(
                            Button::new("grid-revert")
                                .ghost()
                                .small()
                                .icon(IconName::Undo2)
                                .tooltip("Revert changes")
                                .accessibility_label("Revert changes")
                                .disabled(idle)
                                .on_click(cx.listener(|this, _, _, cx| this.revert(cx))),
                        ),
                    )
                    .into_any_element()
            } else {
                pending("grid-submit", IconName::ArrowUp, "Submit changes").into_any_element()
            })
            .children(self.read_only().map(|note| {
                div()
                    .debug_selector(move || format!("grid-read-only:{note}"))
                    .px(theme::space_4())
                    .text_size(theme::ui_sm())
                    .text_color(theme.muted_foreground)
                    .child(note)
            }))
            .child(div().flex_1())
            .child(self.render_export(cx))
            .child(
                Button::new("grid-copy-format")
                    .ghost()
                    .small()
                    .label(if self.copy_csv {
                        "Copy as CSV"
                    } else {
                        "Copy as TSV"
                    })
                    .dropdown_caret(true)
                    .tooltip("Format Ctrl+C copies")
                    .dropdown_menu(move |menu, _, _| {
                        [
                            ("TSV (tab-separated)", false),
                            ("CSV (comma-separated)", true),
                        ]
                        .into_iter()
                        .fold(
                            menu.label("Ctrl+C copies as"),
                            |menu, (name, is_csv)| {
                                let grid = grid.clone();
                                menu.item(
                                    PopupMenuItem::new(name)
                                        .checked(current == is_csv)
                                        .on_click(move |_, _, cx| {
                                            _ = grid.update(cx, |this, cx| {
                                                this.copy_csv = is_csv;
                                                cx.notify();
                                            });
                                        }),
                                )
                            },
                        )
                    }),
            )
    }

    pub(super) fn render_filter_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if self.filter.is_none() {
            let (where_input, order_input) = (
                filter_input("WHERE", window, cx),
                filter_input("ORDER BY", window, cx),
            );
            // A restored tab shows the filter it reads with.
            if let Some((_, where_, order_by)) = self.source() {
                let (where_, order_by) = (where_.to_string(), order_by.to_string());
                where_input.update(cx, |i, cx| i.set_value(where_, window, cx));
                order_input.update(cx, |i, cx| i.set_value(order_by, window, cx));
            }
            self.filter = Some((where_input, order_input));
        }
        let (where_input, order_input) = self.filter.clone().expect("made above");
        let theme = cx.theme();
        let icon = |name: IconName| Icon::new(name).xsmall().text_color(theme.muted_foreground);
        h_flex()
            .flex_none()
            .items_stretch()
            .border_b_1()
            .border_color(theme.border)
            .text_size(theme::ui())
            .child(
                filter_box("where", &where_input, icon(IconName::Funnel))
                    .w(relative(self.where_share)),
            )
            .child(
                div()
                    .id("filter-split")
                    .flex_none()
                    .w(theme::space_8())
                    .cursor_col_resize()
                    .flex()
                    .justify_center()
                    .child(div().w(px(1.)).h_full().bg(theme.border))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.filter_drag = Some((ev.position.x, this.where_share));
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child(filter_box("order", &order_input, icon(IconName::ArrowDownWideNarrow)).flex_1())
    }
}

/// A filter bar box: one line of highlighted, completed SQL; Enter in it applies both boxes.
fn filter_input(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<Grid>,
) -> Entity<EditorState> {
    let input = cx.new(|cx| {
        EditorState::new(window, cx)
            .language("sql")
            .line_number(false)
            .soft_wrap(false)
            .scroll_beyond_last_line(Some(0))
            .submit_on_enter(true)
            .placeholder(placeholder)
    });
    let provider = Rc::new(completion::Provider {
        grid: cx.weak_entity(),
        clause: placeholder,
    });
    input.update(cx, |e, cx| {
        e.lsp_mut().completion_provider = Some(provider);
        e.set_highlighter_factory(completion::highlighter(placeholder), cx);
    });
    cx.subscribe_in(
        &input,
        window,
        |this, input, ev: &InputEvent, window, cx| {
            if let InputEvent::Change = ev {
                input.update(cx, |input, cx| one_line(input, window, cx));
            }
            if let InputEvent::PressEnter { .. } = ev {
                // A typed ORDER BY replaces the header's sort.
                let order_by = this
                    .filter
                    .as_ref()
                    .map(|(_, o)| o.read(cx).value().trim().to_string());
                if order_by.is_some_and(|o| o != this.sort_order_by()) {
                    this.sort = None;
                }
                this.apply_filter(cx);
            }
        },
    )
    .detach();
    input
}

/// Turns line breaks a paste or Shift+Enter brought in into spaces, the caret staying put.
fn one_line(input: &mut EditorState, window: &mut Window, cx: &mut Context<EditorState>) {
    let text = input.value();
    if !text.contains(['\n', '\r']) {
        return;
    }
    let cursor = input.cursor();
    // Byte for byte, so `cursor` still points at the same character.
    let flat = text.replace(['\n', '\r'], " ");
    input.replace_all(flat, window, cx);
    let at = gpui_kit::component::input::RopeExt::offset_to_position(input.text(), cursor);
    input.set_cursor_position(at, window, cx);
}

fn filter_box(name: &'static str, state: &Entity<EditorState>, icon: Icon) -> Div {
    h_flex()
        .debug_selector(move || format!("grid-filter-{name}"))
        .min_w_0()
        .items_center()
        .pl(theme::space_8())
        .gap(theme::space_4())
        .child(icon)
        .child(
            gpui_kit::component::input::Editor::new(state)
                .bordered(false)
                .h(px(28.))
                .flex_1(),
        )
}

/// The row range, which opens the rows-per-page menu of a result in memory.
fn local_pager_size(
    range: String,
    size: usize,
    start: usize,
    pager: WeakEntity<Grid>,
) -> impl IntoElement {
    Button::new("grid-page-size")
        .ghost()
        .small()
        .label(range)
        .dropdown_caret(true)
        .tooltip("Rows per page")
        .dropdown_menu(move |menu, _, _| {
            PAGE_SIZES
                .into_iter()
                .fold(menu.label("Rows per page"), |menu, n| {
                    let pager = pager.clone();
                    menu.item(PopupMenuItem::new(grouped(n)).checked(n == size).on_click(
                        move |_, _, cx| {
                            // Stay on the page holding the current first row.
                            _ = pager.update(cx, |this, cx| this.set_page(start / n, n, cx));
                        },
                    ))
                })
        })
}
