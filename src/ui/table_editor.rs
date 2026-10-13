//! Table → Modify: the table's name, columns, indexes and foreign keys, with the `alter_table` SQL they need. The
//! edits live in a `TableForm`; the view keeps it in step with its inputs.

mod form;
mod lines;

pub use form::*;

use std::sync::Arc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::Cancel,
    menu::{DropdownMenu as _, PopupMenuItem},
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, StyledText, Window,
    div, px,
};

use crate::db::{Driver, Reply, Session};
use crate::model::{ForeignKey, Index};
use crate::theme;
use crate::ui::object_dialog::{Apply, ProductionConfirm, error_line, field, footer, sql_preview};
use lines::Spans;

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

const DIALOG_W: f32 = 900.;

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
    /// The selected tab: columns, indexes, foreign keys, SQL.
    tab: usize,
    /// The selected row of the columns, indexes and foreign keys lists.
    selected: [Option<usize>; 3],
    /// The row of each list open as a form in place of its line.
    editing: [Option<usize>; 3],
    /// The lists take keys: up/down select, Delete removes, Enter opens or closes the form.
    list_focus: FocusHandle,
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
                .w(px(DIALOG_W))
                .child(body.clone())
                .footer(footer(enabled, move |_, cx| {
                    on_apply.update(cx, |d, cx| d.apply(cx))
                }))
        });
        // The footer reads Apply's state when the dialog layer renders, which a body notify skips.
        window.observe(&view, cx, |_, window, _| window.refresh()).detach();
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
            tab: 0,
            selected: [None; 3],
            editing: [None; 3],
            list_focus: cx.focus_handle(),
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
        self.selected = [
            form.columns.len(),
            form.indexes.len(),
            form.foreign_keys.len(),
        ]
        .map(|n| (n > 0).then_some(0));
        // A new table opens on its first, empty column, ready to type.
        self.editing = [form.is_create().then_some(0), None, None];
        self.form = Some(form);
        // A name typed while the definition loaded wins over the one it brought.
        self.sync(cx);
    }

    fn row(c: &crate::model::Column, window: &mut Window, cx: &mut Context<Self>) -> ColumnInputs {
        ColumnInputs {
            name: Self::input(&c.name, "column_name", window, cx),
            ty: Self::input(&c.data_type, "e.g. integer", window, cx),
            default: Self::input(c.default.as_deref().unwrap_or(""), "", window, cx),
        }
    }

    fn index_row(i: &Index, window: &mut Window, cx: &mut Context<Self>) -> IndexInputs {
        IndexInputs {
            name: Self::input(&i.name, "name", window, cx),
            columns: Self::input(&i.columns.join(", "), "a, b", window, cx),
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
        // Ready to type the new row's name.
        row.name.update(cx, |input, cx| input.focus(window, cx));
        self.rows.push(row);
        self.selected[0] = Some(self.rows.len() - 1);
        self.editing[0] = self.selected[0];
        cx.notify();
    }

    fn remove_column(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.columns.remove(ix);
        self.rows.remove(ix);
        self.selected[0] = self.rows.len().checked_sub(1).map(|last| ix.min(last));
        self.editing[0] = None;
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
        // Ready to type the new row's name.
        row.name.update(cx, |input, cx| input.focus(window, cx));
        self.indexes.push(row);
        self.selected[1] = Some(self.indexes.len() - 1);
        self.editing[1] = self.selected[1];
        cx.notify();
    }

    fn remove_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.indexes.remove(ix);
        self.indexes.remove(ix);
        self.selected[1] = self.indexes.len().checked_sub(1).map(|last| ix.min(last));
        self.editing[1] = None;
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
        // Ready to type the new row's name.
        row.name.update(cx, |input, cx| input.focus(window, cx));
        self.foreign_keys.push(row);
        self.selected[2] = Some(self.foreign_keys.len() - 1);
        self.editing[2] = self.selected[2];
        cx.notify();
    }

    fn remove_foreign_key(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        form.foreign_keys.remove(ix);
        self.foreign_keys.remove(ix);
        self.selected[2] = self
            .foreign_keys
            .len()
            .checked_sub(1)
            .map(|last| ix.min(last));
        self.editing[2] = None;
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

    /// Selects row `ix`, closing a form open on another row.
    fn select(&mut self, list: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected[list] = Some(ix);
        if self.editing[list] != Some(ix) {
            self.editing[list] = None;
        }
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    /// Opens row `ix` as a form, its name focused.
    fn edit(&mut self, list: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected[list] = Some(ix);
        self.editing[list] = Some(ix);
        if let Some(name) = self.selected_name(list) {
            name.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn len(&self, list: usize) -> usize {
        [self.rows.len(), self.indexes.len(), self.foreign_keys.len()][list]
    }

    fn add(&mut self, list: usize, window: &mut Window, cx: &mut Context<Self>) {
        match list {
            0 => self.add_column(window, cx),
            1 => self.add_index(window, cx),
            _ => self.add_foreign_key(window, cx),
        }
    }

    fn remove_selected(&mut self, list: usize, cx: &mut Context<Self>) {
        let Some(ix) = self.selected[list] else {
            return;
        };
        match list {
            0 => self.remove_column(ix, cx),
            1 => self.remove_index(ix, cx),
            _ => self.remove_foreign_key(ix, cx),
        }
    }

    /// The selected row's name input, where Enter takes you.
    fn selected_name(&self, list: usize) -> Option<Entity<InputState>> {
        let ix = self.selected[list]?;
        match list {
            0 => self.rows.get(ix).map(|r| r.name.clone()),
            1 => self.indexes.get(ix).map(|r| r.name.clone()),
            _ => self.foreign_keys.get(ix).map(|r| r.name.clone()),
        }
    }

    fn on_list_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.tab;
        let key = ev.keystroke.key.as_str();
        // Keys typed in an open form are the form's, except the two that close it.
        if self.editing[list].is_some() {
            if key != "enter" {
                return;
            }
            self.editing[list] = None;
            window.focus(&self.list_focus, cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let last = self.len(list).checked_sub(1);
        match key {
            "up" => {
                if let Some(ix) = self.selected[list] {
                    self.select(list, ix.saturating_sub(1), window, cx);
                }
            }
            "down" => {
                if let (Some(ix), Some(last)) = (self.selected[list], last) {
                    self.select(list, (ix + 1).min(last), window, cx);
                }
            }
            "delete" => self.remove_selected(list, cx),
            "enter" => {
                if let Some(ix) = self.selected[list] {
                    self.edit(list, ix, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The `+ −` toolbar and the list of one-line summaries, with `form` in place of the line
    /// being edited.
    fn render_list(
        &self,
        list: usize,
        lines: Vec<(String, Spans, bool)>,
        mut form: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = ["column", "index", "fk"][list];
        let noun = ["column", "index", "foreign key"][list];
        let theme = cx.theme();
        let rows = lines
            .into_iter()
            .enumerate()
            .map(|(ix, (text, spans, is_pk))| {
                if self.editing[list] == Some(ix) {
                    if let Some(form) = form.take() {
                        return div()
                            .debug_selector(move || format!("{id}-{ix}"))
                            .mx(theme::space_4())
                            .my(theme::space_4())
                            .p(theme::space_8())
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(theme.list_active_border)
                            .bg(theme.background)
                            .font_family(theme.font_family.clone())
                            .text_size(theme::ui())
                            .child(form)
                            .into_any_element();
                    }
                }
                let selected = self.selected[list] == Some(ix);
                let highlights: Vec<_> = spans
                    .into_iter()
                    .filter_map(|(r, key)| Some((r, theme.highlight_theme.style(key)?)))
                    .collect();
                let line = div()
                    .when(is_pk, |d| {
                        d.debug_selector(move || format!("column-pk-{ix}"))
                    })
                    .child(StyledText::new(text).with_highlights(highlights));
                div()
                    .id((id, ix))
                    .debug_selector(move || format!("{id}-{ix}"))
                    .px(theme::space_8())
                    .py(theme::space_2())
                    .whitespace_nowrap()
                    .when(selected, |d| d.bg(theme.list_active))
                    .when(!selected, |d| d.hover(|d| d.bg(theme.list_hover)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            if ev.click_count >= 2 {
                                this.edit(list, ix, window, cx)
                            } else {
                                this.select(list, ix, window, cx)
                            }
                        }),
                    )
                    .child(line)
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let empty = rows.is_empty();
        let tool = |name: &'static str, icon: Lucide, tip: String| {
            Button::new(name)
                .debug_selector(move || name.into())
                .ghost()
                .small()
                .icon(icon)
                .tooltip(tip)
        };
        let add = format!("{id}-add").leak();
        let remove = format!("{id}-remove").leak();
        v_flex()
            .gap(theme::space_8())
            .child(
                v_flex()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap(theme::space_2())
                            .px(theme::space_4())
                            .py(theme::space_2())
                            .border_b_1()
                            .border_color(theme.border)
                            .child(tool(add, Lucide::Plus, format!("Add {noun}")).on_click(
                                cx.listener(move |this, _, window, cx| this.add(list, window, cx)),
                            ))
                            .child(
                                tool(remove, Lucide::Minus, format!("Remove {noun} (Delete)"))
                                    .disabled(self.selected[list].is_none())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_selected(list, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id(("table-editor-list", list))
                            .track_focus(&self.list_focus)
                            .on_key_down(cx.listener(Self::on_list_key))
                            // Escape is the dialog's Cancel: with a form open it closes the form only.
                            .on_action(cx.listener(|this, _: &Cancel, window, cx| {
                                let list = this.tab;
                                if this.editing[list].take().is_none() {
                                    return cx.propagate();
                                }
                                window.focus(&this.list_focus, cx);
                                cx.notify();
                            }))
                            .h(px(LIST_H))
                            .overflow_y_scroll()
                            .py(theme::space_2())
                            .font_family(theme.mono_font_family.clone())
                            .text_size(theme::mono())
                            .when(empty, |d| {
                                d.child(
                                    div()
                                        .p(theme::space_8())
                                        .text_color(theme::text_secondary(theme))
                                        .child(format!("No {noun}s. Press + to add one.")),
                                )
                            })
                            .children(rows),
                    ),
            )
            .into_any_element()
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
        let pad = lines::pad(form.columns.iter().map(|(_, c)| c.name.as_str()));
        let rows = form
            .columns
            .iter()
            .map(|(_, c)| {
                let (text, spans) = lines::column_line(c, pad);
                (text, spans, c.is_pk)
            })
            .collect();
        let open = self.editing[0].and_then(|ix| {
            let (inputs, (_, c)) = (self.rows.get(ix)?, form.columns.get(ix)?);
            Some(
                v_flex()
                    .gap(theme::space_8())
                    .child(
                        h_flex()
                            .gap(theme::space_8())
                            .child(labeled("Name", Input::new(&inputs.name).small(), cx))
                            .child(labeled("Type", self.type_input(ix, &inputs.ty, form, cx), cx))
                            .child(labeled("Default", Input::new(&inputs.default).small(), cx)),
                    )
                    .child(
                        h_flex().justify_end().child(
                            Checkbox::new("column-not-null")
                                .debug_selector(|| "column-not-null".into())
                                .label("Not null")
                                .checked(!c.nullable)
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.set_nullable(ix, !*checked, cx)
                                })),
                        ),
                    )
                    .into_any_element(),
            )
        });
        self.render_list(0, rows, open, cx)
    }

    /// The Type input, free text, with a menu of types that fills it in.
    fn type_input(
        &self,
        ix: usize,
        input: &Entity<InputState>,
        form: &TableForm,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (choices, current) = (form.type_choices(), form.columns[ix].1.data_type.clone());
        let (input_for_menu, editor) = (input.clone(), cx.entity().downgrade());
        let menu = Button::new("column-type-menu")
            .debug_selector(|| "column-type-menu".into())
            .ghost()
            .xsmall()
            .icon(Lucide::ChevronDown)
            .tooltip("Pick a type")
            .dropdown_menu(move |menu, _, _| {
                choices.iter().fold(menu.scrollable(true).max_h(px(320.)), |menu, t| {
                    let (input, editor, t) = (input_for_menu.clone(), editor.clone(), t.clone());
                    menu.item(PopupMenuItem::new(t.clone()).checked(t == current).on_click(
                        move |_, window, cx| {
                            input.update(cx, |i, cx| i.set_value(t.clone(), window, cx));
                            // set_value emits no change event, so the form is told here.
                            _ = editor.update(cx, |e, cx| e.sync(cx));
                        },
                    ))
                })
            });
        Input::new(input).small().suffix(menu).into_any_element()
    }

    fn render_indexes(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = &self.form else {
            return div().into_any_element();
        };
        let pad = lines::pad(form.indexes.iter().map(|i| i.name.as_str()));
        let rows = form
            .indexes
            .iter()
            .map(|i| {
                let (text, spans) = lines::index_line(i, pad);
                (text, spans, false)
            })
            .collect();
        let open = self.editing[1].and_then(|ix| {
            let (inputs, i) = (self.indexes.get(ix)?, form.indexes.get(ix)?);
            Some(
                v_flex()
                    .gap(theme::space_8())
                    .child(
                        h_flex()
                            .gap(theme::space_8())
                            .child(labeled("Name", Input::new(&inputs.name).small(), cx))
                            .child(labeled(
                                "Columns (comma-separated)",
                                Input::new(&inputs.columns).small(),
                                cx,
                            )),
                    )
                    .child(
                        h_flex().justify_end().child(
                            Checkbox::new("index-unique")
                                .debug_selector(|| "index-unique".into())
                                .label("Unique")
                                .checked(i.unique)
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.set_unique(ix, *checked, cx)
                                })),
                        ),
                    )
                    .into_any_element(),
            )
        });
        self.render_list(1, rows, open, cx)
    }

    fn render_foreign_keys(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = &self.form else {
            return div().into_any_element();
        };
        let pad = lines::pad(form.foreign_keys.iter().map(|f| f.name.as_str()));
        let rows = form
            .foreign_keys
            .iter()
            .map(|f| {
                let (text, spans) = lines::fk_line(f, pad);
                (text, spans, false)
            })
            .collect();
        let open = self.editing[2].and_then(|ix| {
            let inputs = self.foreign_keys.get(ix)?;
            let input = |label, i: &Entity<InputState>| labeled(label, Input::new(i).small(), cx);
            let select = |label, s: &ActionSelect| labeled(label, Select::new(s).small(), cx);
            Some(
                v_flex()
                    .gap(theme::space_8())
                    .child(
                        h_flex()
                            .gap(theme::space_8())
                            .child(input("Name", &inputs.name))
                            .child(input("Columns", &inputs.columns)),
                    )
                    .child(
                        h_flex()
                            .gap(theme::space_8())
                            .child(input("References schema", &inputs.ref_schema))
                            .child(input("Table", &inputs.ref_table))
                            .child(input("Columns", &inputs.ref_columns)),
                    )
                    .child(
                        h_flex()
                            .gap(theme::space_8())
                            .child(select("On delete", &inputs.on_delete))
                            .child(select("On update", &inputs.on_update))
                            // Keeps the selects as wide as the inputs above them.
                            .child(div().flex_1()),
                    )
                    .into_any_element(),
            )
        });
        self.render_list(2, rows, open, cx)
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let form = self.form.as_ref();
        let count = |n: Option<usize>| n.map_or(String::new(), |n| format!(" ({n})"));
        let labels = [
            (
                "tab-columns",
                format!("Columns{}", count(form.map(|f| f.columns.len()))),
            ),
            (
                "tab-indexes",
                format!("Indexes{}", count(form.map(|f| f.indexes.len()))),
            ),
            (
                "tab-fks",
                format!("Foreign keys{}", count(form.map(|f| f.foreign_keys.len()))),
            ),
            (
                "tab-sql",
                format!(
                    "SQL{}",
                    count(Some(self.statements().len()).filter(|n| *n > 0))
                ),
            ),
        ];
        TabBar::new("table-editor-tabs")
            .underline()
            .small()
            .selected_index(self.tab)
            .children(
                labels.map(|(id, label)| Tab::new().label(label).debug_selector(move || id.into())),
            )
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.tab = *ix;
                cx.notify();
            }))
            .into_any_element()
    }
}

/// A small label over `control`, taking an equal share of its row.
fn labeled(label: &'static str, control: impl IntoElement, cx: &App) -> gpui_kit::Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap(theme::space_2())
        .child(
            div()
                .text_size(theme::ui_sm())
                .text_color(theme::text_secondary(cx.theme()))
                .child(label),
        )
        .child(control)
}

impl Render for TableEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            1 => self.render_indexes(cx),
            2 => self.render_foreign_keys(cx),
            3 => div()
                .id("table-editor-sql")
                .h(px(LIST_H + LIST_CHROME))
                .overflow_y_scroll()
                .child(sql_preview(&self.statements(), cx))
                .into_any_element(),
            _ => self.render_columns(cx),
        };
        v_flex()
            .gap(theme::space_12())
            .child(field(
                "name",
                "Table".into(),
                Input::new(&self.name).small(),
                cx,
            ))
            .child(
                v_flex()
                    .gap(theme::space_8())
                    .child(self.render_tabs(cx))
                    .child(body)
                    .child(self.render_status(cx)),
            )
            .children(self.confirm.render(cx))
            .children(error_line(self.error.as_deref(), cx))
    }
}

impl TableEditor {
    /// What Apply will do, or why it can't yet.
    fn render_status(&self, cx: &App) -> AnyElement {
        let n = self.statements().len();
        let blocker = self.form.as_ref().and_then(TableForm::blocker);
        let (text, color) = match (n, blocker) {
            (0, Some(why)) => (why.to_string(), cx.theme().warning),
            (0, None) => ("No changes yet.".to_string(), theme::text_secondary(cx.theme())),
            (1, _) => ("1 statement to run. Review it on the SQL tab.".to_string(), theme::text_secondary(cx.theme())),
            (n, _) => (format!("{n} statements to run. Review them on the SQL tab."), theme::text_secondary(cx.theme())),
        };
        div()
            .debug_selector(|| "table-status".into())
            .text_size(theme::ui_sm())
            .text_color(color)
            .child(text)
            .into_any_element()
    }
}

const LIST_H: f32 = 410.;
/// The toolbar and borders around the list, so the SQL tab matches the others' height.
const LIST_CHROME: f32 = 40.;
