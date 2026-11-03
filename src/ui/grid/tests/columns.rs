//! Paging, the column window, column moves, hidden columns and navigation.

use super::*;

#[test]
fn grouped_puts_commas_every_three_digits() {
    for (n, s) in [
        (0, "0"),
        (999, "999"),
        (1_000, "1,000"),
        (10_000, "10,000"),
        (1_000_000, "1,000,000"),
    ] {
        assert_eq!(super::grouped(n), s);
    }
}

#[test]
fn pages_cover_every_row_once() {
    use super::{Page, page_count};
    let all: std::rc::Rc<dyn super::GridSource> = std::rc::Rc::new(mock::wide_table(1_050));
    assert_eq!(page_count(1_050, 500), 3);
    assert_eq!(page_count(0, 500), 1, "an empty result still has a page");
    let last = Page::new(all.clone(), 2, 500);
    assert_eq!((last.start, last.len), (1_000, 50));
    assert_eq!(last.row(0), all.row(1_000));
    let past = Page::new(all, 9, 500);
    assert_eq!(past.len, 0, "a page past the end is empty, not a panic");
}

#[test]
fn visible_cols_follow_scroll_offset() {
    let offsets = prefix_sums(&[px(100.); 300]);
    assert_eq!(visible_cols(&offsets, px(0.), px(250.), 0), 0..3);
    assert_eq!(
        visible_cols(&offsets, px(100.), px(200.), 0),
        1..3,
        "edge-aligned scroll skips col 0"
    );
    assert_eq!(visible_cols(&offsets, px(150.), px(200.), 2), 0..6);
    assert_eq!(
        visible_cols(&offsets, px(29_900.), px(500.), 2),
        297..300,
        "clamped at the end"
    );
    let hidden = prefix_sums(&[px(100.), px(0.), px(100.), px(100.)]);
    assert_eq!(
        visible_cols(&hidden, px(100.), px(100.), 0),
        2..3,
        "zero-width hidden column at the left edge is skipped"
    );
}

#[test]
fn move_col_drops_before_target() {
    // a=0, b=1, c=2, d=3
    let cases: [(&[usize], usize, usize, &[usize]); 5] = [
        (&[0, 1, 2, 3], 0, 3, &[1, 2, 0, 3]), // drag a before d
        (&[0, 1, 2], 2, 0, &[2, 0, 1]),       // drag c to start
        (&[0, 1, 2], 0, 3, &[1, 2, 0]),       // drag a past the last column
        (&[0, 1, 2, 3], 3, 1, &[0, 3, 1, 2]), // drag d left into the middle
        (&[2, 0, 1], 0, 2, &[0, 2, 1]),       // already permuted order
    ];
    for (start, from, before, expected) in cases {
        let mut order = start.to_vec();
        move_col(&mut order, from, before);
        assert_eq!(order, expected, "{start:?} move {from} before {before}");
    }
}

#[test]
fn widths_hidden_and_sort_travel_with_the_column() {
    // Per source column a..d: widths 10..40, c hidden, sorted on b.
    let (widths, hidden, sorted) = (
        [px(10.), px(20.), px(30.), px(40.)],
        [false, false, true, false],
        1,
    );
    // (from, before) -> (order, display widths, display position of the sorted column, selection cleared)
    let cases: [((usize, usize), [usize; 4], [f32; 4], usize, bool); 3] = [
        ((0, 3), [1, 2, 0, 3], [20., 0., 10., 40.], 0, true),
        ((1, 0), [1, 0, 2, 3], [20., 10., 0., 40.], 0, true),
        ((3, 0), [3, 0, 1, 2], [40., 10., 20., 0.], 2, true),
    ];
    for ((from, before), order, expected, sort_at, cleared) in cases {
        let mut o = vec![0, 1, 2, 3];
        let changed = move_col(&mut o, from, before);
        assert_eq!(o, order, "move {from} before {before}");
        assert_eq!(
            display_widths(&o, &widths, &hidden),
            expected.map(px),
            "widths after move {from} before {before}"
        );
        assert_eq!(
            o.iter().position(|&s| s == sorted),
            Some(sort_at),
            "sort still on b after move {from} before {before}"
        );
        assert_eq!(
            changed, cleared,
            "selection cleared after move {from} before {before}"
        );
    }
}

#[test]
fn drop_on_own_position_keeps_order_and_selection() {
    // (from, before): before itself, before its right neighbour, and the last column at the end.
    for (from, before) in [(0, 0), (0, 1), (2, 2), (2, 3), (3, 3), (3, 4)] {
        let mut order = vec![0, 1, 2, 3];
        let changed = move_col(&mut order, from, before);
        assert_eq!(
            order,
            [0, 1, 2, 3],
            "order after move {from} before {before}"
        );
        assert!(!changed, "selection kept after move {from} before {before}");
    }
}

#[test]
fn ctrl_home_end_skip_hidden_columns() {
    // (key, hidden in display order, from, expected head) over 10 rows
    let cases: [(&str, [bool; 3], (usize, usize), (usize, usize)); 6] = [
        ("end", [false, true, false], (0, 0), (9, 2)), // b hidden -> last row, column c
        ("end", [false, false, true], (0, 0), (9, 1)),
        ("end", [false, false, false], (3, 1), (9, 2)),
        ("home", [true, false, false], (5, 2), (0, 1)),
        ("home", [true, true, false], (5, 2), (0, 2)),
        ("home", [false, false, false], (5, 2), (0, 0)),
    ];
    for (key, hidden, from, expected) in cases {
        assert_eq!(
            nav(key, true, false, from, 10, &hidden, 5),
            Some(expected),
            "Ctrl+{key} from {from:?}, hidden {hidden:?}"
        );
    }
}

#[test]
fn horizontal_keys_skip_hidden_columns() {
    // (key, shift, hidden in display order, from, expected head) over 10 rows
    let cases: [(&str, bool, [bool; 4], (usize, usize), (usize, usize)); 9] = [
        ("right", false, [false, true, false, false], (3, 0), (3, 2)),
        ("right", false, [false, false, true, true], (3, 1), (3, 1)), // nothing visible to the right
        ("left", false, [false, true, false, false], (3, 2), (3, 0)),
        ("left", false, [true, true, false, false], (3, 2), (3, 2)), // nothing visible to the left
        ("tab", false, [false, false, true, false], (3, 1), (3, 3)),
        ("tab", true, [false, false, true, false], (3, 3), (3, 1)),
        ("home", false, [true, false, false, false], (3, 3), (3, 1)),
        ("end", false, [false, false, false, true], (3, 0), (3, 2)),
        ("end", false, [false, true, true, true], (3, 0), (3, 0)),
    ];
    for (key, shift, hidden, from, expected) in cases {
        assert_eq!(
            nav(key, false, shift, from, 10, &hidden, 5),
            Some(expected),
            "{key} shift={shift} from {from:?}, hidden {hidden:?}"
        );
    }
}

#[test]
fn real_move_resets_selection_and_own_position_keeps_it() {
    let selection = || ((2, 1), (4, 3), vec![(0..2, 0..1)]);
    // c hidden after the move [c, a, b] -> first visible display column is 1
    let first_visible = band_cols(None, &[true, false, false]).0;
    assert_eq!(
        selection_after_drop(Some(first_visible), selection()),
        ((0, 1), (0, 1), vec![]),
        "real move"
    );
    assert_eq!(
        selection_after_drop(None, selection()),
        selection(),
        "own-position drop"
    );
}

#[test]
fn select_row_skips_hidden_columns() {
    // (clicked column, hidden in display order, expected (anchor, head) columns)
    let cases: [(Option<usize>, [bool; 3], (usize, usize)); 5] = [
        (None, [false, false, true], (0, 1)), // c hidden -> head on b
        (None, [true, false, false], (1, 2)), // a hidden -> anchor on b
        (None, [true, false, true], (1, 1)),
        (None, [false, false, false], (0, 2)),
        (Some(1), [false, true, false], (1, 1)), // a header click selects that column as is
    ];
    for (col, hidden, expected) in cases {
        assert_eq!(
            band_cols(col, &hidden),
            expected,
            "band {col:?}, hidden {hidden:?}"
        );
    }
}

#[gpui_kit::test]
fn scrollbar_room_only_on_axes_that_overflow(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let room = |cx: &mut gpui_kit::TestAppContext, rows: usize, one_col: bool| {
        let (window, grid) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| super::Grid::new(std::rc::Rc::new(mock::wide_table(rows)), cx))
            })
            .expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        grid.update(&mut cx, |g, cx| {
            if one_col {
                g.hidden.iter_mut().skip(1).for_each(|h| *h = true);
            }
            cx.notify()
        });
        cx.run_until_parked();
        grid.read_with(&cx, |g, _| g.scrollbar_room())
    };
    let (bar, none) = (
        gpui_kit::component::scroll::Scrollbar::width(),
        gpui_kit::Pixels::ZERO,
    );
    assert_eq!(room(cx, 3, true), (none, none), "fits both ways: no room");
    assert_eq!(
        room(cx, 3, false),
        (bar, none),
        "wide only: room at the bottom"
    );
    assert_eq!(
        room(cx, 2000, true),
        (none, bar),
        "tall only: room on the right"
    );
    assert_eq!(room(cx, 2000, false), (bar, bar), "both");
}
