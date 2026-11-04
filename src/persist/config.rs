//! `config.toml` (C7): theme, UI scale, console font size and whether the welcome tour ran.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::theme::{self, ThemeChoice};

/// The settings `config.toml` holds; each value is valid (bad ones load as the default).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub theme: ThemeChoice,
    /// One of `theme::UI_SCALES`.
    pub ui_scale: u16,
    /// The console's mono size after zoom, before UI scale.
    pub console_font_px: f32,
    /// The welcome tour ran (finished or skipped); false only on a fresh install.
    pub onboarded: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            ui_scale: theme::DEFAULT_UI_SCALE,
            console_font_px: theme::MONO.into(),
            onboarded: true,
        }
    }
}

/// Every field optional on read: a missing one loads as the default.
#[derive(Serialize, Deserialize)]
struct FileToml {
    version: Option<u32>,
    theme: Option<String>,
    ui_scale: Option<u16>,
    console_font_px: Option<f32>,
    onboarded: Option<bool>,
}

const USER: &str = "user:";

/// Loads `<root>/config.toml`; defaults when absent, with the welcome tour still to run. A corrupt
/// file is moved to `.bak` and the notice says so.
pub fn load_config(root: &Path) -> (Config, Option<String>) {
    match super::read_or_backup(&root.join("config.toml"), config_from_toml) {
        Ok(config) => (
            config.unwrap_or(Config {
                onboarded: false,
                ..Config::default()
            }),
            None,
        ),
        Err(notice) => (Config::default(), Some(notice)),
    }
}

pub fn save_config(root: &Path, config: &Config) -> std::io::Result<()> {
    super::write_atomic(&root.join("config.toml"), config_to_toml(config).as_bytes())
}

fn config_to_toml(config: &Config) -> String {
    let theme = match &config.theme {
        ThemeChoice::User(name) => format!("{USER}{name}"),
        builtin => builtin.label().to_lowercase(),
    };
    let file = FileToml {
        version: Some(1),
        theme: Some(theme),
        ui_scale: Some(config.ui_scale),
        console_font_px: Some(config.console_font_px),
        onboarded: Some(config.onboarded),
    };
    toml::to_string(&file).expect("config serializes")
}

fn config_from_toml(text: &str) -> Result<Config, toml::de::Error> {
    let file: FileToml = toml::from_str(text)?;
    let default = Config::default();
    let theme = match file.theme.as_deref() {
        Some(name) if name.len() > USER.len() && name.starts_with(USER) => {
            ThemeChoice::User(name[USER.len()..].to_string().into())
        }
        Some(name) => ThemeChoice::ALL
            .into_iter()
            .find(|t| t.label().eq_ignore_ascii_case(name))
            .unwrap_or(default.theme),
        None => default.theme,
    };
    Ok(Config {
        theme,
        ui_scale: file
            .ui_scale
            .filter(|s| theme::UI_SCALES.contains(s))
            .unwrap_or(default.ui_scale),
        // The zoom range (`theme::zoom`).
        console_font_px: file
            .console_font_px
            .filter(|px| (9.0..=24.0).contains(px))
            .unwrap_or(default.console_font_px),
        // A file from before the tour existed belongs to someone who already uses TableX.
        onboarded: file.onboarded.unwrap_or(default.onboarded),
    })
}
