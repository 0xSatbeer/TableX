//! The toolbar and the WHERE / ORDER BY filter bar.

use super::*;

impl Grid {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        // shortcut: no query backend yet, so these stay disabled until results come from a live connection.
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
            .child(
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
                                menu.item(
                                    PopupMenuItem::new(grouped(n)).checked(n == size).on_click(
                                        move |_, _, cx| {
                                            // Stay on the page holding the current first row.
                                            _ = pager.update(cx, |this, cx| {
                                                this.set_page(start / n, n, cx)
                                            });
                                        },
                                    ),
                                )
                            })
                    }),
            )
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
            ))
            .child(divider())
            .child(pending("grid-refresh", IconName::RefreshCw, "Refresh"))
            .child(pending("grid-stop", IconName::Square, "Stop query"))
            .child(divider())
            .child(pending("grid-add-row", IconName::Plus, "Add row"))
            .child(pending("grid-delete-row", IconName::Minus, "Delete row"))
            .child(pending("grid-submit", IconName::ArrowUp, "Submit changes"))
            .child(div().flex_1())
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

    // shortcut: no query backend yet, so WHERE / ORDER BY are typed but not applied; wire to the query once drivers land.
    pub(super) fn render_filter_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (where_input, order_input) = self
            .filter
            .get_or_insert_with(|| {
                let mut input = |placeholder: &'static str, cx: &mut Context<Self>| {
                    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
                };
                (input("WHERE", cx), input("ORDER BY", cx))
            })
            .clone();
        let theme = cx.theme();
        let icon = |name: IconName| Icon::new(name).xsmall().text_color(theme.muted_foreground);
        h_flex()
            .flex_none()
            .items_stretch()
            .border_b_1()
            .border_color(theme.border)
            .text_size(theme::ui())
            .child(
                div().w(relative(self.where_share)).min_w_0().child(
                    Input::new(&where_input)
                        .small()
                        .bordered(false)
                        .prefix(icon(IconName::Funnel)),
                ),
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
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&order_input)
                        .small()
                        .bordered(false)
                        .prefix(icon(IconName::ArrowDownWideNarrow)),
                ),
            )
    }
}
