//! A table tab's Data | Structure | DDL sub-tabs (Data | Definition on a view). Structure and
//! DDL read the table's definition the first time either is shown; F5 on them reads it again.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{TableDef, Trigger};
use crate::ui::grid::{Grid, Table};
use crate::{ddl, theme};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sub {
    Data,
    Structure,
    /// DDL on a table, Definition on a view.
    Ddl,
}

const LOST: &str = "Disconnected";

enum Load {
    NotLoaded,
    Loading,
    Failed(String),
    Table(TableDef, Vec<Trigger>),
    View,
}

pub struct Structure {
    grid: Entity<Grid>,
    table: Table,
    /// The explorer row the tab was opened from, which Modify… selects.
    key: SharedString,
    pub sub: Sub,
    load: Load,
    /// The DDL or view definition, read-only.
    editor: Option<Entity<EditorState>>,
    focus: FocusHandle,
}

impl Structure {
    pub fn new(
        grid: Entity<Grid>,
        table: Table,
        key: SharedString,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            grid,
            table,
            key,
            sub: Sub::Data,
            load: Load::NotLoaded,
            editor: None,
            focus: cx.focus_handle(),
        }
    }

    /// Focus handle of the shown sub-tab.
    pub fn content_focus(&self, cx: &App) -> FocusHandle {
        match self.sub {
            Sub::Data => self.grid.focus_handle(cx),
            _ => self.focus.clone(),
        }
    }

    /// The DDL or view definition shown, empty until loaded.
    pub fn text(&self, cx: &App) -> String {
        self.editor
            .as_ref()
            .map_or(String::new(), |e| e.read(cx).value().to_string())
    }

    fn subs(&self) -> &'static [(Sub, &'static str)] {
        if self.table.view {
            &[(Sub::Data, "Data"), (Sub::Ddl, "Definition")]
        } else {
            &[
                (Sub::Data, "Data"),
                (Sub::Structure, "Structure"),
                (Sub::Ddl, "DDL"),
            ]
        }
    }

    pub fn show(&mut self, sub: Sub, window: &mut Window, cx: &mut Context<Self>) {
        self.sub = sub;
        if sub != Sub::Data && matches!(self.load, Load::NotLoaded) {
            self.reload(window, cx);
        }
        window.focus(&self.content_focus(cx), cx);
        cx.notify();
    }

    /// The connection was lost: what was read is dropped, and F5 reads it again.
    pub fn lost(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.load, Load::NotLoaded) {
            self.load = Load::Failed(LOST.into());
            cx.notify();
        }
    }

    /// Reads again what `lost` dropped, now that the connection is back.
    pub fn restored(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(&self.load, Load::Failed(r) if r == LOST) {
            self.load = Load::NotLoaded;
            if self.sub != Sub::Data {
                self.reload(window, cx);
            }
            cx.notify();
        }
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        let t = self.table.clone();
        let gone = || -> crate::db::Reply<crate::db::Session> {
            Box::pin(futures::future::ready(Err("The explorer is closed".into())))
        };
        let session = t
            .explorer
            .update(cx, |e, cx| e.session(t.connection, &t.database, cx))
            .unwrap_or_else(|_| gone());
        cx.spawn_in(window, async move |this, cx| {
            let read = async {
                let session = session.await?;
                if t.view {
                    let def = t.driver.view_def(&session, &t.schema, &t.name).await?;
                    return Ok((def, Load::View));
                }
                let (def, triggers) = futures::future::try_join(
                    t.driver.table_def(&session, &t.schema, &t.name),
                    t.driver.triggers(&session, &t.schema, &t.name),
                )
                .await?;
                Ok::<_, String>((
                    ddl::table_ddl(&t.schema, &def, &triggers),
                    Load::Table(def, triggers),
                ))
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                match read {
                    Ok((text, load)) => {
                        this.editor = Some(cx.new(|cx| {
                            EditorState::new(window, cx)
                                .language("sql")
                                .default_value(text)
                        }));
                        this.load = load;
                    }
                    Err(reason) => this.load = Load::Failed(reason),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .flex_none()
            .h(theme::tab())
            .px(theme::space_8())
            .gap(theme::space_12())
            .border_b_1()
            .border_color(theme.border)
            .text_size(theme::ui_sm())
            .children(self.subs().iter().map(|&(sub, label)| {
                let active = sub == self.sub;
                div()
                    .id(label)
                    .debug_selector(move || format!("subtab-{label}"))
                    .relative()
                    .h_full()
                    .flex()
                    .items_center()
                    .px(theme::space_4())
                    .cursor_pointer()
                    .text_color(if active {
                        theme.foreground
                    } else {
                        theme::text_secondary(theme)
                    })
                    .hover(|d| d.text_color(theme.foreground))
                    .child(label)
                    .when(active, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .h(theme::stripe())
                                .bg(theme.primary),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.show(sub, window, cx)))
            }))
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let note = |text: String| {
            v_flex()
                .debug_selector(|| format!("structure-state:{text}"))
                .items_start()
                .gap(theme::space_8())
                .p(theme::space_12())
                .text_size(theme::ui_sm())
                .text_color(theme::text_secondary(theme))
                .child(text)
        };
        match (&self.load, self.sub) {
            (_, Sub::Data) => self.grid.clone().into_any_element(),
            (Load::NotLoaded | Load::Loading, _) => note("Loading…".into()).into_any_element(),
            (Load::Failed(reason), _) => note(format!("Couldn't read the definition: {reason}"))
                .child(
                    div().debug_selector(|| "structure-retry".into()).child(
                        Button::new("structure-retry")
                            .small()
                            .label("Retry")
                            .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                    ),
                )
                .into_any_element(),
            (Load::Table(def, triggers), Sub::Structure) => self.sections(def, triggers, cx),
            _ => self.editor_view(cx),
        }
    }

    fn editor_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .px(theme::space_8())
                    .py(theme::space_4())
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("structure-copy")
                            .debug_selector(|| "structure-copy".into())
                            .ghost()
                            .small()
                            .label("Copy")
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(this.text(cx)))
                            })),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Editor::new(editor)
                        .readonly(true)
                        .bordered(false)
                        .h_full()
                        .text_size(theme::mono()),
                ),
            )
            .into_any_element()
    }

    fn sections(&self, def: &TableDef, triggers: &[Trigger], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = |id: String, cells: Vec<String>| {
            h_flex()
                .debug_selector(move || id)
                .gap(theme::space_12())
                .py(theme::space_2())
                .children(
                    cells
                        .into_iter()
                        .map(|c| div().w(px(180.)).truncate().child(c)),
                )
        };
        let section = |title: &'static str, empty: &'static str, rows: Vec<Div>| {
            let slug = empty
                .trim_start_matches("No ")
                .replace("foreign keys", "fks");
            v_flex()
                .gap(theme::space_4())
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .when(rows.is_empty(), |d| {
                    d.child(
                        div()
                            .debug_selector(move || format!("structure-no-{slug}"))
                            .text_color(theme::text_secondary(theme))
                            .child(empty),
                    )
                })
                .children(rows)
        };
        let yes = |on: bool, text: &str| if on { text.to_string() } else { String::new() };
        let columns = def.columns.iter().map(|c| {
            row(
                format!("structure-column-{}", c.name),
                vec![
                    c.name.clone(),
                    c.data_type.clone(),
                    if c.nullable { "NULL" } else { "NOT NULL" }.into(),
                    c.default.clone().unwrap_or_default(),
                    yes(c.is_pk, "PK"),
                    yes(c.identity, "identity"),
                ],
            )
        });
        let indexes = def
            .indexes
            .iter()
            .map(|i| (i, "unique"))
            .chain(def.unique_keys.iter().map(|u| (u, "unique constraint")));
        let indexes = indexes.map(|(i, unique)| {
            row(
                format!("structure-index-{}", i.name),
                vec![i.name.clone(), i.columns.join(", "), yes(i.unique, unique)],
            )
        });
        let fks = def.foreign_keys.iter().map(|f| {
            row(
                format!("structure-fk-{}", f.name),
                vec![
                    f.name.clone(),
                    f.columns.join(", "),
                    format!(
                        "{}.{} ({})",
                        f.ref_schema,
                        f.ref_table,
                        f.ref_columns.join(", ")
                    ),
                    format!("ON DELETE {} ON UPDATE {}", f.on_delete, f.on_update),
                ],
            )
        });
        let trigger_rows = triggers.iter().map(|t| {
            row(
                format!("structure-trigger-{}", t.name),
                vec![t.name.clone(), t.timing.clone(), t.events.clone()],
            )
        });
        v_flex()
            .id("structure-sections")
            .size_full()
            .overflow_y_scroll()
            .p(theme::space_12())
            .gap(theme::space_16())
            .text_size(theme::ui_sm())
            // Modify runs through the explorer, which only edits Postgres tables.
            .when(!self.table.sqlite_table, |d| {
                d.child(
                    h_flex().child(
                        Button::new("structure-modify")
                            .debug_selector(|| "structure-modify".into())
                            .small()
                            .label("Modify…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let key = this.key.clone();
                                this.table
                                    .explorer
                                    .update(cx, |e, cx| {
                                        e.reveal(&key, window, cx);
                                        e.modify_object(window, cx)
                                    })
                                    .ok();
                            })),
                    ),
                )
            })
            .child(section("Columns", "No columns", columns.collect()))
            .child(section("Indexes", "No indexes", indexes.collect()))
            .child(section("Foreign keys", "No foreign keys", fks.collect()))
            .child(section("Triggers", "No triggers", trigger_rows.collect()))
            .into_any_element()
    }
}

impl Render for Structure {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "f5" && this.sub != Sub::Data {
                    this.reload(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(self.strip(cx))
            .child(div().flex_1().min_h_0().child(self.body(cx)))
    }
}

#[cfg(test)]
mod tests;
