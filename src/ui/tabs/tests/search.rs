use super::*;

#[test]
fn overflow_search_field_only_above_ten_tabs() {
    for (count, shown) in [(0, false), (1, false), (10, false), (11, true), (12, true)] {
        assert_eq!(super::show_search(count), shown, "{count} tabs");
    }
}

#[test]
fn match_tabs_filters_by_title() {
    let all = ["orders", "users", "ord_items"];
    for (query, want) in [
        ("ord", vec!["orders", "ord_items"]),
        ("ORD", vec!["orders", "ord_items"]),
        ("items", vec!["ord_items"]),
        ("", vec!["orders", "users", "ord_items"]),
        ("zzz", vec![]),
    ] {
        assert_eq!(matches(&all, query), want, "query {query:?}");
    }
    assert!(matches(&[], "ord").is_empty());
}

#[test]
fn match_tabs_filters_by_path() {
    let origin = |path: &'static str| {
        Some(Origin {
            connection: crate::model::ConnectionId(0),
            color: 0,
            production: false,
            path: path.into(),
            tail: "".into(),
        })
    };
    let tabs = [
        EditorTab {
            origin: origin("prod-pg › app_db › public"),
            ..titled("users")
        },
        EditorTab {
            origin: origin("prod-pg › app_db › audit"),
            ..titled("log")
        },
        titled("big_events"),
    ];
    for (query, want) in [
        ("PUBLIC", vec![0]),
        ("public", vec![0]),
        ("app_db", vec![0, 1]),
        ("prod-pg › app_db › audit", vec![1]),
        ("sample", vec![]),
    ] {
        assert_eq!(super::match_tabs(&tabs, query), want, "query {query:?}");
    }
}

#[test]
fn overflow_no_match_is_labelled() {
    let tabs = [titled("orders"), titled("users"), titled("ord_items")];
    for (query, want) in [("zzz", Some("No tabs match")), ("ord", None), ("", None)] {
        assert_eq!(super::empty_state(&tabs, query), want, "query {query:?}");
    }
    assert_eq!(super::empty_state::<()>(&[], ""), Some("No tabs match"));
}

#[test]
fn overflow_enter_picks_first_match() {
    let tabs = [titled("orders"), titled("users"), titled("ord_items")];
    for (query, want) in [
        ("ord", Some(0)),
        ("ite", Some(2)),
        ("", Some(0)),
        ("zzz", None),
    ] {
        assert_eq!(super::first_match(&tabs, query), want, "query {query:?}");
    }
    assert_eq!(super::first_match::<()>(&[], ""), None);
}
