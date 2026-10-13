use super::*;

#[test]
fn expanding_rows() {
    let log = sample();
    // error row: full SQL plus the message
    let multi = entry(
        1,
        "SELECT *\nFROM t\nWHERE a = 1",
        QueryStatus::Err("relation \"t\" does not exist".into()),
        QuerySource::Console,
    );
    assert_eq!(
        row_detail(&multi),
        RowDetail {
            sql: "SELECT *\nFROM t\nWHERE a = 1".into(),
            error: Some("relation \"t\" does not exist".into())
        }
    );
    // non-error rows show SQL only
    assert_eq!(
        row_detail(&log[0]),
        RowDetail {
            sql: "SELECT 1".into(),
            error: None
        }
    );
    assert_eq!(
        row_detail(&log[4]).error,
        None,
        "cancelled has no error text"
    );

    // click expands, click again collapses; rows are independent
    let mut open = BTreeSet::new();
    toggle_expanded(&mut open, 3);
    assert!(open.contains(&3));
    toggle_expanded(&mut open, 5);
    assert_eq!(open, [3, 5].into());
    toggle_expanded(&mut open, 3);
    assert_eq!(open, [5].into());

    // survives a filter change: hidden while filtered out, open again when it returns
    let only_ok = LogFilter {
        statuses: [StatusKind::Ok].into(),
        ..LogFilter::default()
    };
    assert!(!filter_rows(&log, &only_ok).contains(&5));
    assert!(open.contains(&5));
    assert!(filter_rows(&log, &LogFilter::default()).contains(&5));
}

#[test]
fn time_column_is_labeled_utc() {
    let labels = super::header_labels();
    assert_eq!(labels[0], "Time (UTC)");
    assert_eq!(labels[3], "SQL");
}

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

#[gpui_kit::test]
fn truncated_log_sql_shows_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
    use crate::theme::tests::hover_shows_truncation_tip;
    use gpui_kit::{AppContext as _, Bounds, Entity, WindowBounds, WindowOptions, point, px, size};
    cx.update(gpui_kit::init);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(1200.), px(400.)),
        ))),
        ..Default::default()
    };
    let (window, panel): (_, Entity<super::BottomPanel>) = cx.update(|cx| {
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                super::BottomPanel::new(
                    cx.new(|_| crate::mock::workspaces().remove(0)),
                    crate::mock::mock_query_log(super::LOG_CAP),
                    window,
                    cx,
                )
            })
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let long = format!(
        "SELECT {} FROM orders",
        (0..40)
            .map(|i| format!("column_{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    panel.update(&mut cx, |p, cx| {
        let entry = p.log[0].clone();
        p.log = vec![
            super::QueryLogEntry {
                sql: "SELECT 1".into(),
                ..entry.clone()
            },
            super::QueryLogEntry { sql: long, ..entry },
        ];
        p.filter = super::LogFilter {
            introspection: true,
            ..Default::default()
        };
        p.show_logs();
        p.refilter(cx);
        cx.notify();
    });
    cx.run_until_parked();
    // (selector, tooltip shown); the one that fits first, so no earlier tooltip lingers.
    for (selector, tip) in [("log-sql-0", false), ("log-sql-1", true)] {
        assert_eq!(
            hover_shows_truncation_tip(&mut cx, selector.into()),
            tip,
            "{selector}"
        );
    }
}
