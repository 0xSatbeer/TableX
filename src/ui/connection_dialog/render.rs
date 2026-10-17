use super::*;

/// A labeled control with its entry in `errors` under it. `id` names it for tests (`field-<id>`).
fn field(
    id: &'static str,
    label: &'static str,
    control: impl IntoElement,
    errors: &[(&str, String)],
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let error = errors
        .iter()
        .find(|(field, _)| *field == id)
        .map(|(_, msg)| msg.clone());
    v_flex()
        .id(id)
        .debug_selector(move || format!("field-{id}"))
        .flex_1()
        .gap(theme::space_4())
        .child(
            div()
                .text_size(theme::ui_sm())
                .text_color(theme::text_secondary(cx.theme()))
                .child(label),
        )
        .child(control)
        .when_some(error, |d, msg| {
            d.child(
                div()
                    .debug_selector(move || format!("error-{id}"))
                    .text_size(theme::ui_sm())
                    .text_color(cx.theme().danger)
                    .child(msg),
            )
        })
}

impl Render for ConnectionDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let errors = self.errors(cx);
        let theme = cx.theme();
        let swatches = (0..theme::CONNECTION_COLORS.len() as u8).map(|ix| {
            let picked = ix == self.color;
            div()
                .id(("swatch", ix as usize))
                .debug_selector(move || format!("swatch-{ix}"))
                // 20px at a 28px pitch clears the WCAG 2.5.8 24px target spacing.
                .size(theme::space_16() + theme::space_4())
                .rounded_full()
                .bg(theme::connection_color(ix, theme))
                .when(picked, |d| d.border_2().border_color(theme.foreground))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.color = ix;
                    cx.notify();
                }))
                .into_any_element()
        });
        let locked = self.editing.is_some();
        let kinds = ButtonGroup::new("kind")
            .small()
            .outline()
            // Edit mode locks the type by disabling only the other one, since a disabled button hides
            // its selected style; `children` because `child` overwrites each button's disabled state.
            .children([
                Button::new("kind-postgres")
                    .debug_selector(|| "kind-postgres".into())
                    .label("Postgres")
                    .selected(self.kind == DbKind::Postgres)
                    .disabled(locked && self.kind != DbKind::Postgres),
                Button::new("kind-sqlite")
                    .debug_selector(|| "kind-sqlite".into())
                    .label("SQLite")
                    .selected(self.kind == DbKind::Sqlite)
                    .disabled(locked && self.kind != DbKind::Sqlite),
            ])
            .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
                let kind = if clicked.first() == Some(&1) {
                    DbKind::Sqlite
                } else {
                    DbKind::Postgres
                };
                if kind != this.kind {
                    this.kind = kind;
                    this.connection_changed();
                }
                cx.notify();
            }));
        let fields: Vec<AnyElement> = match self.kind {
            DbKind::Postgres => vec![
                h_flex()
                    .items_start()
                    .gap(theme::space_8())
                    .child(field(
                        "host",
                        "Host",
                        Input::new(&self.host).small(),
                        &errors,
                        cx,
                    ))
                    .child(
                        // A port is five digits; the host gets the rest of the row.
                        field("port", "Port", Input::new(&self.port).small(), &errors, cx)
                            .flex_none()
                            .w(theme::space_24() * 4.),
                    )
                    .into_any_element(),
                h_flex()
                    .items_start()
                    .gap(theme::space_8())
                    .child(field(
                        "user",
                        "User",
                        Input::new(&self.user).small(),
                        &errors,
                        cx,
                    ))
                    .child(field(
                        "database",
                        "Database",
                        Input::new(&self.database).small(),
                        &errors,
                        cx,
                    ))
                    .into_any_element(),
                field(
                    "password",
                    "Password",
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1().child(Input::new(&self.password).small()))
                        .child(
                            Checkbox::new("save-password")
                                .debug_selector(|| "save-password".into())
                                .label("Save password")
                                .checked(self.save_password)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.save_password = *checked;
                                    cx.notify();
                                })),
                        ),
                    &errors,
                    cx,
                )
                .into_any_element(),
                field("ssl", "SSL", Select::new(&self.ssl).small(), &errors, cx).into_any_element(),
            ],
            DbKind::Sqlite => vec![
                field(
                    "path",
                    "File path",
                    Input::new(&self.path).small(),
                    &errors,
                    cx,
                )
                .into_any_element(),
            ],
        };
        v_flex()
            .gap(theme::space_12())
            .child(field("kind", "Type", kinds, &errors, cx))
            .child(field(
                "name",
                "Name",
                Input::new(&self.name).small(),
                &errors,
                cx,
            ))
            .child(field(
                "color",
                "Color",
                h_flex().gap(theme::space_8()).children(swatches),
                &errors,
                cx,
            ))
            .child(
                div()
                    .self_start()
                    .debug_selector(|| "production".into())
                    .child(
                        Switch::new("production")
                            .small()
                            .label("Production")
                            .checked(self.is_production)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.is_production = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .children(fields)
            .when_some(self.test_line(cx), |d, (line, color)| {
                d.child(
                    div()
                        .debug_selector(|| "test-result".into())
                        .text_size(theme::ui_sm())
                        .text_color(color)
                        .child(line),
                )
            })
    }
}
