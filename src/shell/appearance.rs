use gpui_kit::component::{
    IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::*;

use crate::theme;

use super::Shell;

impl Shell {
    /// The size the console editor is given: its zoom at the current UI scale.
    pub(super) fn console_text_size(&self) -> Pixels {
        theme::scaled(self.console_mono)
    }

    pub(super) fn zoom_console(&mut self, step: theme::Zoom, cx: &mut Context<Self>) {
        self.console_mono = theme::zoom(self.console_mono, step);
        cx.notify();
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
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        let (themes, errors) = theme::load_user_themes(dir);
        // Only new problems: an unrelated edit shouldn't repeat every notification.
        for error in errors
            .iter()
            .filter(|e| !self.user_theme_errors.contains(e))
        {
            window.push_notification(Notification::error(error.clone()), cx);
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

    /// ⚙ in the titlebar. Holds the theme and UI scale until there is a settings view (QX-020);
    /// `Ctrl+K Ctrl+T` still cycles the theme from the keyboard.
    pub(super) fn settings_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let current = self.theme.clone();
        // User themes from the user theme dir (`<ConfigDir>/themes`), under the built-ins.
        let user = self
            .user_themes
            .iter()
            .map(|t| theme::ThemeChoice::User(t.name.clone()));
        let choices: Vec<_> = theme::ThemeChoice::ALL.into_iter().chain(user).collect();
        Button::new("settings")
            .ghost()
            .small()
            .icon(IconName::Settings)
            .tooltip("Settings")
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.label("Theme");
                for (ix, choice) in choices.iter().cloned().enumerate() {
                    if ix == theme::ThemeChoice::ALL.len() {
                        menu = menu.separator();
                    }
                    let shell = shell.clone();
                    menu = menu.item(
                        PopupMenuItem::new(choice.label())
                            .checked(choice == current)
                            .on_click(move |_, window, cx| {
                                _ = shell.update(cx, |this, cx| {
                                    this.theme = choice.clone();
                                    this.theme.apply(&this.user_themes, window, cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu = menu.separator().label("UI scale");
                for percent in theme::UI_SCALES {
                    menu = menu.item(
                        PopupMenuItem::new(format!("{percent}%"))
                            .checked(percent == theme::ui_scale())
                            .on_click(move |_, window, cx| {
                                theme::set_ui_scale(percent, cx);
                                window.refresh();
                            }),
                    );
                }
                menu
            })
            .anchor(Anchor::TopRight)
    }
}
