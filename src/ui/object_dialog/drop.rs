use super::*;

/// Drop…: the DROP statement shown, and on production the object's name typed to enable Apply.
pub struct DropDialog {
    statement: String,
    pub confirm: ProductionConfirm,
    /// Waiting for the explorer to run the last Apply.
    applying: bool,
    pub error: Option<String>,
}

impl EventEmitter<Apply> for DropDialog {}

impl DropDialog {
    /// Opens "Drop <name>?" over `window`, offering to run `statement`.
    pub fn open(
        name: &str,
        statement: String,
        is_production: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let view = cx.new(|cx| Self {
            statement,
            confirm: ProductionConfirm::new(name, is_production, window, cx),
            applying: false,
            error: None,
        });
        let body = view.clone();
        let title = format!("Drop {name}?");
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
        // The footer reads Apply's state when the dialog layer renders, which a body notify skips.
        window
            .observe(&view, cx, |_, window, _| window.refresh())
            .detach();
        if is_production {
            let typed = view.read(cx).confirm.typed.clone();
            typed.update(cx, |typed, cx| typed.focus(window, cx));
        }
        view
    }

    fn apply_enabled(&self, cx: &App) -> bool {
        !self.applying && self.confirm.confirmed(cx)
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if !self.apply_enabled(cx) {
            return;
        }
        self.applying = true;
        self.error = None;
        cx.emit(Apply(vec![self.statement.clone()]));
        cx.notify();
    }

    /// The Apply failed with `reason`: shown, and Apply is enabled again.
    pub fn failed(&mut self, reason: String, cx: &mut Context<Self>) {
        self.applying = false;
        self.error = Some(reason);
        cx.notify();
    }
}

impl Render for DropDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(theme::space_12())
            .child(sql_preview(std::slice::from_ref(&self.statement), cx))
            .children(self.confirm.render(cx))
            .children(error_line(self.error.as_deref(), cx))
    }
}
