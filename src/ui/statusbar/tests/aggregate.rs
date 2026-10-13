use crate::model::Value::{self, *};
use crate::ui::statusbar::aggregate;

#[test]
fn single_cell_or_no_selection_is_hidden() {
    // (rows, cols, values, expected shown)
    let cases: [(usize, usize, Vec<Value>, bool); 5] = [
        (1, 1, vec![Int(3)], false),
        (1, 1, vec![Null], false),
        (0, 0, vec![], false), // no grid focused
        (2, 1, vec![Int(3), Int(4)], true),
        (1, 2, vec![Null, Null], true),
    ];
    for (rows, cols, values, shown) in cases {
        assert_eq!(
            aggregate(rows, cols, values.clone()).is_some(),
            shown,
            "{rows}x{cols} {values:?}"
        );
    }
}

/// Runs `(rows, cols, values, expected)` cases through `aggregate`.
fn check(cases: impl IntoIterator<Item = (usize, usize, Vec<Value>, &'static str)>) {
    for (rows, cols, values, expected) in cases {
        assert_eq!(
            aggregate(rows, cols, values.clone()).as_deref(),
            Some(expected),
            "{rows}x{cols} {values:?}"
        );
    }
}

#[test]
fn int_selection_shows_shape_count_sum_avg() {
    check([
        (
            2,
            1,
            vec![Int(3), Int(4)],
            "2 rows × 1 col · count 2 · sum 7 · avg 3.5",
        ),
        (
            1,
            2,
            vec![Int(3), Int(4)],
            "1 row × 2 cols · count 2 · sum 7 · avg 3.5",
        ),
        (
            2,
            2,
            vec![Int(-1), Int(-2), Int(0), Int(7)],
            "2 rows × 2 cols · count 4 · sum 4 · avg 1",
        ),
    ]);
}

#[test]
fn nulls_are_not_counted_and_non_numeric_cells_have_no_sum() {
    let text = |s: &str| Text(s.into());
    check([
        (
            3,
            1,
            vec![Int(3), Null, Int(4)],
            "3 rows × 1 col · count 2 · sum 7 · avg 3.5",
        ),
        (2, 1, vec![text("a"), text("b")], "2 rows × 1 col · count 2"),
        (2, 1, vec![Null, Null], "2 rows × 1 col · count 0"),
        // Non-numeric cells count, but sum and avg cover the numeric ones only.
        (
            3,
            1,
            vec![text("a"), Int(2), Int(4)],
            "3 rows × 1 col · count 3 · sum 6 · avg 3",
        ),
        (
            2,
            1,
            vec![Bool(true), Bool(false)],
            "2 rows × 1 col · count 2",
        ),
        (
            1,
            3,
            vec![Null, Int(5), Null],
            "1 row × 3 cols · count 1 · sum 5 · avg 5",
        ),
    ]);
}

#[test]
fn floats_show_at_most_four_decimals_trimmed() {
    check([
        (
            2,
            1,
            vec![Int(1), Float(0.5)],
            "2 rows × 1 col · count 2 · sum 1.5 · avg 0.75",
        ),
        (
            2,
            1,
            vec![Float(2.0), Float(2.0)],
            "2 rows × 1 col · count 2 · sum 4 · avg 2",
        ),
        (
            3,
            1,
            vec![Int(1), Int(0), Int(0)],
            "3 rows × 1 col · count 3 · sum 1 · avg 0.3333",
        ),
        (
            3,
            1,
            vec![Int(2), Int(0), Int(0)],
            "3 rows × 1 col · count 3 · sum 2 · avg 0.6667",
        ),
        (
            2,
            1,
            vec![Float(0.12345), Float(0.0)],
            "2 rows × 1 col · count 2 · sum 0.1235 · avg 0.0617",
        ),
        // Rounds to zero: no "-0", no "0.0000".
        (
            2,
            1,
            vec![Float(-0.00001), Float(0.0)],
            "2 rows × 1 col · count 2 · sum 0 · avg 0",
        ),
        (
            2,
            1,
            vec![Float(f64::INFINITY), Int(1)],
            "2 rows × 1 col · count 2 · sum inf · avg inf",
        ),
        (
            3,
            1,
            vec![Float(1.5), Null, Text("x".into())],
            "3 rows × 1 col · count 2 · sum 1.5 · avg 1.5",
        ),
    ]);
}

#[test]
fn over_a_million_cells_shows_shape_only_without_reading_values() {
    // (rows, cols, expected); no value may be read.
    let skipped = [
        (
            1_000_001,
            1,
            "1000001 rows × 1 col · aggregate skipped (too many cells)",
        ),
        (
            1,
            1_000_001,
            "1 row × 1000001 cols · aggregate skipped (too many cells)",
        ),
        (
            1001,
            1000,
            "1001 rows × 1000 cols · aggregate skipped (too many cells)",
        ),
    ];
    for (rows, cols, expected) in skipped {
        let reads = std::cell::Cell::new(0);
        let values =
            std::iter::repeat_n(Int(1), rows * cols).inspect(|_| reads.set(reads.get() + 1));
        assert_eq!(aggregate(rows, cols, values).as_deref(), Some(expected));
        assert_eq!(reads.get(), 0, "{rows}x{cols} read values");
    }
    let at_limit = aggregate(1_000_000, 1, std::iter::repeat_n(Int(1), 1_000_000));
    assert_eq!(
        at_limit.as_deref(),
        Some("1000000 rows × 1 col · count 1000000 · sum 1000000 · avg 1")
    );
}

#[test]
fn int_sums_are_exact_past_i64() {
    check([
        (
            2,
            1,
            vec![Int(i64::MAX), Int(1)],
            "2 rows × 1 col · count 2 · sum 9223372036854775808 · avg 4611686018427387904",
        ),
        (
            2,
            1,
            vec![Int(i64::MIN), Int(-1)],
            "2 rows × 1 col · count 2 · sum -9223372036854775809 · avg -4611686018427387904",
        ),
        // Overflow in the middle cancels out; avg is f64, so only the sum is exact.
        (
            3,
            1,
            vec![Int(i64::MAX), Int(i64::MAX), Int(i64::MIN)],
            "3 rows × 1 col · count 3 · sum 9223372036854775806 · avg 3074457345618258432",
        ),
    ]);
}
