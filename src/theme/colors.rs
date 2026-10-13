use gpui_kit::component::theme::Theme;
use gpui_kit::{Hsla, Rgba};

use super::{badge_line, hex, radius_sm, space_4, ui_sm};
use crate::model::ObjectKind;

/// Brand mark colors: lightened from the official ones on dark, the official ones on light.
pub fn brand_color(kind: crate::model::DbKind, theme: &Theme) -> Hsla {
    use crate::model::DbKind::*;
    match (kind, theme.is_dark()) {
        (Postgres, true) => hex(0x6B99F2),
        (Sqlite, true) => hex(0x73C7E5),
        (Postgres, false) => hex(0x336791),
        (Sqlite, false) => hex(0x0F6A94),
    }
    .into()
}

/// Connection colors, indexed by `model::Connection.color`. Same hues per index in both
/// themes, so a connection keeps its identity; each set holds 3:1 on its theme's chrome.
pub const CONNECTION_COLORS: [Rgba; 8] = [
    hex(0xF07178), // red
    hex(0xF5A65B), // orange
    hex(0xE5C76B), // yellow
    hex(0x7FD18B), // green
    hex(0x5CCFC0), // teal
    hex(0x6CA8F7), // blue
    hex(0xA98BFA), // violet
    hex(0xF07FC4), // pink
];

pub const CONNECTION_COLORS_LIGHT: [Rgba; 8] = [
    hex(0xD14B54), // red
    hex(0xB8621A), // orange
    hex(0x8F7510), // yellow
    hex(0x2A8C44), // green
    hex(0x10857A), // teal
    hex(0x2F6FD6), // blue
    hex(0x7A55E0), // violet
    hex(0xC2438A), // pink
];

pub fn connection_color(index: u8, theme: &Theme) -> Hsla {
    let set = if theme.is_dark() {
        &CONNECTION_COLORS
    } else {
        &CONNECTION_COLORS_LIGHT
    };
    set[index as usize % set.len()].into()
}

/// Same hue within a family (rows, routines), different glyph per kind.
pub fn object_color(kind: ObjectKind, theme: &Theme) -> Hsla {
    match kind {
        ObjectKind::Table => theme.blue,
        ObjectKind::View | ObjectKind::MaterializedView => theme.cyan,
        ObjectKind::Sequence => theme.green,
        ObjectKind::Function | ObjectKind::Procedure => theme.red_light,
        ObjectKind::Type => theme.magenta_light,
        ObjectKind::Index => theme.yellow_light,
        ObjectKind::Trigger => theme.green_light,
    }
}

// Tokens the gpui-kit theme format has no slot for. Derived from theme colors so a light
// theme gets them for free. The rest of the token list maps to theme fields (QX-002 ticket).
pub fn text_secondary(theme: &Theme) -> Hsla {
    theme.foreground.opacity(0.72)
}

/// Same as the library's disabled button text.
pub fn text_disabled(theme: &Theme) -> Hsla {
    theme.muted_foreground.opacity(0.5)
}

/// Selection in a panel that does not have focus.
pub fn selected_inactive(theme: &Theme) -> Hsla {
    theme.muted_foreground.opacity(0.18)
}

/// Background wash on anything that belongs to a production connection: tree rows, tabs,
/// the grid's source strip and the status bar.
pub fn production_tint(theme: &Theme) -> Hsla {
    // The same wash reads far louder on white, so light gets about half.
    theme
        .danger
        .opacity(if theme.is_dark() { 0.08 } else { 0.045 })
}

/// The `PROD` hazard label next to a production connection, wherever its name is shown.
/// Solid, not outlined: a label stuck on the thing, not one more box. Page-colored mono text on
/// `danger` holds 6.2:1 dark and 5.1:1 light (tested as `background` on `danger.background`).
pub fn prod_badge(theme: &Theme) -> gpui_kit::Div {
    use gpui_kit::{FontWeight, ParentElement as _, Styled as _};
    gpui_kit::div()
        .flex_none()
        .px(space_4())
        .rounded(radius_sm())
        .bg(theme.danger)
        .text_color(theme.background)
        .font_family(theme.mono_font_family.clone())
        .font_weight(FontWeight::BOLD)
        .text_size(ui_sm())
        .line_height(badge_line())
        .child("PROD")
}

pub fn grid_null(theme: &Theme) -> Hsla {
    theme.muted_foreground
}

pub fn grid_edited(theme: &Theme) -> Hsla {
    theme.warning.opacity(0.16)
}

pub fn grid_inserted(theme: &Theme) -> Hsla {
    theme.success.opacity(0.14)
}

pub fn grid_deleted(theme: &Theme) -> Hsla {
    theme.danger.opacity(0.14)
}

/// The ticket's syntax tokens, as keys in the theme's `highlight.syntax`.
/// `identifier` is tree-sitter's `variable`.
pub const SYNTAX: [&str; 8] = [
    "keyword", "string", "number", "comment", "function", "type", "operator", "variable",
];
