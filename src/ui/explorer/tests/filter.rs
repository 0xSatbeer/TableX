//! The filter field.

use super::*;

#[test]
fn filter_keeps_matches_and_ancestors() {
    assert_eq!(fuzzy_match("order_items", "oit"), Some(vec![0, 6, 7]));
    assert_eq!(fuzzy_match("Users", "us"), Some(vec![0, 1]));
    assert_eq!(fuzzy_match("users", "x"), None);

    let ws = &mock::workspaces()[0];
    let all = flatten(ws, &Default::default(), &Default::default(), true);
    let target = all
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Object(_)))
        .unwrap()
        .label
        .to_string();
    let kept = filter_nodes(all.clone(), &target);
    let at = kept
        .iter()
        .position(|n| n.label.as_ref() == target)
        .expect("match kept");
    // Every kept node is a match or an ancestor of one, and the match has its full chain above it.
    assert_eq!(
        (0..kept[at].depth)
            .filter(|d| kept[..at].iter().any(|n| n.depth == *d))
            .count(),
        kept[at].depth
    );
    assert!(kept.len() < all.len());
}

#[test]
fn typing_ordr_shows_orders_highlighted_with_ancestors() {
    let ws = &mock::workspaces()[0];
    let nodes = tree_nodes(ws, &Default::default(), &Default::default(), "ordr");
    let at = nodes
        .iter()
        .position(|n| n.label.as_ref() == "orders")
        .expect("orders shown");
    assert_eq!(
        highlight_ranges("orders", "ordr"),
        vec![0..1, 1..2, 2..3, 4..5]
    );
    assert_eq!(
        highlight_ranges("orders", ""),
        Vec::<std::ops::Range<usize>>::new()
    );
    assert_eq!(
        highlight_ranges("users", "ordr"),
        Vec::<std::ops::Range<usize>>::new()
    );
    // The real parent chain (connection > database > schema > Tables group), not just any shallower rows.
    let mut chain = vec![];
    let mut cur = at;
    while let Some(p) = parent_of(&nodes, cur) {
        chain.push(nodes[p].key.to_string());
        cur = p;
    }
    chain.reverse();
    let key = nodes[at].key.to_string();
    let want: Vec<String> = key
        .match_indices('/')
        .take(4)
        .map(|(i, _)| key[..i].to_string())
        .collect();
    assert_eq!(chain, want, "ancestors of orders are its own parent chain");
    assert_eq!(chain.len(), 4);
}

#[test]
fn empty_field_restores_the_expanded_set() {
    let ws = &mock::workspaces()[0];
    let expanded = default_expanded(ws);
    let none = Default::default();
    let normal = flatten(ws, &expanded, &none, false);
    let keys = |v: &[super::FlatNode]| v.iter().map(|n| n.key.clone()).collect::<Vec<_>>();
    let filtered = tree_nodes(ws, &expanded, &none, "ordr");
    assert_ne!(keys(&filtered), keys(&normal), "filter changes the tree");
    assert_eq!(keys(&tree_nodes(ws, &expanded, &none, "")), keys(&normal));
    assert_eq!(
        first_match(&normal, ""),
        None,
        "no first match without a filter"
    );
    assert_eq!(
        first_match(&filtered, "ordr").map(|i| filtered[i].label.as_ref()),
        Some("orders")
    );
    assert_eq!(first_match(&filtered, "zzzz"), None);
}

#[test]
fn typing_in_the_tree_moves_into_the_field() {
    use KeyRoute::*;
    use gpui_kit::Modifiers as M;
    let ch = |c: &'static str| Some(c);
    // (key, key_char, modifiers, field_empty, renaming, expected)
    let cases = [
        ("u", ch("u"), M::none(), true, false, ToField("u".into())),
        ("u", ch("U"), M::shift(), true, false, ToField("U".into())),
        ("u", ch("u"), M::none(), false, false, ToField("u".into())),
        ("u", ch("u"), M::control(), true, false, Pass),
        ("u", ch("u"), M::alt(), true, false, Pass),
        ("u", ch("u"), M::command(), true, false, Pass),
        ("u", ch("u"), M::none(), true, true, Pass),
        ("space", ch(" "), M::none(), true, false, Pass),
        ("enter", ch("\n"), M::none(), true, false, Pass),
        ("*", ch("*"), M::none(), true, false, Pass),
        ("*", ch("*"), M::none(), false, false, ToField("*".into())),
        ("up", None, M::none(), true, false, Pass),
        ("backspace", None, M::none(), true, false, Pass),
        ("backspace", None, M::none(), false, false, PopField),
        ("escape", None, M::none(), false, false, ClearField),
        ("escape", None, M::none(), true, false, Pass),
    ];
    for (key, key_char, m, empty, renaming, want) in cases {
        assert_eq!(
            route_key(Focus::Tree, key, key_char, m, empty, || true, renaming),
            want,
            "tree {key:?} {key_char:?} {m:?} empty={empty} renaming={renaming}"
        );
    }
}

#[test]
fn field_keys_escape_and_down() {
    use KeyRoute::*;
    use gpui_kit::Modifiers as M;
    // (key, modifiers, field_empty, has_match, expected)
    let cases = [
        ("escape", M::none(), false, true, ClearField),
        ("escape", M::none(), false, false, ClearField),
        ("escape", M::none(), true, false, FocusTree),
        ("escape", M::none(), true, true, FocusTree),
        ("down", M::none(), false, true, FocusFirstMatch),
        ("down", M::none(), false, false, Pass),
        ("down", M::none(), true, true, Pass),
        ("down", M::shift(), false, true, Pass),
        ("u", M::none(), false, true, Pass),
        ("backspace", M::none(), false, true, Pass),
    ];
    for (key, m, empty, has_match, want) in cases {
        assert_eq!(
            route_key(
                Focus::Field,
                key,
                (key == "u").then_some("x"),
                m,
                empty,
                || has_match,
                false
            ),
            want,
            "field {key:?} empty={empty} match={has_match}"
        );
    }
    // Only "down" needs the (costly) match scan.
    for key in ["escape", "u", "backspace", "left"] {
        route_key(
            Focus::Field,
            key,
            None,
            M::none(),
            false,
            || panic!("{key:?} must not compute first_match"),
            false,
        );
    }
    // Esc twice: first clears, second leaves for the tree.
    let (mut text, mut seq) = ("ord".to_string(), vec![]);
    for _ in 0..2 {
        let r = route_key(
            Focus::Field,
            "escape",
            None,
            M::none(),
            text.is_empty(),
            || true,
            false,
        );
        if r == ClearField {
            text.clear();
        }
        seq.push(r);
    }
    assert_eq!(seq, vec![ClearField, FocusTree]);
    // F2 rename in progress keeps its keys even if Esc reaches the router.
    assert_eq!(
        route_key(Focus::Tree, "escape", None, M::none(), false, || true, true),
        Pass
    );
}

#[test]
fn no_match_shows_one_muted_row() {
    let ws = &mock::workspaces()[0];
    assert!(
        tree_nodes(ws, &Default::default(), &Default::default(), "xyz").is_empty(),
        "nothing survives the filter"
    );
    assert_eq!(no_match_text("xyz"), "No objects match \"xyz\"");
    assert_eq!(no_match_text("a b"), "No objects match \"a b\"");
}

#[gpui_kit::test]
fn reveal_clears_the_field_text_and_the_filter(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{AppContext as _, Entity};
    cx.update(gpui_kit::init);
    let ws = mock::workspaces().remove(0);
    let key = flatten(&ws, &default_expanded(&ws), &Default::default(), true)
        .into_iter()
        .find(|n| n.label.as_ref() == "orders")
        .expect("orders")
        .key;
    let (window, explorer): (_, Entity<super::Explorer>) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| {
                super::Explorer::new(
                    cx.new(|_| ws),
                    Arc::new(crate::db::Live::default()),
                    window,
                    cx,
                )
            })
        })
        .expect("open test window")
    });
    cx.update_window(window, |_, window, cx| {
        explorer.update(cx, |e, cx| {
            e.put_filter("ordr".into(), window, cx);
            e.reveal(&key, window, cx);
            assert_eq!(
                e.field.read(cx).value().to_string(),
                "",
                "field text cleared"
            );
            assert_eq!(e.filter, "", "filter cleared");
        });
    })
    .unwrap();
}
