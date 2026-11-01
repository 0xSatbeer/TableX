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
            .bg(theme.background)
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
            // Query results only page and copy; editing and the filter bar are for table tabs.
            .when(self.is_live(), |d| {
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
                let idle = !self.has_changes();
                d.child(divider())
                    .child(
                        Button::new("grid-refresh")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .tooltip("Refresh (F5)")
                            .accessibility_label("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    )
                    .child(divider())
                    .children([
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
                    ])
                    .child(
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
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit(window, cx)
                                        })),
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
                            ),
                    )
            })
            .children(self.read_only().filter(|_| self.is_live()).map(|note| {
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
                    .label(if self.copy_csv { "CSV" } else { "TSV" })
                    .dropdown_caret(true)
                    .tooltip("Format Ctrl+C copies")
                    .accessibility_label("Format Ctrl+C copies")
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
    ) -> Option<impl IntoElement> {
        if !self.is_live() {
            return None;
        }
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
        let muted = cx.theme().muted_foreground;
        let icon = |name: IconName| Icon::new(name).xsmall().text_color(muted);
        let where_box = filter_box("where", "WHERE", &where_input, icon(IconName::Funnel), cx);
        let order_box = filter_box(
            "order",
            "ORDER BY",
            &order_input,
            icon(IconName::ArrowDownWideNarrow),
            cx,
        );
        let theme = cx.theme();
        Some(
            h_flex()
                .flex_none()
                .items_stretch()
                .bg(theme.table)
                .border_b_1()
                .border_color(theme.border)
                .text_size(theme::ui())
                .child(where_box.w(relative(self.where_share)))
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
                .child(order_box.flex_1()),
        )
    }
}

/// A filter bar box: one line of highlighted, completed SQL; Enter in it applies both boxes.
fn filter_input(
    clause: &'static str,
    window: &mut Window,
    cx: &mut Context<Grid>,
) -> Entity<EditorState> {
    let input = cx.new(|cx| {
        EditorState::new(window, cx)
            .language("sql")
            .line_number(false)
            // One line never folds; the fold gutter would only push the text away from the icon.
            .folding(false)
            .soft_wrap(false)
            .scroll_beyond_last_line(Some(0))
            .submit_on_enter(true)
    });
    let provider = Rc::new(completion::Provider {
        grid: cx.weak_entity(),
        clause,
    });
    input.update(cx, |e, cx| {
        e.lsp_mut().completion_provider = Some(provider);
        e.set_highlighter_factory(completion::highlighter(clause), cx);
    });
    cx.subscribe_in(
        &input,
        window,
        |this, input, ev: &InputEvent, window, cx| {
            if let InputEvent::Change = ev {
                input.update(cx, |input, cx| one_line(input, window, cx));
                // The clear button shows only while the box has text.
                cx.notify();
            }
            if let InputEvent::PressEnter { .. } = ev {
                this.submit_filter(cx);
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

impl Grid {
    /// Applies both boxes (Enter, the clear button). A typed ORDER BY replaces the header's sort.
    fn submit_filter(&mut self, cx: &mut Context<Self>) {
        let order_by = self
            .filter
            .as_ref()
            .map(|(_, o)| o.read(cx).value().trim().to_string());
        if order_by.is_some_and(|o| o != self.sort_order_by()) {
            self.sort = None;
        }
        self.apply_filter(cx);
    }
}

/// A filter bar box: icon, the clause as a keyword label, the editor, and a clear button while it has text.
fn filter_box(
    name: &'static str,
    clause: &'static str,
    state: &Entity<EditorState>,
    icon: Icon,
    cx: &mut Context<Grid>,
) -> Div {
    // The editor pads its line with Medium's input_py above and below, which a 28px box cannot
    // hold: give it its full height and center it in a clipped 28px box, so the line sits mid-row.
    let pad = gpui_kit::component::Size::Medium.input_py();
    let theme = cx.theme();
    let editor_h = theme.mono_font_size * 1.5 + pad * 2.;
    let keyword = theme
        .highlight_theme
        .style("keyword")
        .and_then(|s| s.color)
        .unwrap_or(theme.foreground);
    let label = div()
        .flex_none()
        .ml(theme::space_4())
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
        .text_color(keyword)
        .child(clause);
    let has_text = !state.read(cx).value().is_empty();
    let state_for_clear = state.clone();
    let clear = Button::new(SharedString::from(format!("grid-filter-{name}-clear")))
        .ghost()
        .xsmall()
        .icon(IconName::Close)
        .tooltip("Clear")
        .accessibility_label("Clear")
        .on_click(cx.listener(move |this, _, window, cx| {
            state_for_clear.update(cx, |s, cx| s.set_value("", window, cx));
            this.submit_filter(cx);
        }));
    h_flex()
        .debug_selector(move || format!("grid-filter-{name}"))
        .min_w_0()
        .h(px(28.))
        .overflow_hidden()
        .items_center()
        .pl(theme::space_8())
        .pr(theme::space_4())
        .child(icon)
        .child(label)
        .child(
            // No background or border of its own: the bar's shows, and the pulled-left editor leaves the icon visible.
            gpui_kit::component::input::Editor::new(state)
                .appearance(false)
                .h(editor_h)
                // The code editor insets its text 12px (6px padding + 6px gutter margin) and offers
                // no way to change it: pull it left so the text sits one mono space after the label.
                .ml(theme::space_8() - px(12.))
                .flex_1(),
        )
        .when(has_text, |d| {
            d.child(
                div()
                    .debug_selector(move || format!("grid-filter-{name}-clear"))
                    .child(clear),
            )
        })
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
