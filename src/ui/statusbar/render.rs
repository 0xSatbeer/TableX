use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tooltip::Tooltip,
};
use gpui_kit::*;

use gpui_kit::prelude::FluentBuilder as _;

use crate::{ToggleBottomPanel, ToggleExplorer, theme};

use super::{Connect, StatusBar, readout};

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let toggle = |id: &'static str,
                      icon: IconName,
                      tip: &'static str,
                      action: fn() -> Box<dyn Action>| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip_with_action(tip, &*action(), None)
                .on_click(move |_, window, cx| window.dispatch_action(action(), cx))
        };
        let readout = self
            .origin
            .as_ref()
            .and_then(|o| {
                self.workspace
                    .read(cx)
                    .connections
                    .iter()
                    .find(|c| c.id == o.connection)
            })
            .map(readout);
        let connected = readout.as_ref().is_some_and(|r| r.solid);
        let tip = |selector: String, text: String| {
            move |window: &mut Window, cx: &mut App| {
                let (selector, text) = (selector.clone(), text.clone());
                Tooltip::element(move |_, _| {
                    div()
                        .debug_selector(|| selector.clone())
                        .child(text.clone())
                })
                .build(window, cx)
            }
        };
        let context = match &self.origin {
            Some(o) => h_flex()
                .min_w_0()
                .gap(theme::space_6())
                .child({
                    let color = theme::connection_color(o.color, theme);
                    let dot = div()
                        .id("status-dot")
                        .flex_none()
                        .size(theme::space_6())
                        .rounded_full();
                    match &readout {
                        Some(r) if !r.solid => dot
                            .debug_selector(|| "status-dot-ring".into())
                            .border_2()
                            .border_color(color),
                        _ => dot.debug_selector(|| "status-dot-solid".into()).bg(color),
                    }
                    .when_some(readout.as_ref(), |d, r| {
                        d.tooltip(tip(format!("status-tip:{}", r.tip), r.tip.clone()))
                    })
                })
                .child(
                    div()
                        .debug_selector(|| "status-path".into())
                        .min_w_0()
                        .truncate()
                        .text_color(theme.foreground)
                        .font_family(theme.mono_font_family.clone())
                        .child(theme::truncating(
                            "status-path",
                            StyledText::new(o.path.clone()),
                            o.path.clone(),
                        )),
                )
                .when(o.production, |d| d.child(theme::prod_badge(theme)))
                .when_some(
                    readout.as_ref().and_then(|r| r.text.map(|t| (t, r))),
                    |d, (text, r)| {
                        let selector = format!(
                            "{}:{text}",
                            if r.danger {
                                "status-text-danger"
                            } else {
                                "status-text"
                            }
                        );
                        d.child(
                            div()
                                .id("status-text")
                                .flex_none()
                                .whitespace_nowrap()
                                .debug_selector(|| selector)
                                .when(r.danger, |d| d.text_color(theme.danger))
                                .when_some(r.hint.clone(), |d, hint| {
                                    d.tooltip(tip(format!("status-hint:{hint}"), hint))
                                })
                                .child(text),
                        )
                    },
                )
                .when_some(readout.as_ref().and_then(|r| r.button), |d, label| {
                    let id = o.connection;
                    d.child(
                        div()
                            .flex_none()
                            .debug_selector(|| format!("status-button:{label}"))
                            .child(
                                Button::new("status-connect")
                                    .xsmall()
                                    .label(label)
                                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(Connect(id)))),
                            ),
                    )
                }),
            None if self.has_tab => {
                let text = if self.demo {
                    "Sample data · not from a connection"
                } else {
                    "No connection"
                };
                h_flex()
                    .debug_selector(|| format!("status-origin:{text}"))
                    .child(text)
            }
            None => h_flex(),
        };
        h_flex()
            .flex_none()
            .h(theme::status_bar())
            .px(theme::space_4())
            .gap(theme::space_12())
            .relative()
            .bg(theme.status_bar)
            .when(self.origin.as_ref().is_some_and(|o| o.production), |d| {
                d.child(div().absolute().inset_0().bg(theme::production_tint(theme)))
            })
            .border_t_1()
            .border_color(theme.status_bar_border)
            .text_size(theme::ui_sm())
            .text_color(theme.muted_foreground)
            .child(
                h_flex()
                    .child(toggle(
                        "toggle-explorer",
                        IconName::PanelLeft,
                        "Toggle explorer",
                        || Box::new(ToggleExplorer),
                    ))
                    .child(toggle(
                        "toggle-bottom",
                        IconName::PanelBottom,
                        "Toggle bottom panel",
                        || Box::new(ToggleBottomPanel),
                    )),
            )
            .child(context)
            // ponytail: tx state is fixed until QX-012/025 give it a model; shown only while Connected
            .when(connected, |d| {
                d.child(
                    div()
                        .flex_none()
                        .debug_selector(|| "auto-commit".into())
                        .child("AUTO-COMMIT"),
                )
            })
            .child(div().flex_1())
            .children(self.aggregate_text())
            .children(self.cursor.map(|(line, col, selected)| match selected {
                0 => format!("Ln {line}, Col {col}"),
                n => format!("Ln {line}, Col {col} ({n} selected)"),
            }))
            .children(self.memory_mb.map(|mb| {
                div()
                    .id("memory")
                    .font_family(theme.mono_font_family.clone())
                    .pr(theme::space_4())
                    .tooltip(|window, cx| {
                        Tooltip::new("QueryX private memory (RssAnon)").build(window, cx)
                    })
                    .child(format!("{mb} MB"))
            }))
    }
}
