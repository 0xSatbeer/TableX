//! Header model, height, sort availability and tooltips.

use super::*;

#[test]
fn header_model_shows_name_type_and_key_icons() {
    let m = |n: &str, t: &str, pk, fk| HeaderModel {
        name: n.into(),
        data_type: t.into(),
        pk,
        fk,
    };
    let cases = [
        (col("id", "bigint", true), m("id", "bigint", true, false)),
        (
            col("email", "text", false),
            m("email", "text", false, false),
        ),
        (col("", "", false), m("", "", false, false)),
    ];
    for (c, want) in cases {
        assert_eq!(header_model(&c, &[], &[]), want);
    }
    // A table tab names its keys: a key column read from the table gets its icon, FK too.
    let (key, fks) = (
        ["id".to_string()],
        ["user_id".to_string(), "id".to_string()],
    );
    let flags = |c: ColumnMeta| {
        let m = header_model(&c, &key, &fks);
        (m.pk, m.fk)
    };
    assert_eq!(flags(col("id", "bigint", false)), (true, true));
    assert_eq!(flags(col("user_id", "bigint", false)), (false, true));
    assert_eq!(flags(col("email", "text", false)), (false, false));
    // The icon belongs to the column: a reordered PK keeps it, and so does a hidden one (the model ignores both).
    let cols = [
        col("a", "text", false),
        col("id", "bigint", true),
        col("b", "int", false),
    ];
    let shown: Vec<bool> = [1usize, 0, 2]
        .iter()
        .map(|&s| header_model(&cols[s], &[], &[]).pk)
        .collect();
    assert_eq!(shown, [true, false, false]);
}

#[test]
fn type_icon_follows_the_column_type() {
    use gpui_kit::assets::IconName as Lucide;
    let cases = [
        ("integer", Lucide::Hash),
        ("INTEGER", Lucide::Hash),
        ("numeric(10,2)", Lucide::Hash),
        ("bigserial", Lucide::Hash),
        ("interval", Lucide::Clock),
        ("text", Lucide::Type),
        ("character varying(255)", Lucide::Type),
        ("boolean", Lucide::ToggleLeft),
        ("timestamp with time zone", Lucide::CalendarClock),
        ("timestamptz", Lucide::CalendarClock),
        ("DATETIME", Lucide::CalendarClock),
        ("date", Lucide::Calendar),
        ("time without time zone", Lucide::Clock),
        ("jsonb", Lucide::Braces),
        ("bytea", Lucide::Binary),
        ("BLOB", Lucide::Binary),
        ("uuid", Lucide::FingerprintPattern),
        ("inet", Lucide::Network),
        ("integer[]", Lucide::Brackets),
        ("_int4", Lucide::Brackets),
        ("", Lucide::Type),
    ];
    for (ty, want) in cases {
        assert_eq!(type_icon(&col("c", ty, false)), want, "{ty}");
    }
    let mut kind = col("event", "event_kind", false);
    kind.enum_values = [std::sync::Arc::from("signup")].into();
    assert_eq!(type_icon(&kind), Lucide::List);
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
        assert_eq!(sort_availability(rows, "text"), want, "{rows} rows");
    }
    for ty in ["json", "jsonb", "JSONB"] {
        assert_eq!(
            sort_availability(1, ty),
            SortAvail::Disabled("JSON columns can't be sorted"),
            "{ty}"
        );
    }
    assert_eq!(
        sort_availability(super::BigEvents::new().row_count(), "bigint"),
        SortAvail::Disabled(WHY),
        "big_events"
    );
}

#[gpui_kit::test]
fn truncated_header_names_show_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
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
        ("header-name-1", true),
        ("header-name-2", true),
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
    // The type moved out of the header into the hover tooltip.
    for selector in ["header-name-0", "header-tip-1"] {
        hover_shows_truncation_tip(&mut cx, selector.into());
        assert!(cx.debug_bounds("type-tip").is_some(), "{selector}");
    }
    for selector in ["header-name-0", "header-name-1"] {
        assert!(
            !hover_shows_truncation_tip(&mut cx, selector.into()),
            "{selector}"
        );
    }
}
