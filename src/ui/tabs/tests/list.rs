use super::*;

#[test]
fn preview_replaces_preview_and_pins_on_reopen() {
    let mut t = Tabs::new(vec![tab("console", false)]);
    t.open(tab("users", true));
    t.open(tab("orders", true));
    assert_eq!(
        keys(&t),
        ["console", "orders"],
        "second preview replaced the first"
    );
    t.open(tab("orders", false));
    assert!(!t.tabs[1].preview, "opening pinned pins the preview");
    t.open(tab("items", true));
    assert_eq!(keys(&t), ["console", "orders", "items"]);
    assert_eq!(t.active, 2);
}

#[test]
fn close_and_reopen_keep_active_sane() {
    let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", false)]);
    t.activate(2);
    t.close(2);
    assert_eq!(
        t.active, 0,
        "closing the active tab activates the next MRU tab"
    );
    t.close(0);
    assert_eq!((keys(&t), t.active), (vec!["b"], 0));
    t.reopen();
    assert_eq!((keys(&t), t.active), (vec!["b", "a"], 1));
}

#[test]
fn pin_moves_left_and_close_others_keeps_active() {
    let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", true)]);
    t.toggle_pin(2);
    assert_eq!((keys(&t), t.active), (vec!["c", "a", "b"], 0));
    assert!(
        t.tabs[0].pinned && !t.tabs[0].preview,
        "pinning also ends preview"
    );
    t.activate(2);
    let active = t.active;
    for ix in t.closable(|ix| ix != active).into_iter().rev() {
        t.close(ix);
    }
    assert_eq!(
        (keys(&t), t.active),
        (vec!["c", "b"], 1),
        "close others keeps pinned tabs"
    );

    let mut t = Tabs::new(vec![
        tab("a", false),
        tab("b", false),
        tab("c", false),
        tab("d", false),
    ]);
    t.activate(1);
    for ix in t.closable(|ix| ix > 1).into_iter().rev() {
        t.close(ix);
    }
    assert_eq!(
        (keys(&t), t.active),
        (vec!["a", "b"], 1),
        "close to the right keeps the active tab"
    );
}

#[test]
fn move_tab_unpinned() {
    // (from, drop slot, expected)
    let cases: [(usize, usize, [&str; 4]); 8] = [
        (0, 3, ["b", "c", "a", "d"]),
        (3, 0, ["d", "a", "b", "c"]),
        (0, 4, ["b", "c", "d", "a"]),
        (1, 1, ["a", "b", "c", "d"]),
        (1, 2, ["a", "b", "c", "d"]),
        (2, 99, ["a", "b", "d", "c"]),
        (99, 0, ["a", "b", "c", "d"]),
        (0, 0, ["a", "b", "c", "d"]),
    ];
    for (from, to, want) in cases {
        let mut t = Tabs::new(vec![
            tab("a", false),
            tab("b", false),
            tab("c", false),
            tab("d", false),
        ]);
        t.move_tab(from, to);
        assert_eq!(keys(&t), want, "move {from} before {to}");
    }
    for n in 0..=1 {
        let mut t = Tabs::new((0..n).map(|_| tab("a", false)).collect());
        t.move_tab(0, 0);
        t.move_tab(0, 1);
        assert_eq!(t.tabs.len(), n, "{n} tabs: nothing to move");
    }
}

#[test]
fn move_tab_keeps_pinned_block_left() {
    // (from, drop slot, expected); tabs are [pa, pb, c, d]
    let cases: [(usize, usize, [&str; 4]); 7] = [
        (3, 0, ["pa", "pb", "d", "c"]),
        (3, 1, ["pa", "pb", "d", "c"]),
        (2, 0, ["pa", "pb", "c", "d"]),
        (0, 4, ["pb", "pa", "c", "d"]),
        (0, 3, ["pb", "pa", "c", "d"]),
        (1, 0, ["pb", "pa", "c", "d"]),
        (0, 1, ["pa", "pb", "c", "d"]),
    ];
    for (from, to, want) in cases {
        let mut t = Tabs::new(vec![
            pinned("pa"),
            pinned("pb"),
            tab("c", false),
            tab("d", false),
        ]);
        t.move_tab(from, to);
        assert_eq!(keys(&t), want, "move {from} before {to}");
    }
}

#[test]
fn move_tab_active_follows_its_tab() {
    // (active, from, drop slot, expected keys, expected active)
    let cases: [(usize, usize, usize, [&str; 3], usize); 6] = [
        (2, 2, 0, ["c", "a", "b"], 0),
        (2, 2, 2, ["a", "b", "c"], 2),
        (0, 2, 0, ["c", "a", "b"], 1),
        (1, 0, 3, ["b", "c", "a"], 0),
        (1, 1, 3, ["a", "c", "b"], 2),
        (1, 0, 1, ["a", "b", "c"], 1),
    ];
    for (active, from, to, want, want_active) in cases {
        let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", false)]);
        t.activate(active);
        let key = t.tabs[active].key.clone();
        t.move_tab(from, to);
        assert_eq!(
            (keys(&t), t.active),
            (want.to_vec(), want_active),
            "active {active}, move {from} before {to}"
        );
        assert_eq!(t.tabs[t.active].key, key, "same tab stays active");
    }
}
