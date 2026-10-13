//! Hidden design gallery (Ctrl+Alt+Shift+D): every token, icon and control state on one page,
//! so M1 review can check the design system without hunting through the app.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, ThemeStyled as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{assets, model::ObjectKind, theme};

mod tokens;

fn section(title: &'static str, content: impl IntoElement, cx: &App) -> Div {
    v_flex()
        .gap(theme::space_8())
        .child(
            div()
                .text_size(theme::ui_lg())
                .text_color(cx.theme().muted_foreground)
                .child(title),
        )
        .child(content)
}

fn labeled(label: impl Into<SharedString>, content: impl IntoElement, cx: &App) -> Div {
    v_flex()
        .gap(theme::space_4())
        .items_center()
        .child(content)
        .child(
            div()
                .text_size(theme::ui_sm())
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
}

pub fn gallery(cx: &App) -> impl IntoElement {
    let t = cx.theme();

    let swatch = |color: Hsla| {
        div()
            .w(theme::space_24() * 3.)
            .h(theme::space_24() * 1.5)
            .rounded(theme::radius_sm())
            .border_1()
            .border_color(t.border)
            .bg(color)
    };
    macro_rules! swatches {
        ($($field:ident),* $(,)?) => { [$((stringify!($field), t.$field)),*] };
    }
    let colors = h_flex().flex_wrap().gap(theme::space_12()).children(
        swatches!(
            background,
            foreground,
            muted,
            muted_foreground,
            border,
            popover,
            primary,
            primary_foreground,
            secondary,
            accent,
            selection,
            list_active,
            list_hover,
            table_even,
            success,
            warning,
            danger,
            info,
            red,
            red_light,
            green,
            green_light,
            blue,
            blue_light,
            yellow,
            yellow_light,
            magenta,
            magenta_light,
            cyan,
            cyan_light,
        )
        .map(|(name, color)| labeled(name, swatch(color), cx)),
    );

    let derived = h_flex().flex_wrap().gap(theme::space_12()).children(
        [
            ("text_secondary", theme::text_secondary(t)),
            ("text_disabled", theme::text_disabled(t)),
            ("selected_inactive", theme::selected_inactive(t)),
            ("grid_null", theme::grid_null(t)),
            ("grid_edited", theme::grid_edited(t)),
            ("grid_inserted", theme::grid_inserted(t)),
            ("grid_deleted", theme::grid_deleted(t)),
        ]
        .map(|(name, color)| labeled(name, swatch(color), cx)),
    );
    let grid_states = v_flex()
        .w(theme::space_24() * 16.)
        .font_family(t.mono_font_family.clone())
        .text_size(theme::mono())
        .children(
            [
                ("1  ana@x.io     pro", None, t.background),
                ("2  bo@y.dev     NULL", None, t.table_even),
                (
                    "3  cy@z.io      team",
                    Some("edited"),
                    theme::grid_edited(t),
                ),
                (
                    "4  di@w.dev     free",
                    Some("inserted"),
                    theme::grid_inserted(t),
                ),
                (
                    "5  ed@v.io      pro",
                    Some("deleted"),
                    theme::grid_deleted(t),
                ),
            ]
            .map(|(row, state, bg)| {
                h_flex()
                    .h(theme::grid_row())
                    .px(theme::space_8())
                    .justify_between()
                    .bg(bg)
                    .when(row.ends_with("NULL"), |d| d.text_color(theme::grid_null(t)))
                    .when(state == Some("deleted"), |d| d.line_through())
                    .child(row)
                    .children(state.map(|s| {
                        div()
                            .text_size(theme::ui_sm())
                            .text_color(t.muted_foreground)
                            .child(s)
                    }))
            }),
        );

    let syntax = h_flex()
        .gap(theme::space_16())
        .font_family(t.mono_font_family.clone())
        .text_size(theme::mono())
        .children(theme::SYNTAX.map(|name| {
            let color = t
                .highlight_theme
                .style(name)
                .and_then(|s| s.color)
                .unwrap_or(t.foreground);
            div().text_color(color).child(name)
        }));

    let elevation = h_flex()
        .gap(theme::space_24())
        .child(labeled(
            "panel: 1px hairline, no shadow",
            div()
                .w(theme::space_24() * 8.)
                .h(theme::space_24() * 3.)
                .bg(t.sidebar)
                .border_1()
                .border_color(t.border),
            cx,
        ))
        .child(labeled(
            format!(
                "overlay: popover surface + shadow, {} ms",
                theme::MOTION_OVERLAY.as_millis()
            ),
            div()
                .w(theme::space_24() * 8.)
                .h(theme::space_24() * 3.)
                .popover_style(cx),
            cx,
        ));

    let objects = ObjectKind::ALL.into_iter().flat_map(|kind| {
        let color = theme::object_color(kind, t);
        [
            labeled(
                kind.as_str(),
                Icon::empty()
                    .path(assets::object_icon(kind.as_str(), false))
                    .text_color(color),
                cx,
            ),
            labeled(
                format!("{}s", kind.as_str()),
                Icon::empty()
                    .path(assets::object_icon(kind.as_str(), true))
                    .text_color(color.opacity(0.7)),
                cx,
            ),
        ]
    });
    let others = [
        (
            "postgres",
            Icon::empty().path(assets::POSTGRES_ICON),
            theme::brand_color(crate::model::DbKind::Postgres, cx.theme()),
        ),
        (
            "sqlite",
            Icon::empty().path(assets::SQLITE_ICON),
            theme::brand_color(crate::model::DbKind::Sqlite, cx.theme()),
        ),
        ("database", Icon::new(Lucide::Database), t.yellow),
        ("schema", Icon::new(Lucide::Layers), t.magenta),
        ("folder", Icon::new(Lucide::Folder), t.muted_foreground),
    ]
    .map(|(name, icon, color)| labeled(name, icon.text_color(color), cx));
    let icons = h_flex()
        .flex_wrap()
        .gap(theme::space_16())
        .children(objects)
        .children(others);

    let buttons = h_flex()
        .gap(theme::space_8())
        .child(Button::new("b-default").label("Default"))
        .child(Button::new("b-primary").primary().label("Primary"))
        .child(Button::new("b-secondary").secondary().label("Secondary"))
        .child(Button::new("b-danger").danger().label("Danger"))
        .child(Button::new("b-ghost").ghost().label("Ghost"))
        .child(Button::new("b-outline").outline().label("Outline"))
        .child(Button::new("b-selected").label("Selected").selected(true))
        .child(
            Button::new("b-disabled")
                .primary()
                .label("Disabled")
                .disabled(true),
        )
        .child(Button::new("b-loading").label("Loading").loading(true))
        .child(Button::new("b-small").small().label("Small"));
    let toggles = h_flex()
        .gap(theme::space_16())
        .child(Checkbox::new("c-on").label("Checked").checked(true))
        .child(Checkbox::new("c-off").label("Unchecked"))
        .child(
            Checkbox::new("c-disabled")
                .label("Disabled")
                .checked(true)
                .disabled(true),
        )
        .child(Switch::new("s-on").label("On").checked(true))
        .child(Switch::new("s-off").label("Off"))
        .child(Switch::new("s-disabled").label("Disabled").disabled(true));

    div()
        .id("design-gallery")
        .size_full()
        .overflow_y_scroll()
        .child(
            v_flex()
                .p(theme::space_24())
                .gap(theme::space_24())
                .child(div().text_size(theme::ui_xl()).child("Design gallery"))
                .child(section("Type scale", tokens::type_scale(cx), cx))
                .child(section("Spacing", tokens::spacing(cx), cx))
                .child(section("Radii", tokens::radii(cx), cx))
                .child(section("Sizes", tokens::sizes(cx), cx))
                .child(section("Connection colors", tokens::connections(cx), cx))
                .child(section("Theme colors", colors, cx))
                .child(section("Derived tokens", derived, cx))
                .child(section("Grid row states", grid_states, cx))
                .child(section("Syntax", syntax, cx))
                .child(section("Elevation and motion", elevation, cx))
                .child(section("Icons", icons, cx))
                .child(section(
                    "Controls",
                    v_flex()
                        .gap(theme::space_12())
                        .child(buttons)
                        .child(toggles),
                    cx,
                )),
        )
}
