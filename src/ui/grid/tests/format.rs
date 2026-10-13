//! Cell text, alignment, gutter and row sizes, sort order and the inspector.

use super::*;

#[test]
fn bools_format_as_words() {
    let cases = [
        (true, "true", CellKind::Bool(true)),
        (false, "false", CellKind::Bool(false)),
    ];
    for (b, text, kind) in cases {
        assert_eq!(format_cell(&Value::Bool(b)), (text.into(), kind));
    }
}

#[test]
fn null_aligns_with_its_column() {
    let cases = [
        ("bigint", true),
        ("integer", true),
        ("int4", true),
        ("smallint", true),
        ("numeric(12,2)", true),
        ("double precision", true),
        ("real", true),
        ("text", false),
        ("timestamptz", false),
        ("boolean", false),
        ("", false),
        ("interval", false),
        ("point", false),
        ("INTEGER", true),
        ("integer[]", false),
        ("numeric[]", false),
        ("int4range", false),
    ];
    for (ty, right) in cases {
        assert_eq!(is_numeric_type(ty), right, "type {ty:?}");
    }
}

#[test]
fn gutter_fits_the_row_count() {
    let cases = [
        (0, 3),
        (9, 3),
        (999, 3),
        (1000, 4),
        (99_999, 5),
        (100_000, 6),
        (999_999, 6),
        (1_000_000, 7),
    ];
    for (rows, digits) in cases {
        assert_eq!(gutter_digits(rows), digits, "{rows} rows");
    }
}

#[test]
fn aligns_right_by_kind_and_type() {
    let cases = [
        (CellKind::Null, "integer", true),
        (CellKind::Null, "INTEGER", true),
        (CellKind::Null, "text", false),
        (CellKind::Null, "integer[]", false),
        (CellKind::Number, "integer", true),
        (CellKind::Number, "text", true),
        (CellKind::Text, "integer", false),
        (CellKind::Bool(true), "integer", false),
    ];
    for (kind, ty, right) in cases {
        assert_eq!(aligns_right(&kind, ty), right, "{kind:?} in {ty:?}");
    }
}

#[test]
fn row_height_scales_with_the_zoomed_font() {
    let cases = [
        (12.5, 26.),
        (13.5, 28.08),
        (11.5, 23.92),
        (9., 18.72),
        (24., 49.92),
    ];
    for (mono, want) in cases {
        let got: f32 = row_height(px(mono)).into();
        assert!(
            (got - want).abs() < 0.01,
            "mono {mono}: row {got}, want {want}"
        );
    }
}

#[gpui_kit::test]
fn record_header_row_follows_the_zoom(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(super::BigEvents::new()), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    grid.update(&mut cx, |grid, cx| {
        grid.record = true;
        grid.zoom(crate::theme::Zoom::In, cx);
    });
    cx.run_until_parked();
    let header: f32 = cx
        .debug_bounds("record-header")
        .expect("record view renders its header")
        .size
        .height
        .into();
    let row: f32 = grid.read_with(&cx, |grid, _| grid.row_h()).into();
    // Layout snaps to whole pixels.
    assert!(
        (header - row).abs() < 0.5,
        "header row {header}px, field rows {row}px at mono 13.5"
    );
}

#[test]
fn gutter_width_is_digits_plus_padding() {
    // Spec rule: digits * one mono digit width (0.6 em) + SPACE_12.
    let cases = [(9, 3.0 * 7.5 + 12.0), (1_000_000, 7.0 * 7.5 + 12.0)];
    for (rows, px_width) in cases {
        let got: f32 = gutter_width(rows, crate::theme::MONO).into();
        assert!((got - px_width).abs() < 0.01, "{rows} rows: {got}");
    }
}

#[test]
fn cells_format_by_type() {
    assert_eq!(format_cell(&Value::Null), ("NULL".into(), CellKind::Null));
    assert_eq!(
        format_cell(&Value::Int(-5)),
        ("-5".into(), CellKind::Number)
    );
    assert_eq!(format_cell(&Value::Text("a\nb".into())).0, "a ↵");
    assert_eq!(
        format_cell(&Value::Bytes(std::sync::Arc::from(&[0u8; 5][..]))).0,
        "0x00000000… (5 B)"
    );
    let long = format_cell(&Value::Text("é".repeat(500).into())).0;
    assert_eq!(long.chars().count(), MAX_CHARS + 1);
}

#[test]
fn sort_order_is_typed_with_nulls_last() {
    let mut v = vec![
        Value::Null,
        Value::Float(2.5),
        Value::Int(10),
        Value::Int(-1),
        Value::Text("a".into()),
    ];
    v.sort_by(cmp_values);
    assert_eq!(
        v,
        [
            Value::Int(-1),
            Value::Float(2.5),
            Value::Int(10),
            Value::Text("a".into()),
            Value::Null
        ]
    );
}

#[test]
fn inspector_pretty_prints_json_and_dumps_bytes() {
    assert_eq!(
        pretty_json(r#"{"a": [1,2], "s": "x,{\"y\"}"}"#),
        "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"s\": \"x,{\\\"y\\\"}\"\n}"
    );
    let bytes = Value::Bytes(std::sync::Arc::from(&[0xABu8; 17][..]));
    assert_eq!(inspect_text(&bytes).lines().nth(1), Some("00000010  ab"));
}
