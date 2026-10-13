use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::console::Console;
use crate::{MaximizePanel, ToggleBottomPanel, theme};

use super::filter::empty_message;
use super::row::log_header;
use super::{BottomPanel, LOGS, LogFilter, RESULTS, TABS};

impl Render for BottomPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connection_menu = self.connection_menu(cx);
        let filter_menu = self.filter_menu(cx);
        let tab = if self.console.is_none() && self.tab == RESULTS {
            LOGS
        } else {
            self.tab
        };
        let theme = cx.theme();
        // Flat, same height as the editor strip; the active view is underlined, not boxed.
        let header = h_flex()
            .flex_none()
            .h(theme::tab())
            .pl(theme::space_12())
            .pr(theme::space_8())
            .gap(theme::space_16())
            .border_b_1()
            .border_color(theme.border)
            .children(
                TABS.iter()
                    .enumerate()
                    .filter(|(ix, _)| self.console.is_some() || *ix != RESULTS)
                    .map(|(ix, label)| {
                        let active = ix == tab;
                        div()
                            .id(("panel-tab", ix))
                            .h_full()
                            .flex()
                            .items_center()
                            .text_size(theme::ui_sm())
                            .border_b_2()
                            .border_color(if active {
                                theme.foreground
                            } else {
                                transparent_black()
                            })
                            .text_color(if active {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .hover(|d| d.text_color(theme.foreground))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tab = ix;
                                cx.notify();
                            }))
                            .child(*label)
                    }),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap(theme::space_8())
                    .when(tab == LOGS, |this| {
                        this.child(
                            div()
                                .w(theme::space_24() * 8.)
                                .child(Input::new(&self.search).small()),
                        )
                        .child(connection_menu)
                        .child(filter_menu)
                    })
                    .child({
                        let (icon, label) = if self.maximized {
                            (IconName::WindowRestore, "Restore panel")
                        } else {
                            (IconName::WindowMaximize, "Maximize panel")
                        };
                        Button::new("maximize-panel")
                            .ghost()
                            .xsmall()
                            .icon(icon)
                            .tooltip_with_action(label, &MaximizePanel, None)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(MaximizePanel), cx)
                            })
                    })
                    .child(
                        Button::new("hide-panel")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip_with_action("Hide panel", &ToggleBottomPanel, None)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(ToggleBottomPanel), cx)
                            }),
                    ),
            );
        let empty = |text: &'static str| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let body = match tab {
            LOGS if let Some(message) = empty_message(&self.rows) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(theme::space_8())
                .text_color(theme.muted_foreground)
                .child(message)
                .child(
                    Button::new("reset-filters")
                        .small()
                        .label("Reset filters")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.filter = LogFilter::default();
                            this.search
                                .update(cx, |search, cx| search.set_value("", window, cx));
                            this.refilter(cx);
                            cx.notify();
                        })),
                )
                .into_any_element(),
            // Newest first: during an incident the last query is the one you want.
            LOGS => v_flex()
                .size_full()
                .child(log_header(theme))
                .child(
                    list(
                        self.list.clone(),
                        cx.processor(|this, ix: usize, _, cx| {
                            this.log_row(this.rows[this.rows.len() - 1 - ix], cx)
                                .into_any_element()
                        }),
                    )
                    .flex_1()
                    .w_full(),
                )
                .into_any_element(),
            RESULTS => match self
                .console
                .as_ref()
                .filter(|c| !c.read(cx).results.is_empty())
            {
                Some(console) => results(console, cx).into_any_element(),
                None => empty("Console results will show here").into_any_element(),
            },
            _ => match self
                .console
                .as_ref()
                .filter(|c| !c.read(cx).messages.is_empty())
            {
                Some(console) => v_flex()
                    .size_full()
                    .p(theme::space_8())
                    .gap(theme::space_4())
                    .font_family(theme.mono_font_family.clone())
                    .children(console.read(cx).messages.clone())
                    .into_any_element(),
                None => empty("No messages").into_any_element(),
            },
        };
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
    }
}

/// The console's result tabs as a strip over the active one's grid.
fn results(console: &Entity<Console>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let this = console.read(cx);
    let strip = h_flex()
        .flex_none()
        .px(theme::space_8())
        .gap(theme::space_4())
        .border_b_1()
        .border_color(theme.border)
        .children((0..this.results.len()).map(|ix| {
            let console = console.clone();
            Button::new(("result-tab", ix))
                .ghost()
                .xsmall()
                .selected(ix == this.active_result)
                .label(format!("Result {}", ix + 1))
                .on_click(move |_, _, cx| {
                    console.update(cx, |console, cx| {
                        console.active_result = ix;
                        cx.notify();
                    })
                })
        }));
    v_flex().size_full().child(strip).child(
        div()
            .flex_1()
            .min_h_0()
            .child(this.results[this.active_result].clone()),
    )
}
