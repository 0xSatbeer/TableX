//! The in-cell editor: opening it per column type, its value, and closing it.

use super::*;

/// How a cell of this column is edited.
#[derive(Debug, PartialEq)]
pub(super) enum EditorKind {
    Text,
    /// Pick one of these labels: an enum's, or a boolean's true / false.
    Options(Vec<Arc<str>>),
    Date,
    /// Date and time; `zoned` for timestamptz.
    DateTime {
        zoned: bool,
    },
}

pub(super) fn editor_kind(meta: &ColumnMeta) -> EditorKind {
    let ty = meta.data_type.to_lowercase();
    match () {
        _ if ty.ends_with("[]") => EditorKind::Text,
        _ if !meta.enum_values.is_empty() => EditorKind::Options(meta.enum_values.to_vec()),
        _ if ty.starts_with("bool") => EditorKind::Options(vec!["true".into(), "false".into()]),
        _ if ty == "date" => EditorKind::Date,
        _ if ty.starts_with("timestamp") => EditorKind::DateTime {
            zoned: has_zone(&ty),
        },
        _ => EditorKind::Text,
    }
}

impl Grid {
    /// Opens the editor for display cell `cell`'s type, showing its value; it writes to every selected cell.
    pub(super) fn start_editing(
        &mut self,
        cell: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let meta = &self.source.columns()[self.col_order[cell.1]];
        let value = self.row(cell.0)[self.col_order[cell.1]].clone();
        let text = if value == Value::Null {
            String::new()
        } else {
            value.to_string()
        };
        let editor = match editor_kind(meta) {
            EditorKind::Text => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("NULL"));
                input.update(cx, |input, cx| {
                    input.set_value(text, window, cx);
                    input.focus(window, cx);
                });
                cx.subscribe_in(
                    &input,
                    window,
                    |this, input, ev: &InputEvent, window, cx| match ev {
                        // Clicking elsewhere keeps what was typed, as in a spreadsheet.
                        InputEvent::PressEnter { .. } | InputEvent::Blur => {
                            _ = this.stop_editing(true, window, cx)
                        }
                        // Once a reason shows, it follows the typing, so it goes away when the text is fixed.
                        InputEvent::Change => {
                            let text = input.read(cx).value();
                            if let Some(editing) = &this.editing
                                && editing.error.is_some()
                            {
                                let error = parse_value(
                                    &text,
                                    &this.source.columns()[this.col_order[editing.cell.1]],
                                )
                                .err();
                                this.editing.as_mut().unwrap().error = error;
                                cx.notify();
                            }
                        }
                        _ => {}
                    },
                )
                .detach();
                Editor::Text(input)
            }
            EditorKind::Options(values) => {
                let current = values.iter().position(|v| **v == *text);
                Editor::Options {
                    highlighted: current.unwrap_or(0),
                    current,
                    values,
                }
            }
            kind => {
                let zoned = matches!(kind, EditorKind::DateTime { zoned: true });
                let shown = wall_time(&value).or_else(|| {
                    NaiveDate::parse_from_str(&text, "%Y-%m-%d")
                        .ok()
                        .map(|d| (d.and_time(NaiveTime::MIN), 0))
                });
                let calendar = cx.new(|cx| CalendarState::new(window, cx));
                calendar.update(cx, |c, cx| {
                    c.set_date(Date::Single(shown.map(|(at, _)| at.date())), window, cx)
                });
                let time = (kind != EditorKind::Date).then(|| {
                    let field = cx
                        .new(|cx| TimeFieldState::new(window, cx).precision(TimePrecision::Second));
                    field.update(cx, |f, cx| {
                        f.set_time(
                            shown.map_or(NaiveTime::MIN, |(at, _)| at.time()),
                            window,
                            cx,
                        )
                    });
                    (field, zoned.then(|| shown.map_or(0, |(_, offset)| offset)))
                });
                if time.is_none() {
                    // A date is one click: picking the day is the whole edit.
                    cx.subscribe_in(
                        &calendar,
                        window,
                        |this, _, CalendarEvent::Selected(_), window, cx| {
                            _ = this.stop_editing(true, window, cx)
                        },
                    )
                    .detach();
                }
                Editor::Date {
                    calendar,
                    time,
                    original: value,
                }
            }
        };
        self.notice = None;
        self.editing = Some(Editing {
            editor,
            cell,
            targets: self.selected_cells(),
            error: None,
        });
        cx.notify();
    }

    /// The value an editor holds: `Ok(None)` for a calendar with no day picked, `Err` for text
    /// that isn't a value of the column.
    pub(super) fn editor_value(
        &self,
        editor: &Editor,
        column: &ColumnMeta,
        cx: &App,
    ) -> Result<Option<Value>, String> {
        let day = |calendar: &Entity<CalendarState>| calendar.read(cx).date().start();
        Ok(Some(match editor {
            Editor::Text(input) => parse_value(&input.read(cx).value(), column)?,
            Editor::Options {
                values,
                highlighted,
                ..
            } => parse_value(&values[*highlighted], column)?,
            Editor::Date {
                calendar,
                time: None,
                ..
            } => match day(calendar) {
                Some(day) => Value::Text(day.format("%Y-%m-%d").to_string().into()),
                None => return Ok(None),
            },
            Editor::Date {
                calendar,
                time: Some((field, offset)),
                original,
            } => {
                let Some(day) = day(calendar) else {
                    return Ok(None);
                };
                let mut at = day.and_time(field.read(cx).time());
                // The field edits whole seconds; keep the original's fraction if they match.
                if let Some((was, _)) =
                    wall_time(original).filter(|(was, _)| was.with_nanosecond(0) == Some(at))
                {
                    at = was;
                }
                timestamp_value(at, *offset)
            }
        }))
    }

    /// Closes the editor, writing its value to its targets when `commit`. A value the column
    /// can't hold keeps it open with the reason under the cell; returns whether it closed.
    pub(super) fn stop_editing(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editing) = &self.editing else {
            return true;
        };
        if commit {
            let column = &self.source.columns()[self.col_order[editing.cell.1]];
            match self.editor_value(&editing.editor, column, cx) {
                Err(why) => {
                    self.editing.as_mut().unwrap().error = Some(why);
                    cx.notify();
                    return false;
                }
                Ok(Some(value)) => {
                    let cells = editing
                        .targets
                        .iter()
                        .map(|&cell| (cell, value.clone()))
                        .collect();
                    self.set_values(cells, cx);
                }
                Ok(None) => {}
            }
        }
        self.editing = None;
        // Focus inside the editor goes back to the grid; focus the user moved elsewhere stays.
        if self.focus.contains_focused(window, cx) {
            window.focus(&self.focus, cx);
        }
        cx.notify();
        true
    }

    /// Keys while a list or calendar is open: ↑↓ move through the list, Enter sets, Esc cancels.
    /// Returns whether the key was the editor's.
    pub(super) fn on_picker_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(Editing { editor, .. }) = &mut self.editing else {
            return false;
        };
        if matches!(editor, Editor::Text(_)) {
            return false;
        }
        match (key, editor) {
            ("escape", _) => _ = self.stop_editing(false, window, cx),
            ("enter", _) => _ = self.stop_editing(true, window, cx),
            ("up", Editor::Options { highlighted, .. }) => {
                *highlighted = highlighted.saturating_sub(1)
            }
            (
                "down",
                Editor::Options {
                    highlighted,
                    values,
                    ..
                },
            ) => *highlighted = (*highlighted + 1).min(values.len() - 1),
            // The grid stays still under an open list; the calendar's own keys still reach it.
            (_, Editor::Options { .. }) => {}
            _ => return false,
        }
        cx.notify();
        true
    }
}
