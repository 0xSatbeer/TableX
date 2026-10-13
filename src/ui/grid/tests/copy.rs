//! Copy formats and multi-range copies.

use super::*;

#[test]
fn copy_skips_hidden_columns() {
    // (hidden, selected display positions, TSV)
    let cases: [([bool; 3], std::ops::Range<usize>, &str); 5] = [
        ([false, true, false], 0..3, "a0\tc0\n"), // select row 0 with b hidden
        ([false, false, false], 0..3, "a0\tb0\tc0\n"),
        ([true, false, true], 0..3, "b0\n"),
        ([false, true, false], 1..3, "c0\n"),
        ([false, true, false], 1..2, "\n"), // only the hidden column selected
    ];
    for (hidden, display, expected) in cases {
        assert_eq!(
            copy(tsv, &[0, 1, 2], &hidden, display.clone()),
            expected,
            "hidden {hidden:?}, selected {display:?}"
        );
    }
    assert_eq!(
        copy(csv, &[0, 1, 2], &[false, true, false], 0..3),
        "a,c\na0,c0\n",
        "every format skips hidden"
    );
}

#[test]
fn copy_follows_display_order() {
    // (display order over a, b, c, hidden per source, selected display positions, CSV)
    let cases: [([usize; 3], [bool; 3], std::ops::Range<usize>, &str); 4] = [
        ([2, 0, 1], [false, true, false], 0..3, "c,a\nc0,a0\n"), // display order [c,a]
        ([2, 0, 1], [false, false, false], 0..2, "c,a\nc0,a0\n"),
        ([2, 1, 0], [false, false, false], 0..3, "c,b,a\nc0,b0,a0\n"),
        ([1, 2, 0], [true, false, false], 1..3, "c\nc0\n"), // hidden looked up by source, not position
    ];
    for (order, hidden, display, expected) in cases {
        assert_eq!(
            copy(csv, &order, &hidden, display.clone()),
            expected,
            "order {order:?}, hidden {hidden:?}, selected {display:?}"
        );
    }
}

#[test]
fn copy_formats_escape_their_syntax() {
    let cols = [
        ColumnMeta {
            name: "a".into(),
            data_type: "text".into(),
            is_pk: false,
            ..Default::default()
        },
        ColumnMeta {
            name: "b".into(),
            data_type: "int".into(),
            is_pk: false,
            ..Default::default()
        },
    ];
    let rows = vec![
        vec![Value::Text("it's, \"x\"|".into()), Value::Null],
        vec![Value::Text("y".into()), Value::Int(2)],
    ];
    assert_eq!(csv(&cols, &rows), "a,b\n\"it's, \"\"x\"\"|\",\ny,2\n");
    assert_eq!(
        json(&cols, &rows),
        "[\n  {\"a\": \"it's, \\\"x\\\"|\", \"b\": null},\n  {\"a\": \"y\", \"b\": 2}\n]\n"
    );
    assert_eq!(
        markdown(&cols, &rows).lines().nth(2),
        Some("| it's, \"x\"\\| | NULL |")
    );
    assert_eq!(
        sql_insert(&cols, &rows).lines().next(),
        Some("INSERT INTO table_name (a, b) VALUES ('it''s, \"x\"|', NULL);")
    );
    assert_eq!(sql_in_list(&cols, &rows), "('it''s, \"x\"|', 'y')");
}

#[test]
fn tsv_quotes_and_blanks_nulls() {
    let rs = mock::edge_cases();
    // Rows 0 (null) and 3 (multiline), both columns.
    let rows = |r: std::ops::Range<usize>| {
        rs.rows[r]
            .iter()
            .map(|row| row[0..2].to_vec())
            .collect::<Vec<_>>()
    };
    assert_eq!(tsv(&rs.columns, &rows(0..1)), "null\t\n");
    assert_eq!(
        tsv(&rs.columns, &rows(3..4)),
        "multiline\t\"first line\nsecond line\n\tindented third\"\n"
    );
}

#[test]
fn multi_range_blocks_are_separated_by_a_blank_line() {
    let (cols, row) = vgrid();
    // (ranges as (rows, display cols), TSV)
    let cases: [(Vec<Block>, &str); 4] = [
        (vec![(0..1, 0..1), (2..3, 1..2)], "v00\n\nv21\n"),
        (vec![(0..1, 0..1)], "v00\n"),
        (
            vec![(0..2, 0..2), (2..3, 2..3)],
            "v00\tv01\nv10\tv11\n\nv22\n",
        ), // different widths
        (vec![(0..1, 0..2), (0..1, 1..3)], "v00\tv01\n\nv01\tv02\n"), // overlapping
    ];
    for (ranges, expected) in cases {
        assert_eq!(
            copy_ranges(tsv, &ranges, &[0, 1, 2], &[false; 3], &cols, &row),
            expected,
            "ranges {ranges:?}"
        );
    }
}

#[test]
fn ranges_are_emitted_in_the_order_added() {
    let (cols, row) = vgrid();
    let (r1, r2): (Block, Block) = ((0..1, 0..1), (2..3, 1..2));
    // Ctrl+click adds: R2 first (kept in `extra`), then R1 is active.
    let ranges = ranges_in_order(std::slice::from_ref(&r2), r1.clone());
    assert_eq!(ranges, [r2.clone(), r1.clone()]);
    assert_eq!(
        copy_ranges(tsv, &ranges, &[0, 1, 2], &[false; 3], &cols, &row),
        "v21\n\nv00\n",
        "R2 block first"
    );
    // three ranges keep insertion order, not position order
    let r3: Block = (1..2, 2..3);
    assert_eq!(
        ranges_in_order(&[r2.clone(), r3.clone()], r1.clone()),
        [r2, r3, r1]
    );
    assert_eq!(
        ranges_in_order(&[], (4..5, 0..1)),
        [(4..5, 0..1)],
        "no extras: just the active range"
    );
}

#[test]
fn one_range_matches_the_single_range_copy_in_every_format() {
    let (cols, row) = vgrid();
    // display order [c, a, b] with b hidden, rows 0..2: literal output of each format
    let expected = [
        (
            tsv as fn(&[ColumnMeta], &[Vec<Value>]) -> String,
            "v02\tv00\nv12\tv10\n",
        ),
        (csv, "c,a\nv02,v00\nv12,v10\n"),
        (
            markdown,
            "| c | a |\n| --- | --- |\n| v02 | v00 |\n| v12 | v10 |\n",
        ),
        (
            json,
            "[\n  {\"c\": \"v02\", \"a\": \"v00\"},\n  {\"c\": \"v12\", \"a\": \"v10\"}\n]\n",
        ),
        (
            sql_insert,
            "INSERT INTO table_name (c, a) VALUES ('v02', 'v00');\nINSERT INTO table_name (c, a) VALUES ('v12', 'v10');\n",
        ),
        (sql_in_list, "('v02', 'v12')"),
    ];
    for (format, want) in expected {
        let got = copy_ranges(
            format,
            &[(0..2, 0..3)],
            &[2, 0, 1],
            &[false, true, false],
            &cols,
            &row,
        );
        assert_eq!(got, want);
    }
}

#[test]
fn in_list_merges_every_range_into_one_list() {
    let cols = ["a", "b", "c"]
        .map(|n| ColumnMeta {
            name: n.into(),
            data_type: "text".into(),
            is_pk: false,
            ..Default::default()
        })
        .to_vec();
    let rows: Vec<Vec<Value>> = vec![
        vec![Value::Int(1), Value::Text("x".into()), Value::Null],
        vec![
            Value::Int(2),
            Value::Text("it's".into()),
            Value::Text("z".into()),
        ],
        vec![Value::Int(1), Value::Null, Value::Int(7)],
        vec![Value::Int(5), Value::Text("y".into()), Value::Int(8)],
    ];
    let row = |r: usize| rows[r].clone();
    let id = [0, 1, 2];
    // (ranges, order, hidden, expected); duplicates are kept: the list mirrors the selection, and IN ignores repeats
    let cases: [(Vec<Block>, [usize; 3], [bool; 3], &str); 7] = [
        (
            vec![(0..2, 0..1), (3..4, 0..1)],
            id,
            [false; 3],
            "(1, 2, 5)",
        ),
        (vec![(0..1, 0..1)], id, [false; 3], "(1)"),
        (
            vec![(0..1, 1..2), (1..2, 1..2)],
            id,
            [false; 3],
            "('x', 'it''s')",
        ), // strings quoted, quote doubled
        (
            vec![(0..3, 2..3), (2..3, 1..2)],
            id,
            [false; 3],
            "(NULL, 'z', 7, NULL)",
        ), // NULLs, mixed types
        (vec![(0..1, 0..1), (2..3, 0..1)], id, [false; 3], "(1, 1)"), // duplicates across ranges kept
        (
            vec![(0..2, 0..2), (0..1, 1..3)],
            id,
            [false; 3],
            "(1, 2, 'x')",
        ), // first column of each range only; overlap
        (
            vec![(0..2, 0..2), (3..4, 0..2)],
            [1, 0, 2],
            [true, false, false],
            "('x', 'it''s', 'y')",
        ), // permuted, a hidden: b leads
    ];
    for (ranges, order, hidden, expected) in cases {
        assert_eq!(
            copy_in_list(&ranges, &order, &hidden, &cols, row),
            expected,
            "ranges {ranges:?}, order {order:?}, hidden {hidden:?}"
        );
    }
}
