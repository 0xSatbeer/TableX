//! The list or calendar under the cell being edited, and why a value is refused.

use super::*;

impl Grid {
    /// Why a cell's text can't be saved, under that cell.
    pub(super) fn render_problem(&self, why: String, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let bubble = div()
            .debug_selector(|| "cell-problem".into())
            .occlude()
            .max_w(px(420.))
            .px(theme::space_8())
            .py(theme::space_4())
            .rounded(theme::radius_md())
            .border_1()
            .border_color(theme.danger)
            .bg(theme.popover)
            .text_color(theme.danger)
            .shadow_lg()
            .font_family(theme.font_family.clone())
            .text_size(theme::ui_sm())
            .whitespace_normal()
            .child(why);
        div()
            .absolute()
            .top_full()
            .left_0()
            .child(
                deferred(
                    anchored()
                        .snap_to_window_with_margin(theme::space_8())
                        .child(bubble),
                )
                .with_priority(1),
            )
            .into_any_element()
    }

    /// The list or calendar under the cell being edited; `None` for text, which is edited in place.
    pub(super) fn render_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editing = self.editing.as_ref()?;
        let theme = cx.theme();
        let n = editing.targets.len();
        let panel = v_flex()
            .id("cell-picker")
            .occlude()
            .min_w(self.widths[self.col_order[editing.cell.1]])
            .p(theme::space_4())
            .gap(theme::space_2())
            .rounded(theme::radius_md())
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .shadow_lg()
            .font_family(theme.font_family.clone())
            .text_size(theme::ui())
            .whitespace_normal()
            // Clicking away from a list or calendar picks nothing.
            .on_mouse_down_out(
                cx.listener(|this, _, window, cx| _ = this.stop_editing(false, window, cx)),
            )
            // Say up front that the pick goes to the whole selection, not just this cell.
            .when(n > 1, |d| {
                d.child(
                    div()
                        .px(theme::space_8())
                        .py(theme::space_2())
                        .text_size(theme::ui_sm())
                        .text_color(theme.muted_foreground)
                        .child(format!("Set {} cells to", grouped(n))),
                )
            });
        let panel = match &editing.editor {
            Editor::Text(_) => return None,
            Editor::Options {
                values,
                current,
                highlighted,
            } => panel
                .max_h(px(280.))
                .overflow_y_scroll()
                .children(values.iter().enumerate().map(|(i, v)| {
                    h_flex()
                        .id(("cell-option", i))
                        .debug_selector(move || format!("cell-option-{i}"))
                        .gap(theme::space_8())
                        .px(theme::space_8())
                        .h(self.row_h())
                        .rounded(theme::radius_sm())
                        .cursor_pointer()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(self.mono_px())
                        .hover(|d| d.bg(theme.accent))
                        .when(i == *highlighted, |d| {
                            d.bg(theme.accent).text_color(theme.accent_foreground)
                        })
                        .child(
                            div()
                                .flex_none()
                                .w(theme::space_12())
                                .when(*current == Some(i), |d| {
                                    d.child(Icon::new(IconName::Check).xsmall())
                                }),
                        )
                        .child(v.to_string())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(Editing {
                                editor: Editor::Options { highlighted, .. },
                                ..
                            }) = &mut this.editing
                            {
                                *highlighted = i;
                            }
                            this.stop_editing(true, window, cx);
                        }))
                })),
            Editor::Date { calendar, time, .. } => {
                let picked = calendar.read(cx).date().start().is_some();
                panel
                    .child(Calendar::new(calendar).small().border_0().p_0())
                    .when_some(time.clone(), |d, (field, _)| {
                        d.child(
                            h_flex()
                                .gap(theme::space_8())
                                .pt(theme::space_4())
                                .border_t_1()
                                .border_color(theme.border)
                                .child(div().text_color(theme.muted_foreground).child("Time"))
                                .child(TimeField::new(&field).small())
                                .child(div().flex_1())
                                .child(
                                    Button::new("cell-picker-set")
                                        .primary()
                                        .small()
                                        .label(if n > 1 {
                                            format!("Set {} cells", grouped(n))
                                        } else {
                                            "Set".into()
                                        })
                                        .disabled(!picked)
                                        .tooltip(if picked { "Enter" } else { "Pick a day first" })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            _ = this.stop_editing(true, window, cx)
                                        })),
                                ),
                        )
                    })
            }
        };
        Some(
            div()
                .absolute()
                .top_full()
                .left_0()
                .child(
                    deferred(
                        anchored()
                            .snap_to_window_with_margin(theme::space_8())
                            .child(panel),
                    )
                    .with_priority(1),
                )
                .into_any_element(),
        )
    }
}
