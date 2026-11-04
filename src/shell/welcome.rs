//! The welcome tour: a walk through the look, the keys and adding the first connection.
//! Runs once on a fresh install (no config.toml yet) and again from the palette (`Welcome`).

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::theme::{Theme as PreviewTheme, ThemeRegistry};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    kbd::Kbd,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::connection_dialog::{ConnectionDialog, Saved};
use crate::{assets, keymap::Bindings, theme};

use super::Shell;

pub(super) struct Welcome {
    pub(super) step: usize,
    pub(super) focus: FocusHandle,
    /// First run: the window opened at the tour's size and grows to full size when it closes.
    pub(super) grow_window: bool,
    /// The last step's Add connection form, made on first reaching it with no connections.
    form: Option<(Entity<ConnectionDialog>, Subscription)>,
}

/// The window's size when it opens for the tour on first run.
pub(super) const TOUR_WINDOW: Size<Pixels> = size(px(860.), px(640.));

const STEPS: usize = 4;
const LAST: usize = STEPS - 1;

/// Shortcuts the keyboard step teaches: (action, what it does).
const KEYS: [(&str, &str, &str); 5] = [
    (
        "tablex::QuickOpen",
        "Quick Open",
        "Jump to a table, view or function",
    ),
    (
        "tablex::CommandPalette",
        "Command Palette",
        "Find and run a command by name",
    ),
    (
        "tablex::NewConsole",
        "New SQL console",
        "Open a console for the selection",
    ),
    (
        "tablex::ToggleExplorer",
        "Toggle Explorer",
        "Show or hide the connection browser",
    ),
    (
        "tablex::CheatSheet",
        "Shortcut reference",
        "Browse the full list of shortcuts",
    ),
];

impl Shell {
    pub(super) fn open_welcome(&mut self, cx: &mut Context<Self>) {
        if self.welcome.is_none() {
            self.welcome = Some(Welcome {
                step: 0,
                focus: cx.focus_handle(),
                grow_window: false,
                form: None,
            });
            cx.notify();
        }
    }

    /// Finishes or skips the tour; it won't run on its own again.
    pub(super) fn close_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(welcome) = self.welcome.take() {
            if welcome.grow_window && !window.is_maximized() {
                window.zoom_window();
            }
            self.save_config(window, cx);
            self.focus_content = true;
            cx.notify();
        }
    }

    /// Moves `by` steps; past the last one finishes the tour.
    fn welcome_go(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let no_connections = self.no_connections(cx);
        let Some(welcome) = &mut self.welcome else {
            return;
        };
        match welcome.step.checked_add_signed(by) {
            Some(step) if step <= LAST => welcome.step = step,
            Some(_) => return self.close_welcome(window, cx),
            None => {}
        }
        if welcome.step == LAST && welcome.form.is_none() && no_connections {
            let form =
                ConnectionDialog::embedded(self.workspace.clone(), self.driver.clone(), window, cx);
            // Saved: the tour is done; show the new connection in the explorer.
            let saved = cx.subscribe_in(&form, window, |this, _, Saved(id), window, cx| {
                this.close_welcome(window, cx);
                this.layout.show_explorer();
                this.explorer
                    .update(cx, |e, cx| e.reveal(&id.0.to_string(), window, cx));
            });
            if let Some(welcome) = &mut self.welcome {
                welcome.form = Some((form, saved));
            }
        }
        cx.notify();
    }

    /// The form on the last step, while there is one to fill.
    pub(super) fn welcome_form(&self) -> Option<&Entity<ConnectionDialog>> {
        let welcome = self.welcome.as_ref().filter(|w| w.step == LAST)?;
        welcome.form.as_ref().map(|(form, _)| form)
    }

    /// Adds the form's connection; an invalid form shows its errors instead.
    fn welcome_connect(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.welcome_form().cloned() {
            form.update(cx, |form, cx| form.save(cx));
        }
    }

    fn no_connections(&self, cx: &App) -> bool {
        self.workspace.read(cx).connections.is_empty()
    }

    pub(super) fn welcome_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(welcome) = &self.welcome else {
            return div().into_any_element();
        };
        let step = welcome.step;
        let form = self.welcome_form().cloned();
        let connect = form.is_some();
        let content = match step {
            0 => self.intro(cx),
            1 => self.looks(cx),
            2 => keys(cx),
            _ => connect_step(form.as_ref(), cx),
        };
        let theme = cx.theme();
        let content = div().w_full().child(content);
        // Each step slides in under its own id, so the animation replays on every move.
        let content = if cx.reduce_motion() {
            content.into_any_element()
        } else {
            content
                .with_animation(
                    ElementId::NamedInteger("welcome-step".into(), step as u64),
                    Animation::new(Duration::from_millis(320)).with_easing(ease_out_quint()),
                    |d, t| d.opacity(t).mt(px(16.) * (1. - t)),
                )
                .into_any_element()
        };
        let stage_names = ["Welcome", "Theme", "Shortcuts", "Connect"];
        let progress = h_flex()
            .items_center()
            .gap(theme::space_8())
            .child(
                h_flex()
                    .items_center()
                    .gap(theme::space_4())
                    .children((0..STEPS).map(|ix| {
                        div()
                            .w(if ix == step { px(24.) } else { px(16.) })
                            .h(px(4.))
                            .rounded_full()
                            .bg(if ix <= step {
                                theme.primary
                            } else {
                                theme.border
                            })
                    })),
            )
            .child(
                div()
                    .text_size(theme::ui_sm())
                    .text_color(theme.foreground)
                    .child(stage_names[step]),
            )
            .child(
                div()
                    .text_size(theme::ui_sm())
                    .text_color(theme.muted_foreground)
                    .child(format!("{:02} / {:02}", step + 1, STEPS)),
            );
        let (next_label, next_icon) = match step {
            0 => ("Get started", Some(IconName::ArrowRight)),
            LAST if connect => ("Add connection", Some(IconName::Check)),
            LAST => ("Start exploring", None),
            _ => ("Continue", Some(IconName::ArrowRight)),
        };
        let next = Button::new("welcome-next")
            .debug_selector(|| "welcome-next".into())
            .primary()
            .label(next_label)
            .when_some(next_icon, |b, icon| b.icon(icon))
            .on_click(cx.listener(move |this, _, window, cx| {
                if connect {
                    this.welcome_connect(cx)
                } else {
                    this.welcome_go(1, window, cx)
                }
            }));
        let test = form.map(|form| {
            let (label, enabled) = form.read(cx).test_button(cx);
            Button::new("welcome-test")
                .debug_selector(|| "welcome-test".into())
                .ghost()
                .label(label)
                .disabled(!enabled)
                .on_click(move |_, _, cx| form.update(cx, |f, cx| f.run_test(cx)))
        });
        let back = Button::new("welcome-back")
            .ghost()
            .label("Back")
            .when(step == 0, |b| b.invisible())
            .on_click(cx.listener(|this, _, window, cx| this.welcome_go(-1, window, cx)));
        let skip = Button::new("welcome-skip")
            .debug_selector(|| "welcome-skip".into())
            .ghost()
            .small()
            .label(match step {
                LAST if connect => "I'll do this later",
                LAST => "Close",
                _ => "Skip tour",
            })
            .on_click(cx.listener(|this, _, window, cx| this.close_welcome(window, cx)));
        let glow = theme.primary.opacity(0.14);
        // The tour fills its window; in a big one (from the palette) the column stays readable.
        let column = v_flex()
            .size_full()
            .max_w(px(760.))
            .mx_auto()
            .gap(theme::space_16())
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(
                        h_flex()
                            .items_center()
                            .gap(theme::space_8())
                            .child(img(assets::APP_ICON).size(px(20.)))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .child("TableX"),
                            )
                            .child(
                                div()
                                    .h(px(16.))
                                    .w(px(1.))
                                    .mx(theme::space_4())
                                    .bg(theme.border),
                            )
                            .child(
                                div()
                                    .text_size(theme::ui_sm())
                                    .text_color(theme.muted_foreground)
                                    .child("DATABASE WORKSPACE"),
                            ),
                    )
                    .child(skip),
            )
            .child(
                v_flex()
                    .id("welcome-body")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    // The form is tall and starts at the top; the other steps sit centered.
                    .when(!connect, |d| d.justify_center())
                    .child(content),
            )
            .child(
                h_flex()
                    .w_full()
                    // Equal sides keep progress centered whatever the buttons' widths.
                    .child(h_flex().flex_1().child(back))
                    .child(progress)
                    .child(
                        h_flex()
                            .flex_1()
                            .justify_end()
                            .gap(theme::space_8())
                            .children(test)
                            .child(next),
                    ),
            );
        v_flex()
            .id("welcome")
            .debug_selector(|| "welcome".into())
            .track_focus(&welcome.focus)
            .key_context("Welcome")
            .absolute()
            .inset_0()
            .occlude()
            // Opaque: the app stays out of sight until the tour is done.
            .bg(theme.background)
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let m = &ev.keystroke.modifiers;
                if m.control || m.alt || m.platform || m.shift {
                    return;
                }
                match ev.keystroke.key.as_str() {
                    "escape" => this.close_welcome(window, cx),
                    "enter" if connect => this.welcome_connect(cx),
                    // The form's fields use the arrows themselves.
                    "left" | "right" if connect => return,
                    "enter" | "right" => this.welcome_go(1, window, cx),
                    "left" => this.welcome_go(-1, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            // The window still needs its controls and a place to drag it by.
            .child(super::titlebar::Titlebar::new(div(), div()))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(40.))
                    .pt(theme::space_8())
                    .pb(theme::space_24())
                    .bg(linear_gradient(
                        160.,
                        linear_color_stop(glow, 0.),
                        linear_color_stop(theme.background.opacity(0.), 0.5),
                    ))
                    .child(column),
            )
            .into_any_element()
    }

    fn intro(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let engine = |path: &'static str, kind, name: &'static str| {
            h_flex()
                .gap(theme::space_8())
                .px(theme::space_12())
                .py(theme::space_6())
                .rounded(theme::radius_md())
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .child(
                    Icon::empty()
                        .path(path)
                        .text_color(theme::brand_color(kind, theme)),
                )
                .child(name)
        };
        let card = page(
            "Hello",
            "A clearer view of your data.",
            "TableX brings your databases, tables and SQL into one focused workspace. Set up your space in a few quick steps.",
            h_flex()
                .gap(theme::space_8())
                .child(engine(
                    assets::POSTGRES_ICON,
                    crate::model::DbKind::Postgres,
                    "PostgreSQL",
                ))
                .child(engine(
                    assets::SQLITE_ICON,
                    crate::model::DbKind::Sqlite,
                    "SQLite",
                )),
            cx,
        );
        v_flex()
            .gap(theme::space_24())
            .child(card)
            .child(product_preview(cx))
            .into_any_element()
    }

    fn looks(&self, cx: &mut Context<Self>) -> AnyElement {
        let choices = theme::ThemeChoice::ALL.into_iter().chain(
            self.user_themes
                .iter()
                .map(|t| theme::ThemeChoice::User(t.name.clone())),
        );
        let cards = choices.enumerate().map(|(ix, choice)| {
            let selected = choice == self.theme;
            let preview = preview_theme(&choice, cx.theme(), &self.user_themes, cx);
            let icon = match choice {
                theme::ThemeChoice::Dark => Lucide::Moon,
                theme::ThemeChoice::Light => Lucide::Sun,
                theme::ThemeChoice::System => Lucide::Monitor,
                theme::ThemeChoice::Darcula => Lucide::Code,
                theme::ThemeChoice::User(_) => Lucide::Palette,
            };
            let theme = cx.theme();
            v_flex()
                .id(("welcome-theme", ix))
                .debug_selector(move || format!("welcome-theme-{ix}"))
                .w(px(148.))
                .gap(theme::space_8())
                .p(theme::space_8())
                .rounded(theme::radius_lg())
                .border_2()
                .border_color(if selected {
                    theme.primary
                } else {
                    theme.border
                })
                .bg(theme.secondary)
                .cursor_pointer()
                .hover(|d| d.bg(theme.secondary_hover))
                .child(theme_sample(&preview))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(theme::space_8())
                        .child(Icon::new(icon).text_color(if selected {
                            theme.primary
                        } else {
                            theme.muted_foreground
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(if selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .child(choice.label().to_string()),
                        )
                        .when(selected, |d| {
                            d.child(Icon::new(Lucide::Check).text_color(theme.primary))
                        }),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_theme(choice.clone(), window, cx)
                }))
        });
        let scales = theme::UI_SCALES.into_iter().map(|percent| {
            Button::new(("welcome-scale", percent as usize))
                .small()
                .label(format!("{percent}%"))
                .map(|b| {
                    if percent == theme::ui_scale() {
                        b.primary()
                    } else {
                        b.ghost()
                    }
                })
                .on_click(
                    cx.listener(move |this, _, window, cx| this.set_ui_scale(percent, window, cx)),
                )
        });
        let theme = cx.theme();
        page(
            "Step 1 of 3",
            "Make it yours",
            "Choose a theme and interface size. Changes apply immediately and are easy to revisit in Settings.",
            v_flex()
                .gap(theme::space_16())
                .child(h_flex().flex_wrap().gap(theme::space_12()).children(cards))
                .child(
                    h_flex()
                        .gap(theme::space_8())
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .mr(theme::space_8())
                                .child("Size"),
                        )
                        .children(scales),
                ),
            cx,
        )
    }
}

fn preview_theme(
    choice: &theme::ThemeChoice,
    current: &PreviewTheme,
    user_themes: &[std::rc::Rc<gpui_kit::component::theme::ThemeConfig>],
    cx: &App,
) -> PreviewTheme {
    let config = match choice {
        theme::ThemeChoice::Dark => ThemeRegistry::global(cx)
            .themes()
            .get("TableX Dark")
            .cloned(),
        theme::ThemeChoice::Light => ThemeRegistry::global(cx)
            .themes()
            .get("TableX Light")
            .cloned(),
        theme::ThemeChoice::Darcula => ThemeRegistry::global(cx).themes().get("Darcula").cloned(),
        theme::ThemeChoice::System => Some(if current.mode.is_dark() {
            current.dark_theme.clone()
        } else {
            current.light_theme.clone()
        }),
        theme::ThemeChoice::User(name) => user_themes
            .iter()
            .find(|candidate| candidate.name == *name)
            .cloned(),
    };
    let Some(config) = config else {
        return current.clone();
    };
    let mut preview = current.clone();
    preview.apply_config(&config);
    preview
}

fn theme_sample(preview: &PreviewTheme) -> AnyElement {
    let line = |width: Pixels, color: Hsla| div().h(px(4.)).w(width).rounded_full().bg(color);
    v_flex()
        .w_full()
        .h(px(64.))
        .overflow_hidden()
        .rounded(theme::radius_md())
        .border_1()
        .border_color(preview.border)
        .bg(preview.background)
        .child(
            h_flex()
                .size_full()
                .child(
                    v_flex()
                        .w(px(34.))
                        .h_full()
                        .gap(theme::space_6())
                        .p(theme::space_6())
                        .bg(preview.secondary)
                        .child(line(px(20.), preview.primary))
                        .child(line(px(16.), preview.foreground.opacity(0.42)))
                        .child(line(px(18.), preview.foreground.opacity(0.24)))
                        .child(line(px(14.), preview.foreground.opacity(0.24))),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .h_full()
                        .gap(theme::space_6())
                        .p(theme::space_8())
                        .child(
                            h_flex()
                                .items_center()
                                .gap(theme::space_6())
                                .child(line(px(28.), preview.foreground.opacity(0.72)))
                                .child(line(px(18.), preview.primary.opacity(0.72))),
                        )
                        .child(line(px(76.), preview.border))
                        .child(line(px(92.), preview.border))
                        .child(line(px(66.), preview.border)),
                ),
        )
        .into_any_element()
}

fn product_preview(cx: &App) -> AnyElement {
    let theme = cx.theme();
    let row = |name: &'static str, selected: bool| {
        h_flex()
            .gap(theme::space_8())
            .px(theme::space_8())
            .py(theme::space_6())
            .rounded(theme::radius_sm())
            .when(selected, |d| d.bg(theme.primary.opacity(0.14)))
            .child(
                Icon::new(Lucide::Table2)
                    .size(theme::space_12())
                    .text_color(theme.primary),
            )
            .child(div().text_size(theme::ui_sm()).child(name))
    };
    let heading = |label: &'static str| {
        div()
            .text_size(theme::ui_sm())
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .child(label)
    };
    let cell = |label: &'static str| {
        div()
            .flex_1()
            .px(theme::space_8())
            .py(theme::space_6())
            .text_size(theme::ui_sm())
            .border_b_1()
            .border_color(theme.border)
            .child(label)
    };
    h_flex()
        .w_full()
        .h(px(176.))
        .overflow_hidden()
        .rounded(theme::radius_lg())
        .border_1()
        .border_color(theme.border)
        .bg(theme.table)
        .child(
            v_flex()
                .w(px(168.))
                .h_full()
                .gap(theme::space_8())
                .p(theme::space_12())
                .border_r_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .gap(theme::space_8())
                        .items_center()
                        .child(img(assets::APP_ICON).size(px(20.)))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("TableX")),
                )
                .child(heading("CONNECTIONS"))
                .child(row("local · SQLite", true))
                .child(row("analytics", false))
                .child(heading("TABLES").mt(theme::space_8()))
                .child(row("customers", false))
                .child(row("orders", false)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .p(theme::space_16())
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .pb(theme::space_12())
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("customers"))
                        .child(
                            h_flex()
                                .gap(theme::space_8())
                                .text_size(theme::ui_sm())
                                .text_color(theme.muted_foreground)
                                .child(Icon::new(Lucide::Search).size(theme::space_12()))
                                .child("Filter rows"),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .child(cell("id"))
                        .child(cell("name"))
                        .child(cell("email")),
                )
                .child(
                    h_flex()
                        .w_full()
                        .child(cell("1042"))
                        .child(cell("Maya Chen"))
                        .child(cell("maya@…")),
                )
                .child(
                    h_flex()
                        .w_full()
                        .child(cell("1043"))
                        .child(cell("Noah Patel"))
                        .child(cell("noah@…")),
                )
                .child(
                    h_flex()
                        .w_full()
                        .child(cell("1044"))
                        .child(cell("Ava Brooks"))
                        .child(cell("ava@…")),
                ),
        )
        .into_any_element()
}

/// The last step: the Add connection form, or "all set" when connections already exist.
fn connect_step(form: Option<&Entity<ConnectionDialog>>, cx: &App) -> AnyElement {
    match form {
        Some(form) => page(
            "Step 3 of 3",
            "Connect your first database",
            "PostgreSQL by host, or a SQLite file. Passwords go to your system keychain.",
            v_flex()
                .w_full()
                .max_w(px(560.))
                .p(theme::space_12())
                .rounded(theme::radius_lg())
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().table)
                .child(form.clone()),
            cx,
        ),
        None => page(
            "Step 3 of 3",
            "You're all set",
            "Your connections are in the explorer on the left. Pick a table to open it, or start \
             a console to run SQL.",
            h_flex()
                .gap(theme::space_8())
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(Lucide::Database))
                .child("Workspaces keep each project's connections and tabs apart; switch them from the title bar."),
            cx,
        ),
    }
}

fn keys(cx: &App) -> AnyElement {
    let bound = cx
        .try_global::<Bindings>()
        .map(|b| b.0.as_slice())
        .unwrap_or_default();
    let theme = cx.theme();
    let rows = KEYS.iter().filter_map(|(action, title, description)| {
        // The first binding, each chord as its own key cap; unbound shortcuts are left out.
        let keys = &bound.iter().find(|b| b.action == *action)?.keys;
        let caps: Vec<_> = keys
            .split_whitespace()
            .filter_map(|k| Keystroke::parse(k).ok())
            .map(Kbd::new)
            .collect();
        Some(
            h_flex()
                .w_full()
                .py(theme::space_8())
                .border_b_1()
                .border_color(theme.border)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(theme::space_2())
                        .child(div().font_weight(FontWeight::MEDIUM).child(*title))
                        .child(
                            div()
                                .text_size(theme::ui_sm())
                                .text_color(theme.muted_foreground)
                                .child(*description),
                        ),
                )
                .child(h_flex().gap(theme::space_4()).children(caps)),
        )
    });
    page(
        "Step 2 of 3",
        "Move through TableX faster",
        "These follow your current keymap. Open the shortcut reference to browse the full list.",
        v_flex().w_full().max_w(px(520.)).children(rows),
        cx,
    )
}

/// One step: a small label, a big title, a line of text and the step's own part below.
fn page(
    eyebrow: &'static str,
    title: &'static str,
    body: &'static str,
    visual: impl IntoElement,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .gap(theme::space_12())
        .child(
            div()
                .text_size(theme::ui_sm())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.primary)
                .child(eyebrow),
        )
        .child(
            div()
                .text_size(theme::ui_xl() * 1.75)
                .line_height(relative(1.1))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .max_w(px(560.))
                .text_size(theme::ui_lg())
                .text_color(theme.muted_foreground)
                .child(body),
        )
        .child(div().mt(theme::space_16()).child(visual))
        .into_any_element()
}
