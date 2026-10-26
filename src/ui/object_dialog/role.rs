use std::collections::BTreeSet;
use std::sync::Arc;

use gpui_kit::StatefulInteractiveElement as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::prelude::FluentBuilder as _;

use super::*;
use crate::db::{Driver, Reply, Session};
use crate::ddl::create_role;

/// New Role, or New User when `login`: a name, a password for a user, and the roles it joins.
pub struct RoleDialog {
    pub name: Entity<InputState>,
    /// User only.
    pub password: Option<Entity<InputState>>,
    /// Every role's name, once loaded.
    pub roles: Vec<String>,
    member_of: BTreeSet<String>,
    pub confirm: ProductionConfirm,
    /// Waiting for the explorer to run the last Apply.
    applying: bool,
    pub error: Option<String>,
}

impl EventEmitter<Apply> for RoleDialog {}

impl RoleDialog {
    /// Opens New Role (or New User when `login`) over `window`, listing the roles through the
    /// session `open` resolves to.
    pub fn open(
        driver: Arc<dyn Driver>,
        open: Reply<Session>,
        login: bool,
        is_production: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(driver, open, login, is_production, window, cx));
        let body = view.clone();
        let title = if login { "New user" } else { "New role" };
        window.open_dialog(cx, move |dialog, _, cx| {
            let enabled = body.read(cx).apply_enabled(cx);
            let on_apply = body.clone();
            dialog
                .title(title)
                .child(body.clone())
                .footer(footer(enabled, move |_, cx| {
                    on_apply.update(cx, |d, cx| d.apply(cx))
                }))
        });
        // The footer reads Apply's state when the dialog layer renders, which a body notify skips.
        window
            .observe(&view, cx, |_, window, _| window.refresh())
            .detach();
        view.read(cx)
            .name
            .clone()
            .update(cx, |name, cx| name.focus(window, cx));
        view
    }

    fn new(
        driver: Arc<dyn Driver>,
        open: Reply<Session>,
        login: bool,
        is_production: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx));
        // On production the confirmation is the new role's name.
        cx.subscribe(&name, |this, name, _: &InputEvent, cx| {
            this.confirm.name = name.read(cx).value().to_string();
            cx.notify()
        })
        .detach();
        let password = login.then(|| {
            let password = cx.new(|cx| InputState::new(window, cx).masked(true));
            cx.subscribe(&password, |_, _, _: &InputEvent, cx| cx.notify())
                .detach();
            password
        });
        cx.spawn_in(window, async move |this, cx| {
            let roles = async { driver.roles(&open.await?).await }.await;
            this.update(cx, |this, cx| {
                match roles {
                    Ok(roles) => this.roles = roles.into_iter().map(|r| r.name).collect(),
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            name,
            password,
            roles: Vec::new(),
            member_of: BTreeSet::new(),
            confirm: ProductionConfirm::new("", is_production, window, cx),
            applying: false,
            error: None,
        }
    }

    pub fn statements(&self, cx: &App) -> Vec<String> {
        let name = self.name.read(cx).value();
        if name.is_empty() {
            return Vec::new();
        }
        let password = self.password.as_ref().map(|p| p.read(cx).value());
        let member_of: Vec<String> = self.member_of.iter().cloned().collect();
        create_role(&name, password.as_deref(), &member_of)
    }

    fn apply_enabled(&self, cx: &App) -> bool {
        !self.applying && !self.statements(cx).is_empty() && self.confirm.confirmed(cx)
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if !self.apply_enabled(cx) {
            return;
        }
        self.applying = true;
        self.error = None;
        cx.emit(Apply(self.statements(cx)));
        cx.notify();
    }

    /// The Apply failed with `reason`: shown, and Apply is enabled again.
    pub fn failed(&mut self, reason: String, cx: &mut Context<Self>) {
        self.applying = false;
        self.error = Some(reason);
        cx.notify();
    }
}

impl Render for RoleDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.roles.is_empty() && self.error.is_none();
        // Fixed height, so the dialog neither grows when the roles arrive nor runs off a long list.
        let members = v_flex()
            .id("member-of-list")
            .h(theme::list_row() * 5.)
            .overflow_y_scroll()
            .gap(theme::space_4())
            .when(loading, |d| {
                d.text_size(theme::ui_sm())
                    .text_color(theme::text_secondary(cx.theme()))
                    .child("Loading roles…")
            })
            .children(self.roles.iter().map(|role| {
                let selector = format!("member-{role}");
                let checked = self.member_of.contains(role);
                let role = role.clone();
                div().debug_selector(move || selector.clone()).child(
                    Checkbox::new(SharedString::from(format!("member-{role}")))
                        .label(role.clone())
                        .checked(checked)
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            if *checked {
                                this.member_of.insert(role.clone());
                            } else {
                                this.member_of.remove(&role);
                            }
                            cx.notify();
                        })),
                )
            }));
        v_flex()
            .gap(theme::space_12())
            .child(field(
                "name",
                "Name".into(),
                Input::new(&self.name).small(),
                cx,
            ))
            .children(self.password.as_ref().map(|password| {
                field(
                    "password",
                    "Password".into(),
                    Input::new(password).small(),
                    cx,
                )
            }))
            .child(field("member-of", "Member of".into(), members, cx))
            .child(sql_preview(&self.statements(cx), cx))
            .children(self.confirm.render(cx))
            .children(error_line(self.error.as_deref(), cx))
    }
}
