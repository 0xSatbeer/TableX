//! Lists and calendars in the cell editor, and refusing invalid values.

use super::*;

#[gpui_kit::test]
fn lists_and_calendars_edit_booleans_enums_and_timestamps(cx: &mut gpui_kit::TestAppContext) {
    use super::Editor;
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(super::BigEvents::new()), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let cell = |cx: &mut gpui_kit::VisualTestContext, r: usize, c: usize| {
        grid.read_with(cx, |g, _| g.row(r)[c].clone())
    };
    let open_at = |cx: &mut gpui_kit::VisualTestContext, cell: (usize, usize)| {
        grid.update_in(cx, |g, window, cx| {
            window.focus(&g.focus, cx);
            (g.anchor, g.head) = (cell, cell);
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
    };
    let (ok, event, occurred, logged) = (5, 3, 1, 11);

    // Boolean: a true/false list; ↓ then Enter flips it.
    let was = cell(&mut cx, 0, ok);
    open_at(&mut cx, (0, ok));
    assert!(
        matches!(
            grid.read_with(&cx, |g, _| g
                .editing
                .as_ref()
                .map(|e| matches!(e.editor, Editor::Options { .. }))),
            Some(true)
        ),
        "booleans open a list"
    );
    assert!(
        cx.debug_bounds("cell-option-1").is_some(),
        "the list is drawn"
    );
    let key = if was == Value::Bool(true) {
        "down"
    } else {
        "up"
    };
    cx.simulate_keystrokes(&format!("{key} enter"));
    assert_eq!(
        cell(&mut cx, 0, ok),
        Value::Bool(was != Value::Bool(true)),
        "↑↓ Enter picks the other value"
    );
    assert_eq!(
        grid.read_with(&cx, |g, _| g.head),
        (0, ok),
        "the arrows moved the list, not the grid"
    );

    // Enum: clicking a label sets it, on every selected cell.
    grid.update(&mut cx, |g, _| {
        (g.anchor, g.head) = ((0, event), (2, event))
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let purchase = cx
        .debug_bounds("cell-option-3")
        .expect("enum labels listed")
        .center();
    cx.simulate_click(purchase, Default::default());
    for r in 0..3 {
        assert_eq!(
            cell(&mut cx, r, event),
            Value::Text("purchase".into()),
            "row {r}"
        );
    }

    // Esc closes a list without writing.
    open_at(&mut cx, (5, event));
    let before = cell(&mut cx, 5, event);
    cx.simulate_keystrokes("down escape");
    assert!(grid.read_with(&cx, |g, _| g.editing.is_none()));
    assert_eq!(cell(&mut cx, 5, event), before);

    // Timestamps: a calendar opened on the cell's day; Enter keeps the exact value, sub-seconds included.
    for col in [occurred, logged] {
        let was = cell(&mut cx, 4, col);
        open_at(&mut cx, (4, col));
        let day = grid.read_with(&cx, |g, cx| match &g.editing.as_ref().unwrap().editor {
            Editor::Date { calendar, time, .. } => {
                assert!(time.is_some(), "timestamps also edit the time");
                calendar.read(cx).date().start()
            }
            _ => panic!("timestamps open a calendar"),
        });
        assert_eq!(day, super::wall_time(&was).map(|(at, _)| at.date()));
        cx.simulate_keystrokes("enter");
        assert_eq!(cell(&mut cx, 4, col), was, "column {col}");
    }

    // A changed time is written in the cell's own zone.
    open_at(&mut cx, (6, occurred));
    grid.update_in(&mut cx, |g, window, cx| {
        let Editor::Date {
            time: Some((field, _)),
            ..
        } = &g.editing.as_ref().unwrap().editor
        else {
            unreachable!()
        };
        field.update(cx, |f, cx| {
            f.set_time(
                chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
                window,
                cx,
            )
        });
    });
    cx.simulate_keystrokes("enter");
    let written = cell(&mut cx, 6, occurred);
    assert!(
        matches!(written, Value::TimestampTz { offset_secs: 0, .. })
            && written.to_string().ends_with(" 09:30:00+00"),
        "{written}"
    );
}

#[gpui_kit::test]
fn invalid_values_are_never_saved(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(super::BigEvents::new()), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let cell = |cx: &mut gpui_kit::VisualTestContext, r: usize, c: usize| {
        grid.read_with(cx, |g, _| g.row(r)[c].clone())
    };
    let error = |cx: &mut gpui_kit::VisualTestContext| {
        grid.read_with(cx, |g, _| g.editing.as_ref().and_then(|e| e.error.clone()))
    };
    let (amount, payload) = (4, 6);
    let type_in = |cx: &mut gpui_kit::VisualTestContext, text: &str| {
        grid.update_in(cx, |g, window, cx| {
            text_input(g).update(cx, |i, cx| {
                i.set_value(text.to_string(), window, cx);
                // A textarea's set_value leaves the cursor at the start.
                i.set_cursor_position(
                    gpui_kit::component::input::Position::new(0, u32::MAX),
                    window,
                    cx,
                )
            })
        })
    };
    grid.update_in(&mut cx, |g, window, cx| {
        window.focus(&g.focus, cx);
        (g.anchor, g.head) = ((2, payload), (2, payload));
    });
    let was = cell(&mut cx, 2, payload);

    cx.simulate_keystrokes("enter");
    type_in(&mut cx, "{\"v\": ");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        error(&mut cx).is_some_and(|e| e.starts_with("Not valid JSON")),
        "Enter on broken JSON says why: {:?}",
        error(&mut cx)
    );
    assert!(
        cx.debug_bounds("cell-problem").is_some(),
        "the reason shows under the cell"
    );
    assert_eq!(cell(&mut cx, 2, payload), was, "and nothing is saved");
    let elsewhere = grid.read_with(&cx, |g, _| g.gutter() + g.offsets()[0] + px(5.));
    let body = cx.debug_bounds("grid-body").unwrap().origin;
    cx.simulate_click(
        gpui_kit::point(
            body.x + elsewhere,
            body.y + grid.read_with(&cx, |g, _| g.row_h()) * 8.5,
        ),
        Default::default(),
    );
    assert!(
        grid.read_with(&cx, |g, _| g.editing.is_some() && g.head == (2, payload)),
        "clicking another cell can't leave it either"
    );
    cx.simulate_input("1}");
    assert_eq!(error(&mut cx), None, "typing the fix clears the reason");
    cx.simulate_keystrokes("enter");
    assert_eq!(cell(&mut cx, 2, payload), Value::Json("{\"v\": 1}".into()));

    // Esc always leaves, dropping the bad text.
    grid.update(&mut cx, |g, _| {
        (g.anchor, g.head) = ((3, amount), (3, amount))
    });
    let was = cell(&mut cx, 3, amount);
    cx.simulate_keystrokes("enter");
    type_in(&mut cx, "12,5");
    cx.simulate_keystrokes("enter");
    assert!(error(&mut cx).is_some());
    cx.simulate_keystrokes("escape");
    assert!(grid.read_with(&cx, |g, _| g.editing.is_none()));
    assert_eq!(cell(&mut cx, 3, amount), was);

    // A paste with one bad field writes nothing and points at that field.
    let before: Vec<_> = (0..2).map(|r| cell(&mut cx, r, amount)).collect();
    grid.update(&mut cx, |g, _| {
        (g.anchor, g.head) = ((0, amount), (0, amount))
    });
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("1.5\nlots\n".into()));
    cx.simulate_keystrokes("ctrl-v");
    assert_eq!(
        (0..2).map(|r| cell(&mut cx, r, amount)).collect::<Vec<_>>(),
        before,
        "nothing pasted"
    );
    assert_eq!(
        grid.read_with(&cx, |g, _| (g.head, g.notice.clone())),
        (
            (1, amount),
            Some(((1, amount), "Nothing pasted. Not a number".into()))
        )
    );
}
