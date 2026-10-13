use super::*;

impl ConnectionDialog {
    /// The fields as the dialog shows them.
    pub fn form(&self, cx: &App) -> Form {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let ssl = self
            .ssl
            .read(cx)
            .selected_value()
            .and_then(|v| SSL_MODES.iter().find(|(_, label)| label == v))
            .map_or(SslMode::Prefer, |(mode, _)| *mode);
        Form {
            name: text(&self.name),
            color: self.color,
            is_production: self.is_production,
            kind: self.kind,
            host: text(&self.host),
            port: text(&self.port),
            user: text(&self.user),
            password: text(&self.password),
            database: text(&self.database),
            ssl,
            path: text(&self.path),
        }
    }

    /// Adds the connection to the workspace, or applies the edit, if the form is valid. Returns
    /// whether it did; if not, every field shows its error.
    pub fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let (Ok(name), Ok(params)) = self.validate(cx) else {
            self.touched.extend(TEXT_FIELDS);
            cx.notify();
            return false;
        };
        self.test = None;
        let (color, is_production, editing) = (self.color, self.is_production, self.editing);
        let id = self.workspace.update(cx, |ws, cx| {
            cx.notify();
            match editing {
                Some(id) => {
                    ws.edit(id, name, color, is_production, params);
                    id
                }
                None => ws.add(name, color, is_production, params),
            }
        });
        cx.emit(Saved(id));
        true
    }

    /// The trimmed name (unique among the other connections) and the trimmed params.
    fn validate(
        &self,
        cx: &App,
    ) -> (
        Result<String, String>,
        Result<ConnectionParams, FieldErrors>,
    ) {
        let form = self.form(cx);
        let others: Vec<&str> = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .filter(|c| Some(c.id) != self.editing)
            .map(|c| c.name.as_str())
            .collect();
        (validate_rename(&form.name, &others), form.params())
    }

    /// The inline errors on screen, as (field, message).
    pub fn errors(&self, cx: &App) -> Vec<(&'static str, String)> {
        let (name, params) = self.validate(cx);
        let fields = params.err().unwrap_or_default();
        [
            ("name", name.err()),
            ("host", fields.host),
            ("port", fields.port),
            ("user", fields.user),
            ("database", fields.database),
            ("path", fields.path),
        ]
        .into_iter()
        .filter(|(id, _)| self.touched.contains(id))
        .filter_map(|(id, error)| Some((id, error?)))
        .collect()
    }

    /// Whether the Add connection button is enabled.
    pub fn save_enabled(&self, cx: &App) -> bool {
        let (name, params) = self.validate(cx);
        button_state(name.is_ok(), params.is_ok()).0
    }

    /// The Test connection button: (label, enabled).
    pub fn test_button(&self, cx: &App) -> (&'static str, bool) {
        if self.test.is_some() {
            return ("Testing…", false);
        }
        let (name, params) = self.validate(cx);
        (
            "Test connection",
            button_state(name.is_ok(), params.is_ok()).1,
        )
    }

    /// The test result line above the footer: (text, color).
    pub fn test_line(&self, cx: &App) -> Option<(String, Hsla)> {
        Some(match self.test_result.as_ref()? {
            Ok(()) => ("Connection OK".into(), cx.theme().success),
            Err(reason) => (format!("Could not connect: {reason}"), cx.theme().danger),
        })
    }

    /// A connection field changed, so the last test, or the one running, no longer applies.
    pub(super) fn connection_changed(&mut self) {
        self.test = None;
        self.test_result = None;
    }

    /// Starts Test connection on the current fields, through the driver.
    pub(super) fn run_test(&mut self, cx: &mut Context<Self>) {
        let Ok(params) = self.form(cx).params() else {
            return;
        };
        self.test_result = None;
        let reply = self.driver.test(&params);
        self.test = Some(cx.spawn(async move |this, cx| {
            let outcome = reply.await;
            this.update(cx, |this, cx| {
                this.test = None;
                this.test_result = Some(outcome);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}
