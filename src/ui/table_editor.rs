//! Table → Modify: the table's name, columns, indexes and foreign keys, with the `alter_table` SQL they need. The
//! edits live in a `TableForm`; the view keeps it in step with its inputs.

mod form;

pub use form::*;

use std::sync::Arc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
};

use crate::db::{Driver, Reply, Session};
use crate::model::{ForeignKey, Index};
use crate::theme;
use crate::ui::object_dialog::{Apply, ProductionConfirm, error_line, field, footer, sql_preview};

/// One column row's inputs; its nullable flag lives in the form.
pub struct ColumnInputs {
    pub name: Entity<InputState>,
    pub ty: Entity<InputState>,
    pub default: Entity<InputState>,
}

/// One index row's inputs; its unique flag lives in the form.
pub struct IndexInputs {
    pub name: Entity<InputState>,
    /// Comma-separated column names.
    pub columns: Entity<InputState>,
}

type ActionSelect = Entity<SelectState<Vec<&'static str>>>;

/// One foreign key row's inputs.
pub struct FkInputs {
    pub name: Entity<InputState>,
    pub columns: Entity<InputState>,
    pub ref_schema: Entity<InputState>,
    pub ref_table: Entity<InputState>,
    pub ref_columns: Entity<InputState>,
    pub on_delete: ActionSelect,
    pub on_update: ActionSelect,
}

pub struct TableEditor {
    schema: String,
    pub name: Entity<InputState>,
    /// `None` until the definition arrives.
    pub form: Option<TableForm>,
    pub rows: Vec<ColumnInputs>,
    pub indexes: Vec<IndexInputs>,
    pub foreign_keys: Vec<FkInputs>,
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
        Self::show(view, format!("Modify table {table}"), window, cx)
    }

    /// Opens New table in `schema` over `window`: empty, with nothing to load.
    pub fn create(
        schema: &str,
        is_production: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let view = cx.new(|cx| {
            let mut this = Self::blank(schema, "", is_production, window, cx);
            this.loaded(TableForm::create(), window, cx);
            this
        });
        Self::show(view, "New table".into(), window, cx)
    }

    fn show(view: Entity<Self>, title: String, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let body = view.clone();
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
        let (on_schema, on_table) = (schema.to_string(), table.to_string());
        cx.spawn_in(window, async move |this, cx| {
            let def = async {
                let session = open.await?;
                driver.table_def(&session, &on_schema, &on_table).await
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                match def {
                    Ok(def) => this.loaded(TableForm::modify(def), window, cx),
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self::blank(schema, table, is_production, window, cx)
    }

    fn blank(
        schema: &str,
        table: &str,
        is_production: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            schema: schema.into(),
            name: Self::input(table, "", window, cx),
            form: None,
            rows: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
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

    fn loaded(&mut self, form: TableForm, window: &mut Window, cx: &mut Context<Self>) {
        self.rows = form
            .columns
            .iter()
            .map(|(_, c)| Self::row(c, window, cx))
            .collect();
        self.indexes = form
            .indexes
            .iter()
            .map(|i| Self::index_row(i, window, cx))
            .collect();
        self.foreign_keys = form
            .foreign_keys
            .iter()
            .map(|f| Self::fk_row(f, window, cx))
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

    fn index_row(i: &Index, window: &mut Window, cx: &mut Context<Self>) -> IndexInputs {
        IndexInputs {
            name: Self::input(&i.name, "name", window, cx),
            columns: Self::input(&i.columns.join(", "), "columns", window, cx),
        }
    }

    fn fk_row(f: &ForeignKey, window: &mut Window, cx: &mut Context<Self>) -> FkInputs {
        FkInputs {
            name: Self::input(&f.name, "name", window, cx),
            columns: Self::input(&f.columns.join(", "), "columns", window, cx),
            ref_schema: Self::input(&f.ref_schema, "ref schema", window, cx),
            ref_table: Self::input(&f.ref_table, "ref table", window, cx),
            ref_columns: Self::input(&f.ref_columns.join(", "), "ref columns", window, cx),
            on_delete: Self::action(&f.on_delete, window, cx),
            on_update: Self::action(&f.on_update, window, cx),
        }
    }

    /// A select over `FK_ACTIONS` on `value` whose picks flow into the form.
    fn action(value: &str, window: &mut Window, cx: &mut Context<Self>) -> ActionSelect {
        let selected = FK_ACTIONS.iter().position(|a| *a == value);
        let select = cx.new(|cx| {
            SelectState::new(
                FK_ACTIONS.to_vec(),
                selected.map(IndexPath::new),
                window,
                cx,
            )
        });
        cx.subscribe(
            &select,
            |this, _, _: &SelectEvent<Vec<&'static str>>, cx| this.sync(cx),
        )
        .detach();
        select
    }

    /// Copies the inputs' text into the form.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.name = self.name.read(cx).value().to_string();
        // On production a new table's confirmation is its new name.
        if form.is_create() {
            self.confirm.name = form.name.clone();
        }
        for (row, (_, c)) in self.rows.iter().zip(&mut form.columns) {
            c.name = row.name.read(cx).value().to_string();
            c.data_type = row.ty.read(cx).value().to_string();
            let default = row.default.read(cx).value().to_string();
            c.default = (!default.is_empty()).then_some(default);
        }
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        for (row, i) in self.indexes.iter().zip(&mut form.indexes) {
            i.name = text(&row.name);
            i.columns = names(&text(&row.columns));
        }
        for (row, f) in self.foreign_keys.iter().zip(&mut form.foreign_keys) {
            f.name = text(&row.name);
            f.columns = names(&text(&row.columns));
            f.ref_schema = text(&row.ref_schema);
            f.ref_table = text(&row.ref_table);
            f.ref_columns = names(&text(&row.ref_columns));
            let action = |s: &ActionSelect| s.read(cx).selected_value().map(|a| a.to_string());
            f.on_delete = action(&row.on_delete).unwrap_or_else(|| f.on_delete.clone());
            f.on_update = action(&row.on_update).unwrap_or_else(|| f.on_update.clone());
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

    fn add_index(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.add_index();
        let row = Self::index_row(&form.indexes[form.indexes.len() - 1], window, cx);
        self.indexes.push(row);
        cx.notify();
    }

    fn remove_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.indexes.remove(ix);
        self.indexes.remove(ix);
        cx.notify();
    }

    fn set_unique(&mut self, ix: usize, unique: bool, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.indexes[ix].unique = unique;
        cx.notify();
    }

    fn add_foreign_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.add_foreign_key(&self.schema);
        let fks = &form.foreign_keys;
        let row = Self::fk_row(&fks[fks.len() - 1], window, cx);
        self.foreign_keys.push(row);
        cx.notify();
    }

    fn remove_foreign_key(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.foreign_keys.remove(ix);
        self.foreign_keys.remove(ix);
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

    fn render_indexes(&self, indexes: &[Index], cx: &mut Context<Self>) -> AnyElement {
        let rows = indexes
            .iter()
            .zip(&self.indexes)
            .enumerate()
            .map(|(ix, (i, inputs))| {
                h_flex()
                    .debug_selector(move || format!("index-{ix}"))
                    .gap(theme::space_6())
                    .child(div().w(px(160.)).child(Input::new(&inputs.name).small()))
                    .child(div().w(px(200.)).child(Input::new(&inputs.columns).small()))
                    .child(
                        div()
                            .debug_selector(move || format!("index-unique-{ix}"))
                            .child(
                                Checkbox::new(SharedString::from(format!("index-unique-{ix}")))
                                    .label("Unique")
                                    .checked(i.unique)
                                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                        this.set_unique(ix, *checked, cx)
                                    })),
                            ),
                    )
                    .child(remove_button(
                        "index-remove",
                        ix,
                        cx.listener(move |this, _, _, cx| this.remove_index(ix, cx)),
                    ))
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap(theme::space_4())
            .children(rows)
            .child(add_button(
                "index-add",
                "Add index",
                cx.listener(|this, _, window, cx| this.add_index(window, cx)),
            ))
            .into_any_element()
    }

    fn render_foreign_keys(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self
            .foreign_keys
            .iter()
            .enumerate()
            .map(|(ix, inputs)| {
                let input =
                    |i: &Entity<InputState>, w: f32| div().w(px(w)).child(Input::new(i).small());
                v_flex()
                    .debug_selector(move || format!("fk-{ix}"))
                    .gap(theme::space_4())
                    .child(
                        h_flex()
                            .gap(theme::space_6())
                            .child(input(&inputs.name, 160.))
                            .child(input(&inputs.columns, 140.))
                            .child("→")
                            .child(input(&inputs.ref_schema, 100.))
                            .child(input(&inputs.ref_table, 120.))
                            .child(input(&inputs.ref_columns, 140.)),
                    )
                    .child(
                        h_flex()
                            .gap(theme::space_6())
                            .child("On delete")
                            .child(
                                div()
                                    .w(px(140.))
                                    .child(Select::new(&inputs.on_delete).small()),
                            )
                            .child("On update")
                            .child(
                                div()
                                    .w(px(140.))
                                    .child(Select::new(&inputs.on_update).small()),
                            )
                            .child(remove_button(
                                "fk-remove",
                                ix,
                                cx.listener(move |this, _, _, cx| this.remove_foreign_key(ix, cx)),
                            )),
                    )
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap(theme::space_6())
            .children(rows)
            .child(add_button(
                "fk-add",
                "Add foreign key",
                cx.listener(|this, _, window, cx| this.add_foreign_key(window, cx)),
            ))
            .into_any_element()
    }
}

/// A ghost "Add …" button, drawn as `id` for tests.
fn add_button(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .debug_selector(move || id.into())
        .child(
            Button::new(id)
                .ghost()
                .small()
                .label(label)
                .on_click(on_click),
        )
        .into_any_element()
}

/// Row `ix`'s Remove button, drawn as `<id>-<ix>` for tests.
fn remove_button(
    id: &'static str,
    ix: usize,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .debug_selector(move || format!("{id}-{ix}"))
        .child(
            Button::new((id, ix))
                .ghost()
                .small()
                .label("Remove")
                .on_click(on_click),
        )
        .into_any_element()
}

impl Render for TableEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let columns = self.render_columns(cx);
        let keys = self
            .form
            .as_ref()
            .map(|f| f.indexes.clone())
            .map(|indexes| {
                (
                    self.render_indexes(&indexes, cx),
                    self.render_foreign_keys(cx),
                )
            });
        v_flex()
            .gap(theme::space_12())
            .child(field(
                "name",
                "Name".into(),
                Input::new(&self.name).small(),
                cx,
            ))
            .child(field("columns", "Columns".into(), columns, cx))
            .children(
                keys.map(|(indexes, fks)| {
                    [
                        field("indexes", "Indexes".into(), indexes, cx),
                        field("foreign-keys", "Foreign keys".into(), fks, cx),
                    ]
                })
                .into_iter()
                .flatten(),
            )
            .child(sql_preview(&self.statements(), cx))
            .children(self.confirm.render(cx))
            .children(error_line(self.error.as_deref(), cx))
    }
}
