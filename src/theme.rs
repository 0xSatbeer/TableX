use std::borrow::Cow;
use std::time::Duration;

use gpui_kit::component::theme::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Pixels, Rgba, px};

mod colors;
mod elements;
mod user_themes;

pub use colors::*;
pub use elements::*;
pub use user_themes::*;

const DARK: &str = include_str!("../assets/themes/queryx-dark.json");
const LIGHT: &str = include_str!("../assets/themes/queryx-light.json");
/// Names of the themes in DARK and LIGHT; user themes may not reuse them.
const BUILT_IN: [&str; 2] = ["QueryX Dark", "QueryX Light"];

// Type scale. MONO matches `mono_font.size`.
//
// gpui-component sets the window rem to the theme's `font.size` and sizes its own text in rems:
// `text_sm` (0.875rem) for menus, tooltips and small buttons/inputs, `text_xs` (0.75rem) for
// xsmall ones. `font.size` is therefore UI_SM / 0.875 = 13.714px, so library text lands on
// UI_SM; with 13 it rendered at 11.4px and 9.75px. The shell pins body text to UI, so our own
// text never inherits the rem. Labeled library widgets use `.small()`, never `.xsmall()`.
/// Secondary UI text: tabs, table headers, counts.
const UI_SM: Pixels = px(12.);
const UI: Pixels = px(13.);
const UI_LG: Pixels = px(15.);
const UI_XL: Pixels = px(18.);
pub const MONO: Pixels = px(12.5);

const SPACE_2: Pixels = px(2.);
const SPACE_4: Pixels = px(4.);
const SPACE_6: Pixels = px(6.);
const SPACE_8: Pixels = px(8.);
const SPACE_12: Pixels = px(12.);
const SPACE_16: Pixels = px(16.);
const SPACE_24: Pixels = px(24.);

/// Controls. Cards and menus use RADIUS_MD, dialogs RADIUS_LG (the theme's `radius` and `radius.lg`).
const RADIUS_SM: Pixels = px(4.);
const RADIUS_MD: Pixels = px(6.);
const RADIUS_LG: Pixels = px(10.);

const TREE_ROW: Pixels = px(24.);
/// Log and result-list rows: one line of mono SQL plus badges needs more air than the tree.
const LIST_ROW: Pixels = px(28.);
/// Line height inside badges, so a badge sits inside a row instead of filling it.
const BADGE_LINE: Pixels = px(16.);
pub const GRID_ROW: Pixels = px(26.);
const TAB: Pixels = px(34.);
/// Connection-color rule on tabs.
const STRIPE: Pixels = px(2.);
const TITLEBAR: Pixels = px(38.);
const STATUS_BAR: Pixels = px(24.);
/// Context menu min widths: about 1.2x their widest label plus shortcut, so items don't crowd.
const MENU: Pixels = px(272.);
const MENU_SM: Pixels = px(192.);

// Every token is read through its function, which applies the UI scale.
macro_rules! scaled_tokens {
    ($($token:ident => $name:ident),* $(,)?) => {
        $(pub fn $name() -> Pixels { scaled($token) })*
    };
}

scaled_tokens! {
    UI_SM => ui_sm, UI => ui, UI_LG => ui_lg, UI_XL => ui_xl, MONO => mono,
    SPACE_2 => space_2, SPACE_4 => space_4, SPACE_6 => space_6, SPACE_8 => space_8,
    SPACE_12 => space_12, SPACE_16 => space_16, SPACE_24 => space_24,
    RADIUS_SM => radius_sm, RADIUS_MD => radius_md, RADIUS_LG => radius_lg,
    TREE_ROW => tree_row, LIST_ROW => list_row, BADGE_LINE => badge_line, GRID_ROW => grid_row,
    TAB => tab, STRIPE => stripe, TITLEBAR => titlebar, STATUS_BAR => status_bar,
    MENU => menu, MENU_SM => menu_sm,
}

/// The settings menu's UI scale choices, in percent.
pub const UI_SCALES: [u16; 5] = [90, 100, 110, 120, 130];
pub const DEFAULT_UI_SCALE: u16 = 100;

thread_local! {
    // The UI runs on one thread; per-thread state also keeps parallel tests apart.
    static UI_SCALE: std::cell::Cell<u16> = const { std::cell::Cell::new(DEFAULT_UI_SCALE) };
}

/// `token` at `percent` UI scale.
pub fn scale(token: Pixels, percent: u16) -> Pixels {
    token * (percent as f32 / 100.)
}

/// `token` at the current UI scale.
pub fn scaled(token: Pixels) -> Pixels {
    scale(token, ui_scale())
}

pub fn ui_scale() -> u16 {
    UI_SCALE.get()
}

/// Sets the UI scale (clamped to the menu's range) for every token and the library's rem-based
/// text; the caller refreshes the window.
pub fn set_ui_scale(percent: u16, cx: &mut App) {
    UI_SCALE.set(percent.clamp(UI_SCALES[0], UI_SCALES[UI_SCALES.len() - 1]));
    scale_library_text(cx);
}

/// The library sizes its text in rems of `font_size`, which `Theme::change` resets from the theme JSON.
fn scale_library_text(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    theme.font_size = scaled(UI_SM / 0.875);
    theme.mono_font_size = mono();
}

gpui_kit::actions!(theme, [ZoomIn, ZoomOut, ZoomReset]);

/// A Ctrl+= / Ctrl+- / Ctrl+0 step on a pane's mono font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zoom {
    In,
    Out,
    Reset,
}

/// The mono font size after one zoom step: 1px steps within 9..=24px; Reset goes back to MONO.
pub fn zoom(size: Pixels, step: Zoom) -> Pixels {
    let (min, max) = (px(9.), px(24.));
    match step {
        Zoom::In => (size + px(1.)).clamp(min, max),
        Zoom::Out => (size - px(1.)).clamp(min, max),
        Zoom::Reset => MONO,
    }
}

/// Enter/exit for our own overlays, eased out. Library popovers (150 ms), tooltips (150 ms)
/// and dialogs (250 ms) hard-code their durations and ignore this.
pub const MOTION_OVERLAY: Duration = Duration::from_millis(120);

// Inter and JetBrains Mono (SIL OFL 1.1, licenses next to the files). Embedded so the UI
// looks the same on every machine instead of falling back to whatever the system has.
const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
];

pub fn init(cx: &mut App) {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
        .expect("embedded fonts are valid");
    debug_assert!(
        ["Inter", "JetBrains Mono"].iter().all(|f| cx
            .text_system()
            .all_font_names()
            .iter()
            .any(|n| n == f)),
        "embedded font families not registered"
    );
    init_themes(cx);
}

/// Loads the built-in themes and starts on QueryX Dark.
fn init_themes(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for json in [DARK, LIGHT] {
        registry
            .load_themes_from_str(json)
            .expect("embedded theme is valid JSON");
    }
    let (dark, light) = (
        registry.themes()["QueryX Dark"].clone(),
        registry.themes()["QueryX Light"].clone(),
    );
    // `Theme::change` loads whichever of these two matches the mode it is given.
    Theme::update(cx, |theme| {
        (theme.dark_theme, theme.light_theme) = (dark, light)
    });
    Theme::change(ThemeMode::Dark, None, cx);
    scale_library_text(cx);
}

const fn hex(rgb: u32) -> Rgba {
    Rgba {
        r: ((rgb >> 16) & 0xff) as f32 / 255.,
        g: ((rgb >> 8) & 0xff) as f32 / 255.,
        b: (rgb & 0xff) as f32 / 255.,
        a: 1.,
    }
}

#[cfg(test)]
pub(crate) mod tests;
