use super::*;

#[test]
fn sql_spans_tag_keywords_strings_and_numbers() {
    let sql = "select id FROM users WHERE name = 'jörg' LIMIT 50";
    let tagged: Vec<_> = super::sql_spans(sql)
        .into_iter()
        .map(|(r, k)| (&sql[r], k))
        .collect();
    assert_eq!(
        tagged,
        [
            ("select", "keyword"),
            ("FROM", "keyword"),
            ("WHERE", "keyword"),
            ("'jörg'", "string"),
            ("LIMIT", "keyword"),
            ("50", "number")
        ]
    );
}

#[test]
fn durations_pick_a_readable_unit() {
    assert_eq!(super::format_duration(Duration::from_micros(400)), "0.4 ms");
    assert_eq!(
        super::format_duration(Duration::from_micros(12_345)),
        "12 ms"
    );
    assert_eq!(
        super::format_duration(Duration::from_millis(1_250)),
        "1.2 s"
    );
}

#[test]
fn outcome_line_per_status() {
    let with = |status: QueryStatus, rows: Option<u64>| QueryLogEntry {
        rows,
        duration: Duration::from_millis(220),
        ..entry(1, "SELECT 1", status, QuerySource::Console)
    };
    for (e, want) in [
        (with(QueryStatus::Ok, Some(9)), "9 rows in 220 ms"),
        (with(QueryStatus::Ok, Some(1)), "1 row in 220 ms"),
        (with(QueryStatus::Ok, None), "Completed in 220 ms"),
        (with(QueryStatus::Err("boom".into()), None), "boom"),
        (with(QueryStatus::Cancelled, None), "Cancelled after 220 ms"),
    ] {
        assert_eq!(super::outcome(&e), want);
    }
    assert_eq!(
        super::stamp(std::time::SystemTime::now()).len(),
        "2026-11-01 18:15:37".len()
    );
}
