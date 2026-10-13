use std::sync::Arc;

use super::*;
use crate::db::{Driver, Reply, Session};
use crate::ddl::{alter_database, alter_schema};

/// What a `NameOwnerDialog` modifies.
#[derive(Clone, Copy, PartialEq)]
pub enum Owned {
    Database,
    Schema,
}

/// Modify database or schema: its name and owner.
pub struct NameOwnerDialog {
    kind: Owned,
    original: String,
    pub name: Entity<InputState>,
    /// Every role's name, once loaded.
    pub roles: Vec<String>,
    /// The owner when the dialog opened, once loaded.
    owner_was: String,
    /// `None` until both the roles and the owner arrive.
    pub owner: Option<Entity<SelectState<Vec<String>>>>,
    pub confirm: ProductionConfirm,
    /// Waiting for the explorer to run the last Apply.
    applying: bool,
    pub error: Option<String>,
}

impl EventEmitter<Apply> for NameOwnerDialog {}

impl NameOwnerDialog {
    /// Opens Modify database or schema `name` over `window`, reading its roles and owner through
    /// the session `open` resolves to (a session on the database).
    pub fn open(
        driver: Arc<dyn Driver>,
        open: Reply<Session>,
        kind: Owned,
        name: &str,
        is_production: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(driver, open, kind, name, is_production, window, cx));
        let body = view.clone();
        let noun = match kind {
            Owned::Database => "database",
            Owned::Schema => "schema",
        };
        let title = format!("Modify {noun} {name}");
        window.open_dialog(cx, move |dialog, _, cx| {
            let enabled = body.read(cx).apply_enabled(cx);
            let on_apply = body.clone();
            dialog
                .title(title.clone())
                .child(body.clone())
                .footer(footer(enabled, move |_, cx| {
                    on_apply.update(cx, |d, cx| d.apply(cx))
                }))
        });
        view.read(cx)
            .name
            .clone()
            .update(cx, |name, cx| name.focus(window, cx));
        view
    }

    fn new(
        driver: Arc<dyn Driver>,
        open: Reply<Session>,
        kind: Owned,
        name: &str,
        is_production: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.to_string()));
        cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        let schema = name.to_string();
        cx.spawn_in(window, async move |this, cx| {
            // The session is dropped once read, so it doesn't block a rename.
            let loaded = async {
                let session = open.await?;
                let owner = match kind {
                    Owned::Database => driver.database_owner(&session),
                    Owned::Schema => driver.schema_owner(&session, &schema),
                };
                let (roles, owner) = futures::join!(driver.roles(&session), owner);
                Ok::<_, String>((roles?, owner?))
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                match loaded {
                    Ok((roles, owner)) => this.loaded(roles, owner, window, cx),
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            kind,
            original: name.into(),
            name: input,
            roles: Vec::new(),
            owner_was: String::new(),
            owner: None,
            confirm: ProductionConfirm::new(name, is_production, window, cx),
            applying: false,
            error: None,
        }
    }

    fn loaded(
        &mut self,
        roles: Vec<crate::model::Role>,
        owner: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.roles = roles.into_iter().map(|r| r.name).collect();
        let selected = self.roles.iter().position(|r| *r == owner);
        let select = cx.new(|cx| {
            SelectState::new(self.roles.clone(), selected.map(IndexPath::new), window, cx)
        });
        cx.subscribe(&select, |_, _, _: &SelectEvent<Vec<String>>, cx| {
            cx.notify()
        })
        .detach();
        self.owner = Some(select);
        self.owner_was = owner;
    }

    /// The statements the form's changes need; none until the owner is loaded.
    pub fn statements(&self, cx: &App) -> Vec<String> {
        let Some(owner) = &self.owner else {
            return Vec::new();
        };
        let new_owner = owner
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| self.owner_was.clone());
        let build = match self.kind {
            Owned::Database => alter_database,
            Owned::Schema => alter_schema,
        };
        build(
            &self.original,
            &self.name.read(cx).value(),
            &self.owner_was,
            &new_owner,
        )
    }

    fn apply_enabled(&self, cx: &App) -> bool {
        !self.applying
            && !self.name.read(cx).value().is_empty()
            && !self.statements(cx).is_empty()
            && self.confirm.confirmed(cx)
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

impl Render for NameOwnerDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = match &self.owner {
            Some(owner) => Select::new(owner).small().into_any_element(),
            None => div()
                .debug_selector(|| "owner-loading".into())
                .text_size(theme::ui_sm())
                .text_color(theme::text_secondary(cx.theme()))
                .child("Loading roles…")
                .into_any_element(),
        };
        v_flex()
            .gap(theme::space_12())
            .child(field(
                "name",
                "Name".into(),
                Input::new(&self.name).small(),
                cx,
            ))
            .child(field("owner", "Owner".into(), owner, cx))
            .child(sql_preview(&self.statements(cx), cx))
            .children(self.confirm.render(cx))
            .children(error_line(self.error.as_deref(), cx))
    }
}
