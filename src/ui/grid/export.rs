//! Export…: the dialog, writing the page or streaming every row to a file, and its progress.

use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt as _;
use gpui_kit::component::{Selectable as _, WindowExt as _, notification::Notification};

use super::table_page::select_sql;
use super::*;
use crate::db::{Driver, Reply, Session};
use crate::ddl::ident;
use crate::export::{self, Format, Job, Partial, Writer};

/// A console result's statement, which All rows runs again on a session of its own.
#[derive(Clone)]
pub struct Rerun {
    pub driver: Arc<dyn Driver>,
    pub sql: String,
    pub connect: Rc<dyn Fn() -> Reply<Session>>,
}

/// An All rows export underway.
pub(super) struct Exporting {
    written: usize,
    cancelled: Arc<AtomicBool>,
    /// The export's own session, once connected, for Cancel to stop.
    session: Option<(Arc<dyn Driver>, Session)>,
    _task: Task<()>,
}

/// What All rows reads: the driver, the statement and a fresh session to read it on.
struct AllRows {
    driver: Arc<dyn Driver>,
    sql: String,
    connect: Reply<Session>,
}

impl Grid {
    /// Export… in the toolbar: pick a format and scope, then where to save.
    pub(super) fn open_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let grid = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (format, all) = (grid.read(cx).export_format, grid.read(cx).export_all);
            let all_rows_off = grid.read(cx).all_rows_off();
            let pick = |id: &'static str, label: &'static str, on: bool| {
                Button::new(id)
                    .debug_selector(move || id.into())
                    .small()
                    .label(label)
                    .selected(on)
            };
            let formats = Format::ALL.into_iter().map(|f| {
                let grid = grid.clone();
                pick(f.ext(), f.label(), f == format).on_click(move |_, _, cx| {
                    grid.update(cx, |g, cx| {
                        g.export_format = f;
                        cx.notify();
                    })
                })
            });
            let scope = |id, label, to_all: bool| {
                let grid = grid.clone();
                pick(id, label, all == to_all).on_click(move |_, _, cx| {
                    grid.update(cx, |g, cx| {
                        g.export_all = to_all;
                        cx.notify();
                    })
                })
            };
            let go = grid.clone();
            dialog
                .title("Export")
                .child(
                    v_flex()
                        .gap(theme::space_8())
                        .child(h_flex().gap(theme::space_4()).children(formats))
                        .child(
                            h_flex()
                                .gap(theme::space_4())
                                .child(scope("export-page", "Current page", false))
                                .child(
                                    scope("export-all", "All rows", true)
                                        .disabled(all_rows_off.is_some())
                                        .when_some(all_rows_off, |b, tip| b.tooltip(tip)),
                                ),
                        ),
                )
                .footer(
                    h_flex()
                        .gap(theme::space_8())
                        .child(div().flex_1())
                        .child(
                            Button::new("export-cancel")
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("export-go")
                                .debug_selector(|| "export-go".into())
                                .primary()
                                .small()
                                .label("Export…")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    go.update(cx, |g, cx| g.ask_path(window, cx));
                                }),
                        ),
                )
        });
    }

    /// Why All rows can't be picked, if it can't.
    pub(super) fn all_rows_off(&self) -> Option<&'static str> {
        if self.live.is_some() {
            return None;
        }
        match &self.rerun {
            None => Some("Only query results can export every row"),
            Some(r) if !crate::sql::is_read_only(&r.sql) => {
                Some("All rows would run this writing statement again; export the current page")
            }
            Some(_) => None,
        }
    }

    /// The table tab's name or "result", as the file name and SQL INSERT target.
    fn export_name(&self) -> (String, String) {
        match self.source() {
            Some((t, _, _)) => (
                t.name.clone(),
                format!("{}.{}", ident(&t.schema), ident(&t.name)),
            ),
            None => ("result".into(), "result".into()),
        }
    }

    fn ask_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let all = self.export_all && self.all_rows_off().is_none();
        let format = self.export_format;
        let name = format!("{}.{}", self.export_name().0, format.ext());
        let dir = std::env::home_dir().unwrap_or_default();
        let picked = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let path = match picked.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(_)) => {
                    let reason = "The save dialog couldn't open".to_string();
                    this.update_in(cx, |_, window, cx| toast(Err(reason), window, cx))
                        .ok();
                    return;
                }
            };
            this.update_in(cx, |this, window, cx| match all {
                true => this.export_all_rows(format, path, window, cx),
                false => this.export_page(format, path, window, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Writes the current page, as shown (hidden columns skipped, pending edits in), off the main thread.
    fn export_page(
        &mut self,
        format: Format,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let block = (0..self.source.len, 0..self.col_order.len());
        let (cols, rows) = block_data(
            &block,
            &self.col_order,
            &self.hidden,
            self.source.columns(),
            &|r| self.row(r),
        );
        let table = self.export_name().1;
        let write = cx.background_spawn(async move {
            let written = write_page(format, cols, &rows, &table, &path);
            written.map(|n| (n, path))
        });
        cx.spawn_in(window, async move |_, cx| {
            let done = write.await;
            cx.update(|window, cx| toast(done, window, cx)).ok();
        })
        .detach();
    }

    pub(super) fn export_all_rows(
        &mut self,
        format: Format,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(AllRows {
            driver,
            sql,
            connect,
        }) = self.all_rows(cx)
        else {
            return;
        };
        let table = self.export_name().1;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let session = match connect.await {
                Ok(session) => session,
                Err(reason) => {
                    this.update_in(cx, |this, window, cx| {
                        this.exported(Err(reason), window, cx)
                    })
                    .ok();
                    return;
                }
            };
            let keep = (driver.clone(), session.clone());
            this.update(cx, |this, _| {
                if let Some(e) = &mut this.exporting {
                    e.session = Some(keep);
                }
            })
            .ok();
            let (tx, mut progress) = futures::channel::mpsc::unbounded();
            let job = Job {
                driver,
                session,
                sql,
                format,
                table,
                path: path.clone(),
                cancelled: stop,
            };
            let run = cx.background_spawn(export::run(job, move |n| {
                tx.unbounded_send(n).ok();
            }));
            while let Some(n) = progress.next().await {
                this.update(cx, |this, cx| {
                    if let Some(e) = &mut this.exporting {
                        e.written = n;
                        cx.notify();
                    }
                })
                .ok();
            }
            let done = run.await.map(|n| (n, path));
            this.update_in(cx, |this, window, cx| this.exported(done, window, cx))
                .ok();
        });
        self.exporting = Some(Exporting {
            written: 0,
            cancelled,
            session: None,
            _task: task,
        });
        cx.notify();
    }

    fn all_rows(&self, cx: &mut Context<Self>) -> Option<AllRows> {
        if let Some(r) = &self.rerun {
            return Some(AllRows {
                driver: r.driver.clone(),
                sql: r.sql.clone(),
                connect: (r.connect)(),
            });
        }
        let (t, where_, order_by) = self.source()?;
        let key = self.live.as_ref()?.key.clone().unwrap_or_default();
        Some(AllRows {
            driver: t.driver.clone(),
            sql: select_sql(&t.schema, &t.name, &key, false, (where_, order_by)),
            connect: t.sessions.read(cx).fresh(t.connection, &t.database, cx),
        })
    }

    /// Cancel on the progress row: stops the read; the partial file is removed.
    pub(super) fn cancel_export(&mut self, cx: &mut Context<Self>) {
        // The export's task sees the flag (or the cancelled fetch), removes the file and ends.
        let Some(e) = &self.exporting else { return };
        e.cancelled.store(true, Ordering::Relaxed);
        if let Some((driver, session)) = &e.session {
            cx.background_spawn(driver.cancel(session)).detach();
        }
    }

    fn exported(
        &mut self,
        done: Result<(usize, PathBuf), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.exporting = None;
        cx.notify();
        toast(done, window, cx);
    }

    /// Export… in the toolbar, or the rows written and Cancel while an export runs.
    pub(super) fn render_export(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(e) = &self.exporting else {
            return Button::new("grid-export")
                .debug_selector(|| "grid-export".into())
                .ghost()
                .with_size(super::toolbar::tool_size())
                .icon(IconName::Download)
                .tooltip("Export to a file…")
                .accessibility_label("Export to a file")
                .on_click(cx.listener(|this, _, window, cx| this.open_export(window, cx)))
                .into_any_element();
        };
        h_flex()
            .gap(theme::space_4())
            .child(
                div()
                    .debug_selector(|| "grid-export-progress".into())
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Exporting… {} rows", grouped(e.written))),
            )
            .child(
                Button::new("grid-export-cancel")
                    .debug_selector(|| "grid-export-cancel".into())
                    .ghost()
                    .small()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_export(cx))),
            )
            .into_any_element()
    }
}

fn write_page(
    format: Format,
    cols: Vec<ColumnMeta>,
    rows: &[Vec<Value>],
    table: &str,
    path: &Path,
) -> Result<usize, String> {
    let (file, partial) = Partial::create(path)?;
    let mut out = Writer::new(format, BufWriter::new(file), cols, table);
    out.header().map_err(|e| e.to_string())?;
    out.rows(rows).map_err(|e| e.to_string())?;
    let written = out.written;
    out.finish().map_err(|e| e.to_string())?;
    partial.keep();
    Ok(written)
}

/// "Exported N rows to <file>" with Open folder, or why it failed; nothing after Cancel.
fn toast(done: Result<(usize, PathBuf), String>, window: &mut Window, cx: &mut App) {
    let note = match done {
        Ok((n, path)) => {
            let file = path.file_name().unwrap_or_default().to_string_lossy();
            Notification::success(format!("Exported {} rows to {file}", grouped(n))).action(
                move |_, _, _| {
                    let path = path.clone();
                    Button::new("export-open-folder")
                        .small()
                        .label("Open folder")
                        .on_click(move |_, _, cx| cx.reveal_path(&path))
                },
            )
        }
        Err(reason) if reason == "Cancelled" => return,
        Err(reason) => Notification::error(format!("Export failed: {reason}")),
    };
    crate::ui::toast::show(note, window, cx);
}
