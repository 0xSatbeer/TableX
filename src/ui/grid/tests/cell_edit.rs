//! Pasting, setting values, drag-selecting and double-click editing in a live grid.

use super::*;

#[gpui_kit::test]
fn paste_and_set_value_edit_the_selected_cells(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(mock::wide_table(20)), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let cell = |cx: &mut gpui_kit::VisualTestContext, r: usize, c: usize| {
        grid.read_with(cx, |g, _| g.row(r)[c].clone())
    };
    grid.update_in(&mut cx, |g, window, cx| {
        window.focus(&g.focus, cx);
        (g.anchor, g.head) = ((0, 0), (1, 1));
    });
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("7".into()));
    cx.simulate_keystrokes("ctrl-v");
    assert_eq!(
        [cell(&mut cx, 0, 0), cell(&mut cx, 1, 1)],
        [Value::Int(7), Value::Int(7)],
        "one value fills the selection"
    );
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("1\t2\n3\t4\n".into()));
    grid.update(&mut cx, |g, _| (g.anchor, g.head) = ((5, 2), (5, 2)));
    cx.simulate_keystrokes("ctrl-v");
    assert_eq!(
        [cell(&mut cx, 5, 2), cell(&mut cx, 6, 3)],
        [Value::Int(1), Value::Int(4)],
        "a block pastes from the top-left"
    );
    assert_eq!(
        grid.read_with(&cx, |g, _| (g.anchor, g.head)),
        ((5, 2), (6, 3)),
        "and selects what it wrote"
    );

    cx.simulate_keystrokes("enter");
    cx.simulate_input("99");
    cx.simulate_keystrokes("escape");
    assert!(
        grid.read_with(&cx, |g, _| g.editing.is_none()),
        "Esc closes the input"
    );
    assert_eq!(cell(&mut cx, 5, 2), Value::Int(1), "Esc applies nothing");
    cx.simulate_keystrokes("enter");
    grid.update_in(&mut cx, |g, window, cx| {
        text_input(g).update(cx, |i, cx| i.set_value("", window, cx))
    });
    cx.simulate_keystrokes("enter");
    assert!(
        grid.read_with(&cx, |g, _| g.editing.is_none()),
        "Enter applies and closes"
    );
    assert_eq!(
        [
            cell(&mut cx, 5, 2),
            cell(&mut cx, 5, 3),
            cell(&mut cx, 6, 2),
            cell(&mut cx, 6, 3)
        ],
        [Value::Null, Value::Null, Value::Null, Value::Null],
        "empty sets every selected cell NULL"
    );
}

#[gpui_kit::test]
fn dragging_across_cells_selects_the_rectangle(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(mock::wide_table(20)), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    let (gutter, row_h, offsets) = grid.read_with(&cx, |g, _| (g.gutter(), g.row_h(), g.offsets()));
    let body = cx
        .debug_bounds("grid-body")
        .expect("grid body drawn")
        .origin;
    let at = |r: usize, c: usize| {
        gpui_kit::point(
            body.x + gutter + offsets[c] + px(5.),
            body.y + row_h * (r as f32 + 0.5),
        )
    };
    cx.simulate_mouse_down(at(1, 0), gpui_kit::MouseButton::Left, Default::default());
    cx.simulate_mouse_move(
        at(3, 2),
        Some(gpui_kit::MouseButton::Left),
        Default::default(),
    );
    cx.simulate_mouse_up(at(3, 2), gpui_kit::MouseButton::Left, Default::default());
    assert_eq!(
        grid.read_with(&cx, |g, _| g.selection()),
        (1..4, 0..3),
        "anchor at the press, head at the release"
    );
    cx.simulate_mouse_move(at(5, 4), None, Default::default());
    assert_eq!(
        grid.read_with(&cx, |g, _| g.selection()),
        (1..4, 0..3),
        "moving with no button pressed changes nothing"
    );
}

#[gpui_kit::test]
fn double_click_edits_only_the_clicked_cell(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(mock::wide_table(20)), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    let (gutter, row_h, offsets) = grid.read_with(&cx, |g, _| (g.gutter(), g.row_h(), g.offsets()));
    let body = cx
        .debug_bounds("grid-body")
        .expect("grid body drawn")
        .origin;
    let at = |r: usize, c: usize| {
        gpui_kit::point(
            body.x + gutter + offsets[c] + px(5.),
            body.y + row_h * (r as f32 + 0.5),
        )
    };
    let double_click = |cx: &mut gpui_kit::VisualTestContext, r, c| {
        for click_count in [1, 2] {
            cx.simulate_event(gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position: at(r, c),
                modifiers: Default::default(),
                click_count,
                first_mouse: false,
            });
            cx.simulate_event(gpui_kit::MouseUpEvent {
                button: gpui_kit::MouseButton::Left,
                position: at(r, c),
                modifiers: Default::default(),
                click_count,
            });
        }
    };
    let cell = |cx: &mut gpui_kit::VisualTestContext, r: usize, c: usize| {
        grid.read_with(cx, |g, _| g.row(r)[c].clone())
    };
    let original = cell(&mut cx, 3, 3);

    grid.update(&mut cx, |g, _| (g.anchor, g.head) = ((0, 0), (1, 1)));
    double_click(&mut cx, 1, 0);
    assert_eq!(
        grid.read_with(&cx, |g, _| (
            g.selection(),
            g.editing.as_ref().map(|e| e.cell)
        )),
        ((1..2, 0..1), Some((1, 0))),
        "double-click inside a selection collapses it to the clicked cell"
    );
    grid.update_in(&mut cx, |g, window, cx| {
        text_input(g).update(cx, |i, cx| i.set_value("5", window, cx))
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(
        [0, 1].map(|r| [0, 1].map(|c| cell(&mut cx, r, c) == Value::Int(5))),
        [[false, false], [true, false]],
        "the typed value goes only to the edited cell"
    );

    double_click(&mut cx, 3, 3);
    cx.simulate_input("8");
    assert_eq!(
        cell(&mut cx, 3, 3),
        original,
        "nothing is written while typing"
    );
    cx.simulate_click(at(6, 0), Default::default());
    cx.run_until_parked();
    assert!(
        grid.read_with(&cx, |g, _| g.editing.is_none()),
        "clicking another cell closes the editor"
    );
    assert_eq!(
        cell(&mut cx, 3, 3),
        Value::Int(format!("{original}8").parse().unwrap()),
        "and keeps what was typed, only in the edited cell"
    );
    assert_eq!(
        cell(&mut cx, 6, 0),
        grid.read_with(&cx, |g, _| g.source.row(6)[0].clone()),
        "the clicked cell is untouched"
    );
}

#[gpui_kit::test]
fn setting_a_cell_back_to_its_loaded_value_drops_the_edit(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(mock::wide_table(20)), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    grid.update(&mut cx, |g, cx| {
        let was = g.row(0)[0].clone();
        g.set_values(vec![((0, 0), Value::Int(-1))], cx);
        assert_eq!(g.edits.len(), 1, "a changed value is an edit");
        g.set_values(vec![((0, 0), was)], cx);
        assert!(g.edits.is_empty(), "the loaded value again is no edit");
        // A new row's NULL is typed, not the column default its blank cell stands for.
        g.add_row(cx);
        g.set_values(vec![((0, 0), Value::Null)], cx);
        assert_eq!(g.edits.len(), 1, "a new row keeps it");
    });
}

#[test]
fn unchanged_matches_numeric_text_to_its_float() {
    let t = |s: &str| Value::Text(s.into());
    assert!(super::super::paste::unchanged(
        &t("12.50"),
        &Value::Float(12.5)
    ));
    assert!(super::super::paste::unchanged(
        &Value::Int(3),
        &Value::Int(3)
    ));
    assert!(!super::super::paste::unchanged(
        &t("12.51"),
        &Value::Float(12.5)
    ));
    assert!(!super::super::paste::unchanged(
        &t("abc"),
        &Value::Float(0.)
    ));
    assert!(!super::super::paste::unchanged(&t(""), &Value::Null));
}
