use super::*;

#[test]
fn empty_state_message() {
    assert_eq!(
        super::empty_message(&[], 4),
        Some(("No log entries match these filters", true))
    );
    assert_eq!(
        super::empty_message(&[], 0),
        Some(("Queries you run will show up here", false))
    );
    assert_eq!(super::empty_message(&[3, 1], 4), None);
}

#[test]
fn status_filter() {
    let log = crate::mock::mock_query_log(500);
    let with = |statuses: &[StatusKind]| {
        let filter = LogFilter {
            statuses: statuses.iter().copied().collect(),
            introspection: true,
            ..LogFilter::default()
        };
        filter_rows(&log, &filter)
    };
    let kind = |ix: usize| StatusKind::of(&log[ix].status);
    // (statuses, expected kinds that may appear)
    let cases: [(&[StatusKind], &[StatusKind]); 5] = [
        (&[StatusKind::Error], &[StatusKind::Error]),
        (&[StatusKind::Ok], &[StatusKind::Ok]),
        (&[StatusKind::Cancelled], &[StatusKind::Cancelled]),
        (
            &[StatusKind::Ok, StatusKind::Cancelled],
            &[StatusKind::Ok, StatusKind::Cancelled],
        ),
        (&[], &[]),
    ];
    for (statuses, allowed) in cases {
        let rows = with(statuses);
        assert!(
            rows.iter().all(|&ix| allowed.contains(&kind(ix))),
            "{statuses:?}"
        );
        assert_eq!(
            rows.is_empty(),
            allowed.is_empty()
                || !log
                    .iter()
                    .any(|e| allowed.contains(&StatusKind::of(&e.status))),
            "{statuses:?}"
        );
    }
    assert!(
        !with(&[StatusKind::Error]).is_empty(),
        "mock log has errors"
    );
    assert!(!with(&[StatusKind::Ok]).is_empty(), "mock log has ok rows");
    assert_eq!(
        with(&[StatusKind::Ok, StatusKind::Error, StatusKind::Cancelled]).len(),
        log.len()
    );
}

#[test]
fn connection_and_text_filter() {
    let log = sample();
    let run = |conns: &[u32], text: &str, statuses: Option<&[StatusKind]>| {
        let mut f = LogFilter {
            text: text.into(),
            connections: conns.iter().copied().collect(),
            ..LogFilter::default()
        };
        if let Some(s) = statuses {
            f.statuses = s.iter().copied().collect();
        }
        filter_rows(&log, &f)
    };
    // (connections, text, statuses, expected rows)
    let cases: [(&[u32], &str, Option<&[StatusKind]>, &[usize]); 9] = [
        (&[], "", None, &[0, 1, 2, 3, 4, 5]), // empty set = all connections
        (&[1], "select", None, &[0, 4]),      // spec case, text is case-insensitive
        (&[1], "", None, &[0, 2, 4]),
        (&[1, 2], "", None, &[0, 1, 2, 4, 5]), // several names
        (&[], "SELECT", None, &[0, 1, 3, 4]),  // upper-case query
        (&[], "select", None, &[0, 1, 3, 4]),
        (&[1, 2], "select", Some(&[StatusKind::Ok]), &[0, 1]), // text + status + connection
        (&[1], "select", Some(&[StatusKind::Cancelled]), &[4]), // cancelled is not ok
        (&[9], "", None, &[]),                                 // unknown connection
    ];
    for (conns, text, statuses, expected) in cases {
        assert_eq!(
            run(conns, text, statuses),
            expected,
            "{conns:?} {text:?} {statuses:?}"
        );
    }
}

#[test]
fn introspection_hidden_by_default() {
    let c = QuerySource::Console;
    let i = QuerySource::Introspection;
    let log = vec![
        entry(1, "SELECT 1", QueryStatus::Ok, c),
        entry(1, "SELECT * FROM pg_class", QueryStatus::Ok, i),
        entry(1, "SELECT * FROM pg_class", QueryStatus::Err("x".into()), i),
        entry(1, "SELECT 2", QueryStatus::Ok, QuerySource::TableView),
    ];
    let f = |introspection: bool, text: &str| {
        filter_rows(
            &log,
            &LogFilter {
                introspection,
                text: text.into(),
                ..LogFilter::default()
            },
        )
    };
    assert_eq!(
        filter_rows(&log, &LogFilter::default()),
        [0, 3],
        "default is off"
    );
    assert_eq!(f(false, ""), [0, 3]);
    assert_eq!(f(true, ""), [0, 1, 2, 3]);
    assert_eq!(
        f(false, "pg_class"),
        Vec::<usize>::new(),
        "text can't bring hidden rows back"
    );
    assert_eq!(f(true, "pg_class"), [1, 2]);
}
