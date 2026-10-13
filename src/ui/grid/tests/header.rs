//! Header model, height, sort availability and tooltips.

use super::*;

#[test]
fn header_model_shows_name_type_and_pk_icon() {
    let m = |n: &str, t: &str, pk| HeaderModel {
        name: n.into(),
        data_type: t.into(),
        pk,
    };
    let cases = [
        (col("id", "bigint", true), m("id", "bigint", true)),
        (col("email", "text", false), m("email", "text", false)),
        (col("", "", false), m("", "", false)),
    ];
    for (c, want) in cases {
        assert_eq!(header_model(&c), want);
    }
    // The icon belongs to the column: a reordered PK keeps it, and so does a hidden one (the model ignores both).
    let cols = [
        col("a", "text", false),
        col("id", "bigint", true),
        col("b", "int", false),
    ];
    let shown: Vec<bool> = [1usize, 0, 2]
        .iter()
        .map(|&s| header_model(&cols[s]).pk)
        .collect();
    assert_eq!(shown, [true, false, false]);
}

#[test]
fn header_height_fits_two_lines() {
    use crate::theme::{badge_line, space_4, space_8};
    assert_eq!(header_height(badge_line(), space_8()), px(40.));
    assert_eq!(header_height(px(16.), px(0.)), px(32.));
    assert_eq!(header_height(px(14.), space_4()), px(32.));
    assert!(header_height(badge_line(), space_8()) >= badge_line() * 2.);
}

#[test]
fn sort_is_available_up_to_client_sort_max() {
    const WHY: &str = "Too many rows to sort here; use ORDER BY";
    let cases = [
        (0, SortAvail::Enabled),
        (1, SortAvail::Enabled),
        (99_999, SortAvail::Enabled),
        (100_000, SortAvail::Enabled),
        (100_001, SortAvail::Disabled(WHY)),
        (1_000_000, SortAvail::Disabled(WHY)),
    ];
    for (rows, want) in cases {
        assert_eq!(sort_availability(rows), want, "{rows} rows");
    }
    assert_eq!(
        sort_availability(super::BigEvents::new().row_count()),
        SortAvail::Disabled(WHY),
        "big_events"
    );
}

#[gpui_kit::test]
fn truncated_header_names_and_types_show_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
    use crate::model::ResultSet;
    use crate::theme::tests::hover_shows_truncation_tip;
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let long = "shipment_line_item_carrier_tracking_events_archive_partitioned_by_month_2026_q3x";
    let col = |name: &str, data_type: &str| ColumnMeta {
        name: name.into(),
        data_type: data_type.into(),
        is_pk: false,
        ..Default::default()
    };
    let source = ResultSet {
        columns: vec![
            col("id", "bigint"),
            col(long, "text"),
            col("created_at", "timestamp with time zone"),
        ],
        rows: vec![
            vec![
                Value::Int(1),
                Value::Text("a".into()),
                Value::Text("2026-10-10".into()),
            ]
            .into(),
        ],
    };
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(source), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    // As if dragged by the user: wide enough for id/bigint, narrow for created_at.
    grid.update(&mut cx, |grid, cx| {
        grid.widths[0] = px(200.);
        grid.widths[2] = px(60.);
        cx.notify();
    });
    cx.run_until_parked();
    // (selector, tooltip shown); ones that fit first, so no earlier tooltip lingers.
    let cases = [
        ("header-name-0", false),
        ("header-type-0", false),
        ("header-type-1", false),
        ("header-name-1", true),
        ("header-type-2", true),
    ];
    for (selector, tip) in cases {
        assert_eq!(
            hover_shows_truncation_tip(&mut cx, selector.into()),
            tip,
            "{selector}"
        );
    }
}

#[gpui_kit::test]
fn opening_widths_fit_short_headers_beside_the_sort_control(cx: &mut gpui_kit::TestAppContext) {
    use crate::model::ResultSet;
    use crate::theme::tests::hover_shows_truncation_tip;
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let col = |name: &str, data_type: &str, is_pk| ColumnMeta {
        name: name.into(),
        data_type: data_type.into(),
        is_pk,
        ..Default::default()
    };
    // One-character cells: only the header can size these columns.
    let source = ResultSet {
        columns: vec![col("id", "bigint", true), col("ok", "boolean", false)],
        rows: vec![vec![Value::Int(1), Value::Bool(true)].into()],
    };
    let (window, _grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(source), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    for selector in [
        "header-name-0",
        "header-type-0",
        "header-name-1",
        "header-type-1",
    ] {
        assert!(
            !hover_shows_truncation_tip(&mut cx, selector.into()),
            "{selector}"
        );
    }
}
