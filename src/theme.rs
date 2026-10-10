use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{App, Hsla, Pixels, Rgba, SharedString, Window, px};
use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};

use crate::model::ObjectKind;

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
pub const UI_SM: Pixels = px(12.);
pub const UI: Pixels = px(13.);
pub const UI_LG: Pixels = px(15.);
pub const UI_XL: Pixels = px(18.);
pub const MONO: Pixels = px(12.5);

pub const SPACE_2: Pixels = px(2.);
pub const SPACE_4: Pixels = px(4.);
pub const SPACE_6: Pixels = px(6.);
pub const SPACE_8: Pixels = px(8.);
pub const SPACE_12: Pixels = px(12.);
pub const SPACE_16: Pixels = px(16.);
pub const SPACE_24: Pixels = px(24.);

/// Controls. Cards and menus use RADIUS_MD, dialogs RADIUS_LG (the theme's `radius` and `radius.lg`).
pub const RADIUS_SM: Pixels = px(4.);
pub const RADIUS_MD: Pixels = px(6.);
pub const RADIUS_LG: Pixels = px(10.);

pub const TREE_ROW: Pixels = px(24.);
/// Log and result-list rows: one line of mono SQL plus badges needs more air than the tree.
pub const LIST_ROW: Pixels = px(28.);
/// Line height inside badges, so a badge sits inside a row instead of filling it.
pub const BADGE_LINE: Pixels = px(16.);
pub const GRID_ROW: Pixels = px(26.);
pub const TAB: Pixels = px(34.);
/// Connection-color rule on tabs.
pub const STRIPE: Pixels = px(2.);
pub const TITLEBAR: Pixels = px(38.);
pub const STATUS_BAR: Pixels = px(24.);

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
        ["Inter", "JetBrains Mono"].iter().all(|f| cx.text_system().all_font_names().iter().any(|n| n == f)),
        "embedded font families not registered"
    );
    init_themes(cx);
}

/// Loads the built-in themes and starts on QueryX Dark.
fn init_themes(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for json in [DARK, LIGHT] {
        registry.load_themes_from_str(json).expect("embedded theme is valid JSON");
    }
    let (dark, light) = (registry.themes()["QueryX Dark"].clone(), registry.themes()["QueryX Light"].clone());
    // `Theme::change` loads whichever of these two matches the mode it is given.
    Theme::update(cx, |theme| (theme.dark_theme, theme.light_theme) = (dark, light));
    Theme::change(ThemeMode::Dark, None, cx);
}

/// Where users drop gpui-kit theme-set JSON files (`*.json`). QueryX only reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserThemeDir {
    pub dir: PathBuf,
}

impl UserThemeDir {
    /// `queryx/themes/` under the platform config dir; `env` reads an environment variable.
    pub fn resolve(env: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        // Empty or relative values count as unset, as the XDG spec says.
        let abs = |k: &str| env(k).map(PathBuf::from).filter(|p| p.is_absolute());
        let config = if cfg!(target_os = "macos") {
            abs("HOME").map(|h| h.join("Library/Application Support"))
        } else if cfg!(windows) {
            abs("APPDATA")
        } else {
            abs("XDG_CONFIG_HOME").or_else(|| abs("HOME").map(|h| h.join(".config")))
        };
        config.map(|c| Self { dir: c.join("queryx/themes") })
    }
}

/// Loads every theme in `dir/*.json`, with one notification message per file or theme skipped.
pub fn load_user_themes(dir: &Path) -> (Vec<Rc<ThemeConfig>>, Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => return (Vec::new(), vec![format!("Couldn't read theme folder {}: {e}", dir.display())]),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let (mut themes, mut errors) = (Vec::new(), Vec::new());
    for path in files {
        // A scratch registry parses the theme-set format without a JSON dependency of our own.
        let mut set = ThemeRegistry::default();
        let loaded = std::fs::read_to_string(&path).map_err(|e| e.to_string());
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        if let Err(e) = loaded.and_then(|json| set.load_themes_from_str(&json).map_err(|e| e.to_string())) {
            errors.push(format!("Couldn't load theme {file}: {e}"));
            continue;
        }
        let mut found: Vec<_> = set.themes().values().cloned().collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        for theme in found {
            if BUILT_IN.contains(&theme.name.as_ref()) {
                errors.push(format!("Theme {} in {file} clashes with a built-in theme", theme.name));
            } else {
                themes.push(theme);
            }
        }
    }
    (themes, errors)
}

/// Changes whenever a file in `dir` is added, removed or modified; empty when `dir` is missing.
pub fn user_theme_stamp(dir: &Path) -> Vec<(OsString, Option<std::time::SystemTime>, u64)> {
    let mut stamp: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let e = e.ok()?;
            // Follows symlinks, so an edit to a dotfiles-managed theme counts too.
            let meta = std::fs::metadata(e.path()).ok();
            Some((e.file_name(), meta.as_ref().and_then(|m| m.modified().ok()), meta.map_or(0, |m| m.len())))
        })
        .collect();
    stamp.sort();
    stamp
}

/// The user's theme setting. `System` follows the OS light/dark preference live.
/// `User` names a theme loaded from the `UserThemeDir`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    System,
    User(SharedString),
}

impl ThemeChoice {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::System];

    pub fn next(&self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::System,
            Self::System | Self::User(_) => Self::Dark,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "System",
            Self::User(name) => name,
        }
    }

    /// Applies the theme. Called again on OS appearance changes and user theme reloads;
    /// a no-op when nothing changed. A user theme that no longer exists falls back to Dark.
    pub fn apply(&self, user: &[Rc<ThemeConfig>], window: &Window, cx: &mut App) {
        let theme = cx.global::<Theme>();
        let builtin = |name: &str| ThemeRegistry::global(cx).themes().get(name).cloned();
        let mut dark = builtin(BUILT_IN[0]).unwrap_or_else(|| theme.dark_theme.clone());
        let mut light = builtin(BUILT_IN[1]).unwrap_or_else(|| theme.light_theme.clone());
        let mode = match self {
            Self::Light => ThemeMode::Light,
            Self::System => window.appearance().into(),
            Self::User(name) => match user.iter().find(|t| t.name == *name) {
                Some(t) => {
                    *(if t.mode.is_dark() { &mut dark } else { &mut light }) = t.clone();
                    t.mode
                }
                None => ThemeMode::Dark,
            },
            Self::Dark => ThemeMode::Dark,
        };
        // `Theme::change` loads whichever of these two matches the mode it is given.
        if theme.mode != mode || !Rc::ptr_eq(&theme.dark_theme, &dark) || !Rc::ptr_eq(&theme.light_theme, &light) {
            Theme::update(cx, |theme| (theme.dark_theme, theme.light_theme) = (dark, light));
            Theme::change(mode, None, cx);
        }
    }
}

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

const fn hex(rgb: u32) -> Rgba {
    Rgba {
        r: ((rgb >> 16) & 0xff) as f32 / 255.,
        g: ((rgb >> 8) & 0xff) as f32 / 255.,
        b: (rgb & 0xff) as f32 / 255.,
        a: 1.,
    }
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
    let set = if theme.is_dark() { &CONNECTION_COLORS } else { &CONNECTION_COLORS_LIGHT };
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
    theme.danger.opacity(if theme.is_dark() { 0.08 } else { 0.045 })
}

/// The `PROD` hazard label next to a production connection, wherever its name is shown.
/// Solid, not outlined: a label stuck on the thing, not one more box. Page-colored mono text on
/// `danger` holds 6.2:1 dark and 5.1:1 light (tested as `background` on `danger.background`).
pub fn prod_badge(theme: &Theme) -> gpui_kit::Div {
    use gpui_kit::{FontWeight, ParentElement as _, Styled as _};
    gpui_kit::div()
        .flex_none()
        .px(SPACE_4)
        .rounded(RADIUS_SM)
        .bg(theme.danger)
        .text_color(theme.background)
        .font_family(theme.mono_font_family.clone())
        .font_weight(FontWeight::BOLD)
        .text_size(UI_SM)
        .line_height(BADGE_LINE)
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
pub const SYNTAX: [&str; 8] = ["keyword", "string", "number", "comment", "function", "type", "operator", "variable"];

#[cfg(test)]
pub(crate) mod tests {
    use super::{CONNECTION_COLORS, CONNECTION_COLORS_LIGHT, DARK, LIGHT, Rgba, Zoom, px, zoom};

    #[test]
    fn zoom_steps_1px_within_9_to_24_and_resets_to_12_5() {
        use Zoom::*;
        let cases = [
            (12.5, In, 13.5),
            (24., In, 24.),
            (23.5, In, 24.),
            (23., In, 24.),
            (9.5, Out, 9.),
            (9., Out, 9.),
            (10., Out, 9.),
            (13.5, Out, 12.5),
            (9., In, 10.),
            (24., Out, 23.),
            (24., Reset, 12.5),
            (9., Reset, 12.5),
            (12.5, Reset, 12.5),
            // Out of range (never produced by zoom itself) lands back inside it.
            (30., In, 24.),
            (30., Out, 24.),
            (5., Out, 9.),
            (5., In, 9.),
        ];
        for (size, step, want) in cases {
            assert_eq!(zoom(px(size), step), px(want), "{size} {step:?}");
        }
    }

    fn rgb(hex: &str) -> [f64; 3] {
        [0, 2, 4].map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64 / 255.)
    }

    /// `#RRGGBB` value of `key` in a theme JSON.
    fn color(theme: &str, key: &str) -> [f64; 3] {
        let pat = format!("\"{key}\": \"#");
        let at = theme.find(&pat).unwrap_or_else(|| panic!("{key} missing")) + pat.len();
        rgb(&theme[at..at + 6])
    }

    /// WCAG 2 contrast ratio.
    fn ratio(a: [f64; 3], b: [f64; 3]) -> f64 {
        let lum = |c: [f64; 3]| {
            let [r, g, b] = c.map(|v| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (lum(a), lum(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn text_tokens_meet_contrast_targets() {
        let text = [
            ("foreground", "background"),
            ("muted.foreground", "background"),
            ("muted.foreground", "sidebar.background"),
            ("muted.foreground", "muted.background"),
            ("muted.foreground", "list.hover.background"),
            ("muted.foreground", "table.hover.background"),
            ("muted.foreground", "status_bar.background"),
            ("muted.foreground", "tab_bar.background"),
            ("sidebar.foreground", "sidebar.background"),
            ("tab.foreground", "tab_bar.background"),
            ("table.head.foreground", "table.head.background"),
            ("popover.foreground", "popover.background"),
            ("primary.foreground", "primary.background"),
            ("danger.foreground", "background"),
            ("danger.foreground", "sidebar.background"),
            ("background", "danger.background"),
            ("foreground", "muted.background"),
            ("foreground", "list.hover.background"),
            ("tab.active.foreground", "tab.active.background"),
            ("muted.foreground", "sidebar.accent.background"),
        ];
        // Status text on the surfaces it is drawn on, hover rows included.
        let status = ["success", "warning", "info", "danger"];
        let surfaces = ["background", "sidebar.background", "muted.background", "list.hover.background", "table.hover.background"];
        let text: Vec<(String, &str)> = text.iter().map(|&(f, b)| (f.to_string(), b)).chain(
            status.iter().flat_map(|s| surfaces.iter().map(move |b| (format!("{s}.foreground"), *b))),
        ).collect();
        for (name, theme) in [("dark", DARK), ("light", LIGHT)] {
            for (fg, bg) in &text {
                let r = ratio(color(theme, fg), color(theme, bg));
                assert!(r >= 4.5, "{name}: {fg} on {bg} is {r:.2}:1, needs 4.5:1");
            }
            for bg in ["table.background", "table.even.background", "table.hover.background"] {
                let r = ratio(color(theme, "foreground"), color(theme, bg));
                assert!(r >= 7., "{name}: grid text on {bg} is {r:.2}:1, needs 7:1");
            }
        }
    }

    /// Error text is drawn as `theme.danger` (= danger.background).
    #[test]
    fn drawn_status_and_null_text_meet_contrast() {
        let pairs = [
            ("danger.background", "background"),
            ("danger.background", "sidebar.background"),
            ("danger.background", "muted.background"),
        ];
        for (name, theme) in [("dark", DARK), ("light", LIGHT)] {
            for (fg, bg) in pairs {
                let r = ratio(color(theme, fg), color(theme, bg));
                assert!(r >= 4.5, "{name}: {fg} on {bg} is {r:.2}:1, needs 4.5:1");
            }
        }
    }

    #[test]
    fn grid_null_text_meets_contrast() {
        for (name, theme) in [("dark", DARK), ("light", LIGHT)] {
            for bg in ["table.background", "table.even.background"] {
                let r = ratio(color(theme, "muted.foreground"), color(theme, bg));
                assert!(r >= 4.5, "{name}: grid NULL on {bg} is {r:.2}:1, needs 4.5:1");
            }
        }
    }

    /// Dots and tab stripes are UI components: 3:1 against every surface they sit on.
    #[test]
    fn connection_colors_read_on_their_theme() {
        let rgba = |c: Rgba| [c.r as f64, c.g as f64, c.b as f64];
        for (name, theme, set) in [("dark", DARK, CONNECTION_COLORS), ("light", LIGHT, CONNECTION_COLORS_LIGHT)] {
            for bg in ["background", "sidebar.background", "tab_bar.background", "status_bar.background", "table.head.background"] {
                for (i, c) in set.iter().enumerate() {
                    let r = ratio(rgba(*c), color(theme, bg));
                    assert!(r >= 3., "{name}: connection color {i} on {bg} is {r:.2}:1, needs 3:1");
                }
            }
        }
    }

    #[test]
    fn themes_define_every_syntax_token() {
        for theme in [DARK, LIGHT] {
            let syntax = theme.split("\"syntax\"").nth(1).expect("theme has highlight.syntax");
            for key in super::SYNTAX {
                assert!(syntax.contains(&format!("\"{key}\": {{")), "syntax.{key} missing from theme JSON");
            }
        }
    }

    #[test]
    fn light_theme_has_the_dark_key_set() {
        let keys = |t: &'static str| {
            let parts: Vec<&str> = t.split('"').collect();
            parts.windows(2).filter(|w| w[1].trim_start().starts_with(':')).map(|w| w[0]).collect::<Vec<_>>()
        };
        assert_eq!(keys(DARK), keys(LIGHT));
    }

    /// Library `text_sm` must land on UI_SM (see the type scale note); mono on MONO.
    #[test]
    fn theme_font_sizes_match_the_type_scale() {
        let num = |theme: &str, key: &str| -> f32 {
            let pat = format!("\"{key}\": ");
            let at = theme.find(&pat).unwrap_or_else(|| panic!("{key} missing")) + pat.len();
            theme[at..].split(',').next().unwrap().trim().parse().unwrap()
        };
        for theme in [DARK, LIGHT] {
            assert!((num(theme, "font.size") * 0.875 - f32::from(super::UI_SM)).abs() < 0.01, "font.size * 0.875 != UI_SM");
            assert_eq!(num(theme, "mono_font.size"), f32::from(super::MONO));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn user_theme_dir_follows_xdg_then_home() {
        use super::UserThemeDir;
        use std::path::PathBuf;
        // (XDG_CONFIG_HOME, HOME, expected dir)
        let cases: [(Option<&str>, Option<&str>, Option<&str>); 7] = [
            (Some("/x"), Some("/h"), Some("/x/queryx/themes")),
            (None, Some("/h"), Some("/h/.config/queryx/themes")),
            (Some(""), Some("/h"), Some("/h/.config/queryx/themes")), // empty counts as unset (XDG spec)
            (Some("rel"), Some("/h"), Some("/h/.config/queryx/themes")), // relative is ignored (XDG spec)
            (Some("/x"), None, Some("/x/queryx/themes")),
            (None, None, None),
            (None, Some(""), None),
        ];
        for (xdg, home, want) in cases {
            let env = |k: &str| match k {
                "XDG_CONFIG_HOME" => xdg.map(Into::into),
                "HOME" => home.map(Into::into),
                _ => None,
            };
            let got = UserThemeDir::resolve(env).map(|d| d.dir);
            assert_eq!(got, want.map(PathBuf::from), "XDG_CONFIG_HOME={xdg:?} HOME={home:?}");
        }
    }

    /// A fresh dir under the workspace's `target/`, never the user's config dir.
    pub(crate) fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-themes").join(name);
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_user_theme_dir_loads_nothing_and_reports_nothing() {
        let dir = test_dir("missing").join("themes");
        let (themes, errors) = super::load_user_themes(&dir);
        assert!(themes.is_empty(), "themes from a missing dir");
        assert_eq!(errors, Vec::<String>::new(), "a missing dir is not an error");
        assert!(!dir.exists(), "loading must not create the dir");
    }

    #[test]
    fn unreadable_theme_dir_is_reported_unlike_a_missing_one() {
        let dir = test_dir("not-a-dir").join("themes");
        std::fs::write(&dir, "").unwrap(); // a file where the folder should be: ENOTDIR
        let (themes, errors) = super::load_user_themes(&dir);
        assert!(themes.is_empty());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with(&format!("Couldn't read theme folder {}: ", dir.display())), "{errors:?}");
    }

    #[test]
    fn theme_file_that_is_not_text_is_skipped_and_reported() {
        let dir = test_dir("not-utf8");
        std::fs::write(dir.join("bad.json"), [0xFF, 0xFE]).unwrap();
        std::fs::write(dir.join("nord.json"), NORD).unwrap();
        let (themes, errors) = super::load_user_themes(&dir);
        assert_eq!(names(&themes), ["Nord"]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("Couldn't load theme bad.json: ") && errors[0].len() > 30, "{errors:?}");
    }

    const NORD: &str = r##"{"name": "Nord", "themes": [{"name": "Nord", "mode": "dark", "colors": {"background": "#2E3440"}}]}"##;
    const PAPER: &str = r##"{"name": "Paper", "themes": [{"name": "Paper", "mode": "light", "colors": {"background": "#F7F3E8"}}]}"##;

    fn names(themes: &[std::rc::Rc<gpui_kit::component::theme::ThemeConfig>]) -> Vec<String> {
        themes.iter().map(|t| t.name.to_string()).collect()
    }

    #[test]
    fn valid_user_theme_file_is_listed() {
        let dir = test_dir("valid");
        std::fs::write(dir.join("nord.json"), NORD).unwrap();
        let (themes, errors) = super::load_user_themes(&dir);
        assert_eq!((names(&themes), errors), (vec!["Nord".to_string()], vec![]));
    }

    /// Each case is a fresh dir: (files written into it, theme names loaded, errors expected).
    /// An expected error ending in ": " is matched as a prefix followed by the parse error.
    #[test]
    fn user_theme_files_load_skip_or_report() {
        const TWO: &str = r##"{"name": "Set", "themes": [{"name": "B", "mode": "dark"}, {"name": "A", "mode": "light"}]}"##;
        let bad = "Couldn't load theme bad.json: ";
        let cases: [(&[(&str, &str)], &[&str], &[&str]); 11] = [
            (&[("bad.json", "not json")], &[], &[bad]),
            (&[("x.txt", NORD)], &[], &[]), // valid theme, wrong extension
            (&[("nord", NORD)], &[], &[]), // no extension
            (&[("NORD.JSON", NORD)], &[], &[]), // does not end in `.json`
            (&[("nord.json", NORD), ("bad.json", "{")], &["Nord"], &[bad]), // a broken file doesn't stop the rest
            (&[("bad.json", "")], &[], &[bad]),
            (&[("bad.json", r#"{"themes": 5}"#)], &[], &[bad]), // JSON, but not a theme set
            (&[("empty.json", "{}")], &[], &[]), // a theme set with no themes
            (&[("two.json", TWO)], &["A", "B"], &[]),
            (&[("a.json", "["), ("b.json", "[")], &[], &["Couldn't load theme a.json: ", "Couldn't load theme b.json: "]),
            (&[], &[], &[]), // empty dir
        ];
        for (ix, (files, want_names, want_errors)) in cases.into_iter().enumerate() {
            let dir = test_dir(&format!("files-{ix}"));
            for (name, json) in files {
                std::fs::write(dir.join(name), json).unwrap();
            }
            let (themes, errors) = super::load_user_themes(&dir);
            assert_eq!(names(&themes), want_names, "themes from {files:?}");
            assert_eq!(errors.len(), want_errors.len(), "errors from {files:?}: {errors:?}");
            for (got, want) in errors.iter().zip(want_errors) {
                assert!(got.starts_with(want) && got.len() > want.len(), "error {got:?} should be {want:?} + the parse error");
            }
        }
        // A directory named like a theme file is not a file.
        let dir = test_dir("files-dir");
        std::fs::create_dir(dir.join("folder.json")).unwrap();
        let (themes, errors) = super::load_user_themes(&dir);
        assert_eq!((names(&themes), errors), (vec![], vec![]), "a dir named folder.json");
    }

    #[test]
    fn user_theme_named_like_a_built_in_is_skipped_and_reported() {
        let set = |names: &[&str]| {
            let themes: Vec<String> = names.iter().map(|n| format!(r#"{{"name": "{n}", "mode": "dark"}}"#)).collect();
            format!(r#"{{"name": "Set", "themes": [{}]}}"#, themes.join(","))
        };
        let clash = |theme: &str, file: &str| format!("Theme {theme} in {file} clashes with a built-in theme");
        // (file, themes in it, themes loaded, errors)
        let cases = [
            ("dark.json", vec!["QueryX Dark"], vec![], vec![clash("QueryX Dark", "dark.json")]),
            ("light.json", vec!["QueryX Light"], vec![], vec![clash("QueryX Light", "light.json")]),
            ("mixed.json", vec!["Nord", "QueryX Dark"], vec!["Nord"], vec![clash("QueryX Dark", "mixed.json")]), // the rest of the file loads
            ("both.json", vec!["QueryX Dark", "QueryX Light"], vec![], vec![clash("QueryX Dark", "both.json"), clash("QueryX Light", "both.json")]),
            ("case.json", vec!["queryx dark"], vec!["queryx dark"], vec![]), // names match exactly
        ];
        for (ix, (file, inside, want_names, want_errors)) in cases.into_iter().enumerate() {
            let dir = test_dir(&format!("clash-{ix}"));
            std::fs::write(dir.join(file), set(&inside)).unwrap();
            let (themes, errors) = super::load_user_themes(&dir);
            assert_eq!((names(&themes), errors), (want_names.iter().map(|n| n.to_string()).collect(), want_errors), "{file} with {inside:?}");
        }
    }

    /// Choosing a user theme applies it at once, in its own mode; choosing a built-in goes back.
    #[gpui_kit::test]
    fn choosing_a_user_theme_applies_it_immediately(cx: &mut gpui_kit::TestAppContext) {
        use super::ThemeChoice;
        use gpui_kit::component::ActiveTheme as _;
        let dir = test_dir("choose");
        std::fs::write(dir.join("nord.json"), NORD).unwrap();
        std::fs::write(dir.join("paper.json"), PAPER).unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            super::init_themes(cx);
        });
        let (themes, _) = super::load_user_themes(&dir);
        let cx = cx.add_empty_window();
        // (choice, theme name after applying it, dark?, background)
        let cases = [
            (ThemeChoice::User("Nord".into()), "Nord", true, Some(0x2E3440)),
            (ThemeChoice::Light, "QueryX Light", false, None),
            (ThemeChoice::User("Paper".into()), "Paper", false, Some(0xF7F3E8)),
            (ThemeChoice::User("Nord".into()), "Nord", true, Some(0x2E3440)), // light -> user dark
            (ThemeChoice::Dark, "QueryX Dark", true, None), // same mode, back to the built-in
            (ThemeChoice::User("Gone".into()), "QueryX Dark", true, None), // removed theme falls back to Dark
        ];
        for (choice, name, dark, bg) in cases {
            let (got, is_dark, background) = cx.update(|window, cx| {
                choice.apply(&themes, window, cx);
                (cx.theme().theme_name().to_string(), cx.theme().is_dark(), cx.theme().background)
            });
            assert_eq!((got.as_str(), is_dark), (name, dark), "after {choice:?}");
            if let Some(bg) = bg {
                assert_eq!(background, gpui_kit::Hsla::from(super::hex(bg)), "background after {choice:?}");
            }
        }
    }

    #[test]
    fn theme_choice_cycles_through_all_three() {
        use super::ThemeChoice::*;
        assert_eq!([Dark.next(), Light.next(), System.next(), User("Nord".into()).next()], [Light, System, Dark, Dark]);
    }
}
