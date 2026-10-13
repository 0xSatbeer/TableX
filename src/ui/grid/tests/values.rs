//! Reading typed and pasted text as values, and picking each column's editor.

use super::*;

#[test]
fn parse_tsv_reads_back_what_tsv_copies() {
    let cols = [ColumnMeta {
        name: "a".into(),
        data_type: "text".into(),
        is_pk: false,
        ..Default::default()
    }];
    let rows = vec![
        vec![Value::Text("x\ty".into()), Value::Null],
        vec![Value::Text("say \"hi\"\nbye".into()), Value::Int(3)],
    ];
    let parsed = super::parse_tsv(&tsv(&cols, &rows));
    assert_eq!(parsed, [vec!["x\ty", ""], vec!["say \"hi\"\nbye", "3"]]);
    assert_eq!(
        super::parse_tsv("1\t2\r\n3\t4"),
        [vec!["1", "2"], vec!["3", "4"]],
        "CRLF and no trailing newline"
    );
    assert_eq!(super::parse_tsv("v\n"), [vec!["v"]]);
}

#[test]
fn parse_value_follows_the_column_type() {
    let col = |ty: &str| ColumnMeta {
        name: "c".into(),
        data_type: ty.into(),
        ..Default::default()
    };
    let ok = |text: &str, ty: &str| {
        super::parse_value(text, &col(ty)).unwrap_or_else(|e| panic!("{text:?} as {ty}: {e}"))
    };
    let err = |text: &str, ty: &str| {
        super::parse_value(text, &col(ty))
            .expect_err(&format!("{text:?} as {ty} should be refused"))
    };
    assert_eq!(
        ok("", "integer"),
        Value::Null,
        "empty is NULL in any column"
    );
    assert_eq!(ok("42", "integer"), Value::Int(42));
    assert_eq!(ok("4.5", "numeric(10,2)"), Value::Float(4.5));
    assert_eq!(ok("TRUE", "boolean"), Value::Bool(true));
    assert_eq!(ok("0", "boolean"), Value::Bool(false));
    assert_eq!(
        ok("{\"a\": [1, 2]}", "jsonb"),
        Value::Json("{\"a\": [1, 2]}".into())
    );
    assert_eq!(ok("42", "text"), Value::Text("42".into()));
    assert_eq!(ok("2026-10-12", "date"), Value::Text("2026-10-12".into()));
    assert_eq!(
        ok("\\x0aFF", "bytea"),
        Value::Bytes([0x0a, 0xff][..].into())
    );
    assert_eq!(
        ok("123E4567-e89b-12d3-a456-426614174000", "uuid"),
        Value::Text("123e4567-e89b-12d3-a456-426614174000".into())
    );
    assert_eq!(ok("32767", "smallint"), Value::Int(32_767));
    for (text, ty, why) in [
        ("abc", "bigint", "Not a whole number"),
        ("4.5", "integer", "Not a whole number"),
        ("32768", "smallint", "Too big for smallint"),
        ("2147483648", "integer", "Too big for integer"),
        ("12,5", "double precision", "Not a number"),
        ("maybe", "boolean", "Use true or false"),
        ("{\"a\": }", "jsonb", "Not valid JSON"),
        ("{'a': 1}", "json", "Not valid JSON"),
        ("soon", "timestamptz", "Not a date and time"),
        ("2026-02-30", "date", "Not a date"),
        ("123e4567e89b12d3a456", "uuid", "Not a UUID"),
        ("123e4567-e89b12d3-a456-426614174000-", "uuid", "Not a UUID"),
        ("0aff", "bytea", "Use hex bytes"),
        ("\\x0af", "bytea", "two hex digits"),
    ] {
        let got = err(text, ty);
        assert!(got.contains(why), "{text:?} as {ty}: {got:?}, want {why:?}");
    }
    let status = ColumnMeta {
        enum_values: ["open", "paid"].map(std::sync::Arc::from).into(),
        ..col("order_status")
    };
    assert_eq!(
        super::parse_value("paid", &status),
        Ok(Value::Text("paid".into()))
    );
    assert_eq!(
        super::parse_value("Paid", &status),
        Err("Not one of: open, paid".into())
    );
}

#[test]
fn paste_fills_one_value_and_lays_out_a_block() {
    use super::paste_targets;
    let block = |rows: &[&[&str]]| {
        rows.iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect::<Vec<Vec<String>>>()
    };
    let one = paste_targets(
        &block(&[&["x"]]),
        &[(0..2, 0..2), (1..2, 1..3)],
        (1, 1),
        10,
        &[false, false, true],
    );
    let cells: Vec<_> = one.iter().map(|(c, _)| *c).collect();
    assert_eq!(
        cells,
        [(0, 0), (0, 1), (1, 0), (1, 1)],
        "every selected visible cell once, hidden column 2 skipped"
    );
    let grid = paste_targets(
        &block(&[&["a", "b", "c"], &["d", "e", "f"], &["g", "h", "i"]]),
        &[(1..2, 0..1)],
        (1, 0),
        3,
        &[false, true, false, false],
    );
    let got: Vec<_> = grid
        .iter()
        .map(|((r, c), v)| (*r, *c, v.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (1, 0, "a"),
            (1, 2, "b"),
            (1, 3, "c"),
            (2, 0, "d"),
            (2, 2, "e"),
            (2, 3, "f")
        ],
        "skips hidden column 1, clipped at the last row"
    );
}

#[test]
fn each_type_gets_its_editor() {
    use super::{EditorKind, editor_kind};
    let meta = |ty: &str| ColumnMeta {
        name: "c".into(),
        data_type: ty.into(),
        ..Default::default()
    };
    let cases = [
        (
            "boolean",
            EditorKind::Options(vec!["true".into(), "false".into()]),
        ),
        ("date", EditorKind::Date),
        ("timestamp", EditorKind::DateTime { zoned: false }),
        (
            "timestamp without time zone",
            EditorKind::DateTime { zoned: false },
        ),
        ("timestamptz", EditorKind::DateTime { zoned: true }),
        (
            "timestamp with time zone",
            EditorKind::DateTime { zoned: true },
        ),
        ("timestamptz[]", EditorKind::Text),
        ("text", EditorKind::Text),
        ("integer", EditorKind::Text),
    ];
    for (ty, kind) in cases {
        assert_eq!(editor_kind(&meta(ty)), kind, "{ty}");
    }
    let status = ColumnMeta {
        enum_values: ["open", "paid"].map(std::sync::Arc::from).into(),
        ..meta("order_status")
    };
    assert_eq!(
        editor_kind(&status),
        EditorKind::Options(vec!["open".into(), "paid".into()])
    );
}

#[test]
fn timestamps_read_back_what_the_grid_shows() {
    use super::{parse_value, timestamp_value, wall_time};
    let values = [
        Value::Timestamp(1_760_000_000_123_456),
        Value::TimestampTz {
            micros: 1_760_000_000_000_000,
            offset_secs: 19_800,
        },
        Value::TimestampTz {
            micros: 1_760_000_000_000_000,
            offset_secs: -3_600,
        },
    ];
    for v in values {
        let ty = if matches!(v, Value::Timestamp(_)) {
            "timestamp"
        } else {
            "timestamptz"
        };
        let column = ColumnMeta {
            data_type: ty.into(),
            ..Default::default()
        };
        assert_eq!(parse_value(&v.to_string(), &column), Ok(v.clone()), "{v}");
        let (at, offset) = wall_time(&v).unwrap();
        assert_eq!(
            timestamp_value(at, (ty == "timestamptz").then_some(offset)),
            v,
            "wall time round trip of {v}"
        );
    }
    let at = |text: &str, ty: &str| {
        parse_value(
            text,
            &ColumnMeta {
                data_type: ty.into(),
                ..Default::default()
            },
        )
        .unwrap()
        .to_string()
    };
    assert_eq!(
        at("2026-10-12", "timestamp"),
        "2026-10-12 00:00:00",
        "a bare date is midnight"
    );
    assert_eq!(
        at("2026-10-12 09:30", "timestamptz"),
        "2026-10-12 09:30:00+00",
        "no offset reads as UTC"
    );
}
