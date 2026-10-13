//! Table → Modify: the table's name and columns, with the `alter_table` SQL they need. The
//! edits live in a `TableForm`; the view keeps it in step with its inputs.

mod form;

pub use form::*;

use std::sync::Arc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
};

use crate::db::{Driver, Reply, Session};
use crate::model::TableDef;
use crate::theme;
use crate::ui::object_dialog::{Apply, ProductionConfirm, error_line, field, footer, sql_preview};

/// One column row's inputs; its nullable flag lives in the form.
pub struct ColumnInputs {
    pub name: Entity<InputState>,
    pub ty: Entity<InputState>,
    pub default: Entity<InputState>,
}

pub struct TableEditor {
    schema: String,
    pub name: Entity<InputState>,
    /// `None` until the definition arrives.
    pub form: Option<TableForm>,
    pub rows: Vec<ColumnInputs>,
    pub confirm: ProductionConfirm,
    /// Waiting for the explorer to run the last Apply.
    applying: bool,
    pub error: Option<String>,
}

impl EventEmitter<Apply> for TableEditor {}

impl TableEditor {
    /// Opens Modify table `schema.table` over `window`, reading its definition through the
    /// session `open` resolves to.
    pub fn open(
        driver: Arc<dyn Driver>,
        open: Reply<Session>,
        (schema, table): (&str, &str),
        is_production: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let view = cx.new(|cx| Self::new(driver, open, (schema, table), is_production, window, cx));
        let body = view.clone();
        let title = format!("Modify table {table}");
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
        (schema, table): (&str, &str),
        is_production: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = Self::input(table, "", window, cx);
        let (on_schema, on_table) = (schema.to_string(), table.to_string());
        cx.spawn_in(window, async move |this, cx| {
            let def = async {
                let session = open.await?;
                driver.table_def(&session, &on_schema, &on_table).await
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                match def {
                    Ok(def) => this.loaded(def, window, cx),
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            schema: schema.into(),
            name,
            form: None,
            rows: Vec::new(),
            confirm: ProductionConfirm::new(table, is_production, window, cx),
            applying: false,
            error: None,
        }
    }

    /// An input holding `value` whose edits flow into the form.
    fn input(
        value: &str,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value.to_string())
        });
        cx.subscribe(&input, |this, _, _: &InputEvent, cx| this.sync(cx))
            .detach();
        input
    }

    fn loaded(&mut self, def: TableDef, window: &mut Window, cx: &mut Context<Self>) {
        let form = TableForm::modify(def);
        self.rows = form
            .columns
            .iter()
            .map(|(_, c)| Self::row(c, window, cx))
            .collect();
        self.form = Some(form);
        // A name typed while the definition loaded wins over the one it brought.
        self.sync(cx);
    }

    fn row(c: &crate::model::Column, window: &mut Window, cx: &mut Context<Self>) -> ColumnInputs {
        ColumnInputs {
            name: Self::input(&c.name, "name", window, cx),
            ty: Self::input(&c.data_type, "type", window, cx),
            default: Self::input(c.default.as_deref().unwrap_or(""), "default", window, cx),
        }
    }

    /// Copies the inputs' text into the form.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.name = self.name.read(cx).value().to_string();
        for (row, (_, c)) in self.rows.iter().zip(&mut form.columns) {
            c.name = row.name.read(cx).value().to_string();
            c.data_type = row.ty.read(cx).value().to_string();
            let default = row.default.read(cx).value().to_string();
            c.default = (!default.is_empty()).then_some(default);
        }
        cx.notify();
    }

    fn add_column(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.add_column();
        let row = Self::row(&form.columns[form.columns.len() - 1].1, window, cx);
        self.rows.push(row);
        cx.notify();
    }

    fn remove_column(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.columns.remove(ix);
        self.rows.remove(ix);
        cx.notify();
    }

    fn set_nullable(&mut self, ix: usize, nullable: bool, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.columns[ix].1.nullable = nullable;
        cx.notify();
    }

    pub fn statements(&self) -> Vec<String> {
        self.form
            .as_ref()
            .map_or_else(Vec::new, |f| f.statements(&self.schema))
    }

    fn apply_enabled(&self, cx: &App) -> bool {
        !self.applying && !self.statements().is_empty() && self.confirm.confirmed(cx)
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if !self.apply_enabled(cx) {
            return;
        }
        self.applying = true;
        self.error = None;
        cx.emit(Apply(self.statements()));
        cx.notify();
    }

    /// The Apply failed with `reason`: shown, and Apply is enabled again.
    pub fn failed(&mut self, reason: String, cx: &mut Context<Self>) {
        self.applying = false;
        self.error = Some(reason);
        cx.notify();
    }

    fn render_columns(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = &self.form else {
            return div()
                .debug_selector(|| "columns-loading".into())
                .text_size(theme::ui_sm())
                .text_color(theme::text_secondary(cx.theme()))
                .child("Loading columns…")
                .into_any_element();
        };
        let rows = form
            .columns
            .iter()
            .zip(&self.rows)
            .enumerate()
            .map(|(ix, ((_, c), inputs))| self.render_row(ix, c, inputs, cx))
            .collect::<Vec<_>>();
        v_flex()
            .gap(theme::space_4())
            .children(rows)
            .child(
                div().debug_selector(|| "column-add".into()).child(
                    Button::new("column-add")
                        .ghost()
                        .small()
                        .label("Add column")
                        .on_click(cx.listener(|this, _, window, cx| this.add_column(window, cx))),
                ),
            )
            .into_any_element()
    }

    fn render_row(
        &self,
        ix: usize,
        c: &crate::model::Column,
        inputs: &ColumnInputs,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = div().flex_none().w(theme::space_12());
        let key = if c.is_pk {
            key.debug_selector(move || format!("column-pk-{ix}")).child(
                Icon::new(Lucide::KeyRound)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            )
        } else {
            key
        };
        h_flex()
            .debug_selector(move || format!("column-{ix}"))
            .gap(theme::space_6())
            .child(key)
            .child(div().w(px(160.)).child(Input::new(&inputs.name).small()))
            .child(div().w(px(140.)).child(Input::new(&inputs.ty).small()))
            .child(
                Checkbox::new(SharedString::from(format!("column-null-{ix}")))
                    .label("Null")
                    .checked(c.nullable)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        this.set_nullable(ix, *checked, cx)
                    })),
            )
            .child(div().w(px(140.)).child(Input::new(&inputs.default).small()))
            .child(
                div()
                    .debug_selector(move || format!("column-remove-{ix}"))
                    .child(
                        Button::new(("column-remove", ix))
                            .ghost()
                            .small()
                            .label("Remove")
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_column(ix, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for TableEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let columns = self.render_columns(cx);
        v_flex()
            .gap(theme::space_12())
            .child(field(
                "name",
                "Name".into(),
                Input::new(&self.name).small(),
                cx,
            ))
            .child(field("columns", "Columns".into(), columns, cx))
            .child(sql_preview(&self.statements(), cx))
            .children(self.confirm.render(cx))
            .children(error_line(self.error.as_deref(), cx))
    }
}
