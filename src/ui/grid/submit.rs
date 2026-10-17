//! Pending edits, deletions and added rows as one transaction of SQL; Revert drops them.

use std::collections::BTreeMap;
use std::time::{Instant, SystemTime};

use gpui_kit::component::WindowExt as _;

use super::*;
use crate::ddl::ident;
use crate::model::{QueryError, QueryErrorKind, QueryStatus};

/// Why a table tab can't be edited: views, and Postgres tables with no primary key.
pub(super) fn read_only_note(
    view: bool,
    sqlite_table: bool,
    key: &[String],
) -> Option<&'static str> {
    if view {
        return Some("Views are read-only");
    }
    (key.is_empty() && !sqlite_table).then_some("No primary key: read-only")
}

/// The whole batch as (row, SQL): a DELETE per row in `deleted`, an UPDATE per other edited
/// loaded row, then an INSERT per row of `all` under `added`, naming only the cells filled in.
pub(super) fn changes(
    (schema, table, key): (&str, &str, &[String]),
    all: &dyn GridSource,
    added: usize,
    edits: &HashMap<(usize, usize), Value>,
    deleted: &BTreeSet<usize>,
) -> Vec<(usize, String)> {
    let columns = all.columns();
    let target = format!("{}.{}", ident(schema), ident(table));
    let mut rows: BTreeMap<usize, Vec<(usize, &Value)>> = BTreeMap::new();
    for (&(r, c), value) in edits {
        rows.entry(r).or_default().push((c, value));
    }
    rows.values_mut()
        .for_each(|cells| cells.sort_by_key(|&(c, _)| c));
    let name = |c: usize| ident(&columns[c].name);
    // Rows are matched by the key columns' values as read, so an edited key still finds its row.
    let key_of = |r: usize| {
        let row = all.row(r);
        let cells = key
            .iter()
            .filter_map(|k| columns.iter().position(|c| *c.name == **k))
            .map(|c| (c, &row[c]));
        pairs(columns, cells, " AND ")
    };
    let deletes = deleted
        .iter()
        .map(|&r| (r, format!("DELETE FROM {target} WHERE {}", key_of(r))));
    let updates = rows
        .range(added..)
        .filter(|(r, _)| !deleted.contains(r))
        .map(|(&r, cells)| {
            let sql = format!(
                "UPDATE {target} SET {} WHERE {}",
                pairs(columns, cells.iter().copied(), ", "),
                key_of(r)
            );
            (r, sql)
        });
    let inserts = (0..added).map(|r| {
        let sql = match rows.get(&r) {
            None => format!("INSERT INTO {target} DEFAULT VALUES"),
            Some(cells) => {
                let list = |f: &dyn Fn(&(usize, &Value)) -> String| {
                    cells.iter().map(f).collect::<Vec<_>>().join(", ")
                };
                format!(
                    "INSERT INTO {target} ({}) VALUES ({})",
                    list(&|&(c, _)| name(c)),
                    list(&|&(_, v)| sql_literal(v))
                )
            }
        };
        (r, sql)
    });
    deletes.chain(updates).chain(inserts).collect()
}

fn pairs<'a>(
    columns: &[ColumnMeta],
    cells: impl Iterator<Item = (usize, &'a Value)>,
    sep: &str,
) -> String {
    cells
        .map(|(c, v)| format!("{} = {}", ident(&columns[c].name), sql_literal(v)))
        .collect::<Vec<_>>()
        .join(sep)
}

impl Grid {
    /// Submit and Ctrl+Enter: the pending edits as one transaction, after a confirm on production.
    pub(super) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.stop_editing(true, window, cx) || !self.has_changes() {
            return;
        }
        let Some(live) = &self.live else { return };
        let (Some(key), None) = (&live.key, self.read_only()) else {
            return;
        };
        let t = &live.table;
        let rowid = ["rowid".to_string()];
        let key = if key.is_empty() { &rowid[..] } else { key };
        let statements = changes(
            (&t.schema, &t.name, key),
            &*self.all,
            self.added,
            &self.edits,
            &self.deleted,
        );
        let production = t
            .explorer
            .upgrade()
            .is_some_and(|e| e.read(cx).is_production(t.connection, cx));
        if production {
            return self.confirm_production(statements, window, cx);
        }
        self.run_updates(statements, cx);
    }

    /// Lists the exact SQL; only "Apply to production" runs it.
    fn confirm_production(
        &self,
        statements: Vec<(usize, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let grid = cx.entity();
        let n = statements.len();
        let title = format!(
            "Apply {n} {} to production?",
            if n == 1 { "change" } else { "changes" }
        );
        window.open_dialog(cx, move |dialog, _, cx| {
            let (grid, run) = (grid.clone(), statements.clone());
            let theme = cx.theme();
            dialog
                .title(
                    h_flex()
                        .gap(theme::space_8())
                        .child(theme::prod_badge(theme))
                        .child(title.clone()),
                )
                .child(
                    v_flex()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme::ui_sm())
                        .text_color(theme::text_secondary(theme))
                        .children(statements.iter().map(|(_, sql)| {
                            let sql = sql.clone();
                            div()
                                .debug_selector({
                                    let sql = sql.clone();
                                    move || format!("prod-submit-sql:{sql}")
                                })
                                .child(sql)
                        })),
                )
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1())
                        .child(
                            Button::new("prod-submit-cancel")
                                .debug_selector(|| "prod-submit-cancel".into())
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("prod-submit-confirm")
                                .debug_selector(|| "prod-submit-confirm".into())
                                .danger()
                                .small()
                                .label("Apply to production")
                                .on_click(move |_, window, cx| {
                                    let run = run.clone();
                                    grid.update(cx, |g, cx| g.run_updates(run, cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    fn run_updates(&mut self, statements: Vec<(usize, String)>, cx: &mut Context<Self>) {
        let session = self.session(cx);
        let Some(live) = &mut self.live else { return };
        live.submit_error = None;
        let driver = live.table.driver.clone();
        let sql: Vec<String> = statements.iter().map(|(_, s)| s.clone()).collect();
        live._submit = Some(cx.spawn(async move |this, cx| {
            let (at, started) = (SystemTime::now(), Instant::now());
            let result = match session.await {
                Ok(session) => driver.execute(&session, sql.clone(), true).await,
                Err(message) => Err(QueryError {
                    kind: QueryErrorKind::Disconnected,
                    message,
                    position: None,
                    statement: 0,
                }),
            };
            this.update(cx, |this, cx| {
                this.log_updates(&sql, (at, started), &result, cx);
                match result {
                    Ok(()) => {
                        this.revert(cx);
                        this.load(cx)
                    }
                    Err(error) => this.submit_failed(&statements, error, cx),
                }
            })
            .ok();
        }));
        cx.notify();
    }

    /// Each statement that ran, as TableView: the failing one with its error, those before it rolled back.
    fn log_updates(
        &self,
        sql: &[String],
        (at, started): (SystemTime, Instant),
        result: &Result<(), QueryError>,
        cx: &mut Context<Self>,
    ) {
        let ran = result.as_ref().err().map_or(sql.len(), |e| e.statement + 1);
        for (i, sql) in sql.iter().take(ran).enumerate() {
            let status = match result {
                Ok(()) => QueryStatus::Ok,
                Err(e) if e.statement == i => QueryStatus::Err(e.message.clone()),
                Err(_) => QueryStatus::Err("Rolled back".into()),
            };
            self.log_status(sql.clone(), (at, started), (None, status), cx);
        }
    }

    fn submit_failed(
        &mut self,
        statements: &[(usize, String)],
        error: QueryError,
        cx: &mut Context<Self>,
    ) {
        let row = statements.get(error.statement).map(|&(r, _)| r);
        if let Some(live) = &mut self.live {
            live.submit_error = Some((row, error.message));
            cx.notify();
        }
    }

    pub(super) fn revert(&mut self, cx: &mut Context<Self>) {
        self.edits.clear();
        self.deleted.clear();
        self.revision += 1;
        if self.added > 0 {
            self.set_added(0, cx);
        }
        if let Some(live) = &mut self.live {
            live.submit_error = None;
        }
        cx.notify();
    }

    /// The toolbar note on a table tab that can't be edited.
    pub(super) fn read_only(&self) -> Option<&'static str> {
        let live = self.live.as_ref()?;
        let key = live.key.as_ref()?;
        read_only_note(live.table.view, live.table.sqlite_table, key)
    }

    /// Row in `all` whose statement failed the last Submit.
    pub(super) fn failed_row(&self) -> Option<usize> {
        self.live.as_ref()?.submit_error.as_ref()?.0
    }

    /// Why the last Submit failed, under the toolbar.
    pub(super) fn render_submit_error(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let reason = self.live.as_ref()?.submit_error.as_ref()?.1.clone();
        let theme = cx.theme();
        Some(
            div()
                .debug_selector(|| format!("submit-error:{reason}"))
                .flex_none()
                .px(theme::space_8())
                .py(theme::space_2())
                .border_b_1()
                .border_color(theme.border)
                .text_size(theme::ui_sm())
                .text_color(theme.danger)
                .child(format!("Submit failed, nothing was saved: {reason}")),
        )
    }
}
