use std::rc::Rc;

use gpui_kit::component::{
    IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};
use gpui_kit::*;

use crate::{persist, theme};

use super::Shell;

const THEME_HELP: &str = "Colors for the whole window. System follows the OS light or dark mode; \
    themes in the themes folder next to config.toml are listed after the built-ins.";
const FONT_HELP: &str =
    "The console editor's font size in pixels, before UI scale. Ctrl+= and Ctrl+- change it too.";

impl Shell {
    /// The size the console editor is given: its zoom at the current UI scale.
    pub(super) fn console_text_size(&self) -> Pixels {
        theme::scaled(self.console_mono)
    }

    pub(super) fn zoom_console(
        &mut self,
        step: theme::Zoom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.console_mono = theme::zoom(self.console_mono, step);
        self.save_config(window, cx);
        cx.notify();
    }

    /// Applies `config.toml` as `load_config` read it from `root`, and saves every later theme, UI
    /// scale or console zoom change there.
    pub(super) fn persist_config(
        &mut self,
        root: &std::path::Path,
        (config, notice): (persist::Config, Option<String>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A user theme that doesn't exist falls back to Dark once the theme dir is read.
        self.theme = config.theme;
        self.theme.apply(&self.user_themes, window, cx);
        theme::set_ui_scale(config.ui_scale, cx);
        self.console_mono = px(config.console_font_px);
        self.config_root = Some(root.to_path_buf());
        if !config.onboarded {
            self.open_welcome(cx);
        }
        if let Some(notice) = notice {
            // Deferred: this runs while the window is being built, and notifications need its Root.
            cx.defer_in(window, move |_, window, cx| {
                crate::ui::toast::error(notice, window, cx);
            });
        }
    }

    /// Writes `config.toml` with the current theme, UI scale and console font size.
    pub(super) fn save_config(&self, window: &mut Window, cx: &mut App) {
        let Some(root) = &self.config_root else {
            return;
        };
        let config = persist::Config {
            theme: self.theme.clone(),
            ui_scale: theme::ui_scale(),
            console_font_px: self.console_mono.into(),
            // A theme picked mid-tour is saved; the tour still runs next start until it's closed.
            onboarded: self.welcome.is_none(),
        };
        if let Err(e) = persist::save_config(root, &config) {
            let message = format!("Couldn't save {}: {e}", root.join("config.toml").display());
            crate::ui::toast::error(message, window, cx);
        }
    }

    /// Loads user themes from `dir` and reloads them when its files change. Never writes to `dir`.
    pub(super) fn watch_user_themes(
        &mut self,
        dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // shortcut: polls the dir's listing once a second instead of a file watcher. gpui-kit's
        // `ThemeRegistry::watch_dir` creates the dir and drops our built-ins on reload, and `notify`
        // isn't a dependency; switch to a watcher if polling ever shows up in a profile.
        let mut stamp = theme::user_theme_stamp(&dir);
        // Deferred: this runs while the window is being built, and notifications need its Root.
        let first = dir.clone();
        cx.defer_in(window, move |this, window, cx| {
            this.reload_user_themes(&first, window, cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let now = theme::user_theme_stamp(&dir);
                let changed = now != stamp;
                stamp = now;
                let alive = this.update_in(cx, |this, window, cx| {
                    if changed {
                        this.reload_user_themes(&dir, window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Lists the themes in `dir`, notifies about skipped files and re-applies the active theme.
    pub(super) fn reload_user_themes(
        &mut self,
        dir: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (themes, errors) = theme::load_user_themes(dir);
        // Only new problems: an unrelated edit shouldn't repeat every notification.
        for error in errors
            .iter()
            .filter(|e| !self.user_theme_errors.contains(e))
        {
            crate::ui::toast::error(error.clone(), window, cx);
        }
        self.user_theme_errors = errors;
        self.user_themes = themes;
        if let theme::ThemeChoice::User(name) = &self.theme
            && !self.user_themes.iter().any(|t| t.name == *name)
        {
            self.theme = theme::ThemeChoice::Dark;
        }
        self.theme.apply(&self.user_themes, window, cx);
        cx.notify();
    }

    /// Switches to `choice` and saves it.
    pub(super) fn set_theme(
        &mut self,
        choice: theme::ThemeChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.theme = choice;
        self.theme.apply(&self.user_themes, window, cx);
        self.save_config(window, cx);
        cx.notify();
    }

    /// ⚙ in the titlebar: opens Settings.
    pub(super) fn settings_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("settings")
            .debug_selector(|| "settings".into())
            .ghost()
            .small()
            .icon(IconName::Settings)
            .tooltip("Settings")
            .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)))
    }

    /// Sets the UI scale (one of `theme::UI_SCALES`) and saves it.
    pub(super) fn set_ui_scale(
        &mut self,
        percent: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        theme::set_ui_scale(percent, cx);
        self.save_config(window, cx);
        window.refresh();
    }

    /// Sets the console's mono size (before UI scale) and saves it.
    pub(super) fn set_console_font(
        &mut self,
        size: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.console_mono = size;
        self.save_config(window, cx);
        cx.notify();
    }

    /// The Settings dialog: a sidebar of pages with searchable rows. Every change applies and
    /// saves at once, so there is no Save button.
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shell = cx.entity();
        let handle = window.window_handle();
        // Setting fields only get `App`; the change runs once the window is free again.
        let update =
            move |cx: &mut App, f: Rc<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>| {
                let shell = shell.downgrade();
                cx.defer(move |cx| {
                    _ = handle.update(cx, |_, window, cx| {
                        shell.update(cx, |this, cx| f(this, window, cx))
                    });
                });
            };
        let config = self
            .config_root
            .as_ref()
            .map(|root| root.join("config.toml"));
        let shell = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            // User themes from the user theme dir (`<ConfigDir>/themes`), after the built-ins.
            let choices: Vec<_> = theme::ThemeChoice::ALL
                .into_iter()
                .chain(
                    shell
                        .read(cx)
                        .user_themes
                        .iter()
                        .map(|t| theme::ThemeChoice::User(t.name.clone())),
                )
                .collect();
            let options = choices
                .iter()
                .enumerate()
                .map(|(ix, c)| (ix.to_string().into(), c.label().to_string().into()))
                .collect();
            let theme_field = {
                let (shell, set) = (shell.clone(), update.clone());
                let current = choices.clone();
                SettingField::scrollable_dropdown(
                    options,
                    move |cx| {
                        let theme = &shell.read(cx).theme;
                        let ix = current.iter().position(|c| c == theme).unwrap_or(0);
                        ix.to_string().into()
                    },
                    move |ix: SharedString, cx| {
                        let Some(choice) = ix.parse().ok().and_then(|ix: usize| choices.get(ix))
                        else {
                            return;
                        };
                        let choice = choice.clone();
                        set(
                            cx,
                            Rc::new(move |this, window, cx| {
                                this.set_theme(choice.clone(), window, cx)
                            }),
                        );
                    },
                )
                .default_value("0")
            };
            let scale_field = {
                let set = update.clone();
                SettingField::dropdown(
                    theme::UI_SCALES
                        .into_iter()
                        .map(|p| (p.to_string().into(), format!("{p}%").into()))
                        .collect(),
                    |_| theme::ui_scale().to_string().into(),
                    move |percent: SharedString, cx| {
                        let Ok(percent) = percent.parse() else {
                            return;
                        };
                        set(
                            cx,
                            Rc::new(move |this, window, cx| this.set_ui_scale(percent, window, cx)),
                        );
                    },
                )
                .default_value(theme::DEFAULT_UI_SCALE.to_string())
            };
            let font_field = {
                let (shell, set) = (shell.clone(), update.clone());
                SettingField::number_input(
                    // The zoom range (`theme::zoom`).
                    NumberFieldOptions {
                        min: 9.,
                        max: 24.,
                        step: 1.,
                    },
                    move |cx| f32::from(shell.read(cx).console_mono) as f64,
                    move |size: f64, cx| {
                        let size = px(size as f32);
                        set(
                            cx,
                            Rc::new(move |this, window, cx| {
                                this.set_console_font(size, window, cx)
                            }),
                        );
                    },
                )
                .default_value(f32::from(theme::MONO) as f64)
            };
            let settings = Settings::new("settings")
                .sidebar_width(px(200.))
                .page(
                    SettingPage::new("Appearance")
                        .resettable(true)
                        .group(
                            SettingGroup::new().title("Theme").item(
                                SettingItem::new("Theme", theme_field)
                                    .description(THEME_HELP)
                                    .keywords(["color", "dark", "light"]),
                            ),
                        )
                        .group(
                            SettingGroup::new().title("Scale").item(
                                SettingItem::new("UI scale", scale_field)
                                    .description("Size of all text and controls.")
                                    .keywords(["zoom", "size"]),
                            ),
                        ),
                )
                .page(
                    SettingPage::new("Console").resettable(true).group(
                        SettingGroup::new().title("Editor").item(
                            SettingItem::new("Font size", font_field)
                                .description(FONT_HELP)
                                .keywords(["zoom", "text"]),
                        ),
                    ),
                );
            // With p_0 the close button sits in the corner; a title row keeps it off the page's
            // Reset button.
            let dialog = dialog
                .w(px(820.))
                .p_0()
                .title(
                    div()
                        .px(theme::space_16())
                        .pt(theme::space_12())
                        .child("Settings"),
                )
                .child(
                    div()
                        .debug_selector(|| "settings".into())
                        .h(px(520.))
                        .child(settings),
                );
            match config.clone() {
                None => dialog,
                Some(path) => dialog.footer(
                    h_flex().px(theme::space_16()).pb(theme::space_16()).child(
                        Button::new("settings-open-config")
                            .ghost()
                            .small()
                            .icon(IconName::ExternalLink)
                            .label("Open config.toml")
                            .on_click(move |_, _, cx| cx.open_with_system(&path)),
                    ),
                ),
            }
        });
    }
}
