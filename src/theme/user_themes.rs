use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, SharedString, Window};

use super::{BUILT_IN, scale_library_text};

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
        config.map(|c| Self {
            dir: c.join("queryx/themes"),
        })
    }
}

/// Loads every theme in `dir/*.json`, with one notification message per file or theme skipped.
pub fn load_user_themes(dir: &Path) -> (Vec<Rc<ThemeConfig>>, Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => {
            return (
                Vec::new(),
                vec![format!("Couldn't read theme folder {}: {e}", dir.display())],
            );
        }
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
        if let Err(e) =
            loaded.and_then(|json| set.load_themes_from_str(&json).map_err(|e| e.to_string()))
        {
            errors.push(format!("Couldn't load theme {file}: {e}"));
            continue;
        }
        let mut found: Vec<_> = set.themes().values().cloned().collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        for theme in found {
            if BUILT_IN.contains(&theme.name.as_ref()) {
                errors.push(format!(
                    "Theme {} in {file} clashes with a built-in theme",
                    theme.name
                ));
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
            Some((
                e.file_name(),
                meta.as_ref().and_then(|m| m.modified().ok()),
                meta.map_or(0, |m| m.len()),
            ))
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
                    *(if t.mode.is_dark() {
                        &mut dark
                    } else {
                        &mut light
                    }) = t.clone();
                    t.mode
                }
                None => ThemeMode::Dark,
            },
            Self::Dark => ThemeMode::Dark,
        };
        // `Theme::change` loads whichever of these two matches the mode it is given.
        if theme.mode != mode
            || !Rc::ptr_eq(&theme.dark_theme, &dark)
            || !Rc::ptr_eq(&theme.light_theme, &light)
        {
            Theme::update(cx, |theme| {
                (theme.dark_theme, theme.light_theme) = (dark, light)
            });
            Theme::change(mode, None, cx);
            scale_library_text(cx);
        }
    }
}
