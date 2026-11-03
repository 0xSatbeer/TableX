//! Our own data grid (decision D3): rows virtualized by `uniform_list`, columns by
//! prefix-summed widths against the horizontal scroll offset.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Timelike as _};
use gpui_kit::assets::IconName;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::calendar::{Calendar, CalendarEvent, CalendarState, Date};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::time_field::{TimeField, TimeFieldState, TimePrecision};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
    input::{EditorState, Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{ColumnMeta, ResultSet, Row, Value};
use crate::{mock, theme};

mod completion;
mod copy;
mod edit;
mod export;
mod format;
mod header;
mod layout;
mod live;
mod parse;
mod paste;
mod pending;
mod picker;
mod rows;
mod selection;
mod source;
mod submit;
mod table_page;
mod toolbar;

pub use self::export::Rerun;
pub use self::live::{GridEvent, Table};
pub use self::source::{BigEvents, GridSource};
use self::{copy::*, format::*, layout::*, parse::*, selection::*, source::*};
#[cfg(test)]
use self::{edit::*, header::*, paste::*};

actions!(
    grid,
    [
        CopyCsv,
        CopyJson,
        CopyMarkdown,
        CopySqlInsert,
        CopySqlInList,
        HideColumn,
        ShowAllColumns,
        Paste,
        SetValue
    ]
);

pub struct Grid {
    /// The current page of `all`; everything but paging works on it.
    source: Rc<Page>,
    /// `loaded` under the `added` new rows; row indices into it are what edits and marks key by.
    all: Rc<dyn GridSource>,
    loaded: Rc<dyn GridSource>,
    added: usize,
    /// Rows in `all` marked for deletion on Submit.
    deleted: BTreeSet<usize>,
    page: usize,
    page_size: usize,
    widths: Vec<Pixels>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// Selection is the rectangle between anchor and head, as (row, col).
    anchor: (usize, usize),
    head: (usize, usize),
    /// (column, mouse x at drag start, width at drag start)
    resizing: Option<(usize, Pixels, Pixels)>,
    /// (column, ascending) and the display-to-source row permutation it produced.
    sort: Option<(usize, bool)>,
    order: Option<Vec<usize>>,
    /// Earlier ranges kept by Ctrl+click, as (rows, cols). Copy uses only the active range.
    extra: Vec<(Range<usize>, Range<usize>)>,
    /// Transposed one-row view of the head row (Ctrl+Shift+R).
    record: bool,
    hidden: Vec<bool>,
    /// Display position to source column. Widths, hidden and sort are keyed by source column,
    /// so they travel with it; selection and navigation work in display positions.
    col_order: Vec<usize>,
    /// Column the header context menu was opened on.
    menu_col: usize,
    /// This pane's mono font size, before UI scale (Ctrl+= / Ctrl+- / Ctrl+0).
    mono: Pixels,
    /// What Ctrl+C copies as, picked in the toolbar.
    copy_csv: bool,
    /// WHERE and ORDER BY inputs; made on first render, since EditorState needs the window.
    filter: Option<(Entity<EditorState>, Entity<EditorState>)>,
    /// Share of the filter bar the WHERE input takes, and (mouse x, share) while its edge is dragged.
    where_share: f32,
    filter_drag: Option<(Pixels, f32)>,
    /// Pending cell values by (row in `all`, source column), shown over the source until submitted.
    edits: HashMap<(usize, usize), Value>,
    /// Bumped on every edit, so the status bar recomputes an unchanged selection.
    revision: usize,
    /// The in-cell editor, while open (double-click, Enter, F2).
    editing: Option<Editing>,
    /// A refused paste: the cell that couldn't take its field, and why. Cleared by the next selection.
    notice: Option<((usize, usize), String)>,
    /// A left button pressed on a cell is still down, so moving over cells extends the selection.
    drag_selecting: bool,
    /// A table tab's database reads; `None` for a result already in memory.
    live: Option<live::Live>,
    /// A console result's statement, for Export's All rows.
    pub rerun: Option<Rerun>,
    /// A console result: shown, copied and exported, never edited.
    pub results: bool,
    /// The Export dialog's last picks, and the All rows export underway.
    export_format: crate::export::Format,
    export_all: bool,
    exporting: Option<export::Exporting>,
}

/// An open cell editor: the display cell it sits on and every cell its value goes to.
struct Editing {
    editor: Editor,
    cell: (usize, usize),
    // Fixed at open, since the click that blurs (and commits) it moves the selection first.
    targets: Vec<(usize, usize)>,
    /// Why the typed text can't be saved, shown under the cell until it is fixed.
    error: Option<String>,
}

enum Editor {
    /// Typed in place of the cell text.
    Text(Entity<InputState>),
    /// A list under the cell; `highlighted` is the row Enter picks.
    Options {
        values: Vec<Arc<str>>,
        current: Option<usize>,
        highlighted: usize,
    },
    /// A calendar under the cell, with a time field for timestamps. `original` is the cell's
    /// value, whose sub-second part survives when the time is left as it was.
    Date {
        calendar: Entity<CalendarState>,
        time: Option<(Entity<TimeFieldState>, Option<i32>)>,
        original: Value,
    },
}

pub type SelectionKey = (Range<usize>, Vec<usize>, Option<(usize, bool)>, usize);

impl Grid {
    pub fn new(source: Rc<dyn GridSource>, cx: &mut Context<Self>) -> Self {
        let n = source.columns().len();
        let mut grid = Self {
            source: Rc::new(Page::new(source.clone(), 0, PAGE_SIZES[1])),
            all: source.clone(),
            loaded: source,
            added: 0,
            deleted: BTreeSet::new(),
            page: 0,
            page_size: PAGE_SIZES[1],
            widths: vec![DEFAULT_WIDTH; n],
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            anchor: (0, 0),
            head: (0, 0),
            resizing: None,
            sort: None,
            order: None,
            extra: Vec::new(),
            record: false,
            hidden: vec![false; n],
            col_order: (0..n).collect(),
            menu_col: 0,
            mono: theme::GRID_MONO,
            copy_csv: false,
            filter: None,
            where_share: 0.6,
            filter_drag: None,
            edits: HashMap::new(),
            revision: 0,
            editing: None,
            notice: None,
            drag_selecting: false,
            live: None,
            rerun: None,
            results: false,
            export_format: crate::export::Format::Csv,
            export_all: false,
            exporting: None,
        };
        // Open fitted to the first rows, so timestamps and UUIDs don't arrive clipped.
        for c in 0..n {
            grid.autofit(c);
            grid.widths[c] = grid.widths[c].min(MAX_OPEN_WIDTH);
        }
        grid
    }
}

impl Grid {
    /// Shows `rs` in place of the rows from its first page, dropping pending edits. Column
    /// widths, order and hidden columns stay while the columns do; returns whether they changed.
    pub fn replace(&mut self, rs: ResultSet, cx: &mut Context<Self>) -> bool {
        let reset = rs.columns != self.all.columns();
        self.loaded = Rc::new(rs);
        self.all = self.loaded.clone();
        (self.added, self.deleted) = (0, BTreeSet::new());
        self.edits.clear();
        self.editing = None;
        self.set_page(0, self.page_size, cx);
        if reset {
            let n = self.all.columns().len();
            (self.widths, self.hidden) = (vec![DEFAULT_WIDTH; n], vec![false; n]);
            self.col_order = (0..n).collect();
            for c in 0..n {
                self.autofit(c);
                self.widths[c] = self.widths[c].min(MAX_OPEN_WIDTH);
            }
        }
        reset
    }
}

impl Focusable for Grid {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Grid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (bottom_room, right_room) = self.scrollbar_room();
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &theme::ZoomIn, _, cx| this.zoom(theme::Zoom::In, cx)))
            .on_action(
                cx.listener(|this, _: &theme::ZoomOut, _, cx| this.zoom(theme::Zoom::Out, cx)),
            )
            .on_action(
                cx.listener(|this, _: &theme::ZoomReset, _, cx| this.zoom(theme::Zoom::Reset, cx)),
            )
            .on_action(cx.listener(|this, _: &CopyCsv, _, cx| this.copy_as(csv, cx)))
            .on_action(cx.listener(|this, _: &HideColumn, _, cx| {
                // Keep at least one column visible.
                if this.hidden.iter().filter(|h| !**h).count() > 1 {
                    this.hidden[this.menu_col] = true;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ShowAllColumns, _, cx| {
                this.hidden.fill(false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyJson, _, cx| this.copy_as(json, cx)))
            .on_action(cx.listener(|this, _: &CopyMarkdown, _, cx| this.copy_as(markdown, cx)))
            .on_action(cx.listener(|this, _: &CopySqlInsert, _, cx| this.copy_as(sql_insert, cx)))
            .on_action(cx.listener(|this, _: &CopySqlInList, _, cx| this.copy_in_list(cx)))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &SetValue, window, cx| {
                this.start_editing(this.head, None, window, cx)
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some((c, start_x, start_w)) = this.resizing {
                    this.widths[c] = (start_w + ev.position.x - start_x).max(MIN_WIDTH);
                    cx.notify();
                }
                if let Some((start_x, start_share)) = this.filter_drag {
                    let width = this.scroll.0.borrow().base_handle.bounds().size.width;
                    if width > Pixels::ZERO {
                        this.where_share =
                            (start_share + (ev.position.x - start_x) / width).clamp(0.15, 0.85);
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    (this.resizing, this.filter_drag, this.drag_selecting) = (None, None, false)
                }),
            )
            // Re-render on scroll so the header and column window follow the offset.
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .child(self.render_toolbar(cx))
            .children(self.render_filter_bar(window, cx))
            .children(self.render_filter_error(cx))
            .children(self.render_submit_error(cx))
            .when(!self.record, |d| d.child(self.render_header(window, cx)))
            .when(self.record && self.source.row_count() > 0, |d| {
                d.child(self.render_record(cx))
            })
            .when(!self.record, |d| {
                d.child(
                    div()
                        .id("grid-body")
                        .debug_selector(|| "grid-body".into())
                        .bg(cx.theme().table)
                        .relative()
                        .flex_1()
                        .min_h_0()
                        // The row-number gutter runs on under the last row, through the scrollbar gap.
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .w(self.gutter())
                                .h_full()
                                .bg(cx.theme().table_head)
                                .border_r_1()
                                .border_color(cx.theme().border),
                        )
                        .child(
                            uniform_list(
                                "grid-rows",
                                self.source.row_count(),
                                cx.processor(Self::render_rows),
                            )
                            .track_scroll(&self.scroll)
                            .with_horizontal_sizing_behavior(
                                ListHorizontalSizingBehavior::Unconstrained,
                            )
                            .pb(bottom_room)
                            .pr(right_room)
                            .size_full(),
                        )
                        .child(self.render_col_lines(window, cx))
                        // Always shown: where you are in the rows and columns is worth seeing at rest.
                        .child(
                            Scrollbar::new(&self.scroll)
                                .id("grid-scrollbar")
                                .mode(ScrollbarMode::Always),
                        )
                        .children(self.render_live_state(cx))
                        .context_menu({
                            let focus = self.focus.clone();
                            let grid = cx.entity().downgrade();
                            move |menu, _, _| {
                                let grid = grid.clone();
                                menu.action_context(focus.clone())
                                    .min_w(theme::menu())
                                    // The everyday copy first, in the format Ctrl+C uses.
                                    .item(
                                        PopupMenuItem::new("Copy (Ctrl+C)")
                                            .icon(Icon::new(Lucide::Copy))
                                            .on_click(move |_, _, cx| {
                                                _ = grid.update(cx, |this, cx| {
                                                    this.copy_as(
                                                        if this.copy_csv { csv } else { tsv },
                                                        cx,
                                                    )
                                                });
                                            }),
                                    )
                                    .separator()
                                    .label("Copy as")
                                    .menu_with_icon(
                                        "CSV",
                                        Icon::new(Lucide::Sheet),
                                        Box::new(CopyCsv),
                                    )
                                    .menu_with_icon(
                                        "JSON",
                                        Icon::new(Lucide::Braces),
                                        Box::new(CopyJson),
                                    )
                                    .menu_with_icon(
                                        "Markdown",
                                        Icon::new(Lucide::FileText),
                                        Box::new(CopyMarkdown),
                                    )
                                    .menu_with_icon(
                                        "SQL INSERT",
                                        Icon::new(Lucide::Code),
                                        Box::new(CopySqlInsert),
                                    )
                                    .menu_with_icon(
                                        "SQL IN-list",
                                        Icon::new(Lucide::List),
                                        Box::new(CopySqlInList),
                                    )
                                    .separator()
                                    .menu_with_icon(
                                        "Paste (Ctrl+V)",
                                        Icon::new(Lucide::ClipboardPaste),
                                        Box::new(Paste),
                                    )
                                    .menu_with_icon(
                                        "Edit selected cells… (Enter)",
                                        Icon::new(Lucide::Pencil),
                                        Box::new(SetValue),
                                    )
                            }
                        }),
                )
            })
    }
}

#[cfg(test)]
mod tests;
