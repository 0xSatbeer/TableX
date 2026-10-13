//! Design-token sections of the gallery: type scale, spacing, radii, sizes, connection colors.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use super::labeled;
use crate::theme;

pub(super) fn type_scale(cx: &App) -> Div {
    let t = cx.theme();
    v_flex()
        .gap(theme::space_4())
        .children(
            [
                ("UI_SM 12", theme::ui_sm()),
                ("UI 13", theme::ui()),
                ("UI_LG 15", theme::ui_lg()),
                ("UI_XL 18", theme::ui_xl()),
            ]
            .map(|(name, size)| {
                div()
                    .text_size(size)
                    .child(format!("{name}  The quick brown fox, 0123456789"))
            }),
        )
        .child(
            div()
                .font_family(t.mono_font_family.clone())
                .text_size(theme::mono())
                .child("MONO 12.5  SELECT id, email FROM users; 0123456789 NULL"),
        )
}

pub(super) fn spacing(cx: &App) -> Div {
    let t = cx.theme();
    h_flex().gap(theme::space_16()).items_end().children(
        [
            theme::space_2(),
            theme::space_4(),
            theme::space_6(),
            theme::space_8(),
            theme::space_12(),
            theme::space_16(),
            theme::space_24(),
        ]
        .map(|s| {
            labeled(
                format!("{}", f32::from(s)),
                div().w(s).h(theme::space_24()).bg(t.primary),
                cx,
            )
        }),
    )
}

pub(super) fn radii(cx: &App) -> Div {
    let t = cx.theme();
    h_flex().gap(theme::space_16()).children(
        [
            ("SM 4", theme::radius_sm()),
            ("MD 6", theme::radius_md()),
            ("LG 10", theme::radius_lg()),
        ]
        .map(|(name, r)| {
            labeled(
                name,
                div()
                    .w(theme::space_24() * 3.)
                    .h(theme::space_24() * 2.)
                    .rounded(r)
                    .border_1()
                    .border_color(t.border)
                    .bg(t.popover),
                cx,
            )
        }),
    )
}

pub(super) fn sizes(cx: &App) -> Div {
    let t = cx.theme();
    h_flex().gap(theme::space_16()).items_end().children(
        [
            ("tree row", theme::tree_row()),
            ("grid row", theme::grid_row()),
            ("tab", theme::tab()),
            ("titlebar", theme::titlebar()),
            ("status bar", theme::status_bar()),
        ]
        .map(|(name, h)| {
            labeled(
                format!("{name} {}", f32::from(h)),
                div().w(theme::space_24() * 4.).h(h).bg(t.secondary),
                cx,
            )
        }),
    )
}

pub(super) fn connections(cx: &App) -> Div {
    h_flex()
        .gap(theme::space_16())
        .children((0..theme::CONNECTION_COLORS.len() as u8).map(|i| {
            let color = theme::connection_color(i, cx.theme());
            labeled(
                i.to_string(),
                h_flex()
                    .gap(theme::space_6())
                    .child(div().size(theme::space_6()).rounded_full().bg(color))
                    .child(
                        div()
                            .w(theme::space_24() * 2.)
                            .h(theme::tree_row())
                            .border_l_2()
                            .border_color(color)
                            .bg(color.opacity(0.12)),
                    ),
                cx,
            )
        }))
}
