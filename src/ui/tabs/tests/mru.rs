use super::*;

#[test]
fn ctrl_tab_release_commits_previous_mru_tab() {
    // (MRU before, steps, active after, MRU after)
    let cases: [(&[&'static str], &str, &str, &[&str]); 2] = [
        (&["c", "b", "a"], "nr", "b", &["b", "c", "a"]),
        (&["b", "a"], "nr", "a", &["a", "b"]),
    ];
    for (mru, steps, active, want) in cases {
        let mut t = with_mru(mru);
        run(&mut t, steps);
        assert_eq!(
            state(&t),
            (active, want.to_vec()),
            "MRU {mru:?}, steps {steps}"
        );
    }
}

#[test]
fn ctrl_tab_held_walks_further_down_the_mru() {
    // (MRU before, steps, active after, MRU after)
    let cases: [(&[&'static str], &str, &str, &[&str]); 3] = [
        (&["c", "b", "a"], "nnr", "a", &["a", "c", "b"]),
        (&["c", "b", "a"], "nn", "a", &["c", "b", "a"]),
        (&["c", "b", "a"], "nrnr", "c", &["c", "b", "a"]),
    ];
    for (mru, steps, active, want) in cases {
        let mut t = with_mru(mru);
        run(&mut t, steps);
        assert_eq!(
            state(&t),
            (active, want.to_vec()),
            "MRU {mru:?}, steps {steps}"
        );
    }
}

#[test]
fn ctrl_tab_wraps_and_shift_walks_backward() {
    // (MRU before, steps, active after, MRU after)
    let cases: [(&[&'static str], &str, &str, &[&str]); 6] = [
        (&["c", "b", "a"], "pr", "a", &["a", "c", "b"]),
        (&["c", "b", "a"], "nnnr", "c", &["c", "b", "a"]),
        (&["c", "b", "a"], "ppr", "b", &["b", "c", "a"]),
        (&["c", "b", "a"], "npr", "c", &["c", "b", "a"]),
        (&["c", "b", "a"], "nnnnr", "b", &["b", "c", "a"]),
        (&["b", "a"], "nnr", "b", &["b", "a"]),
    ];
    for (mru, steps, active, want) in cases {
        let mut t = with_mru(mru);
        run(&mut t, steps);
        assert_eq!(
            state(&t),
            (active, want.to_vec()),
            "MRU {mru:?}, steps {steps}"
        );
    }
}

#[test]
fn ctrl_tab_with_one_or_no_tab_changes_nothing() {
    for steps in ["nr", "pr", "nnr", "n", "r"] {
        let mut t = with_mru(&["a"]);
        run(&mut t, steps);
        assert_eq!(state(&t), ("a", vec!["a"]), "one tab, steps {steps}");
        let mut t: Tabs<()> = Tabs::new(Vec::new());
        run(&mut t, steps);
        assert_eq!(
            (t.tabs.len(), t.active, t.mru.len()),
            (0, 0, 0),
            "no tabs, steps {steps}"
        );
    }
}

#[test]
fn close_activates_next_mru_tab() {
    // (MRU before, steps before the close, closed key, active after, MRU after); tabs [a, b, c]
    let cases: [(&[&'static str], &str, &str, &str, &[&str]); 7] = [
        (&["c", "b", "a"], "", "c", "b", &["b", "a"]),
        (&["a", "c", "b"], "", "a", "c", &["c", "b"]),
        (&["c", "b", "a"], "", "a", "c", &["c", "b"]),
        (&["c", "b", "a"], "", "b", "c", &["c", "a"]),
        // Mid-cycle: the previewed tab is committed first, as if Ctrl were released.
        (&["c", "b", "a"], "n", "b", "c", &["c", "a"]),
        (&["c", "b", "a"], "n", "a", "b", &["b", "c"]),
        (&["c", "b", "a"], "nn", "c", "a", &["a", "b"]),
    ];
    for (mru, steps, closed, active, want) in cases {
        let mut t = with_mru(mru);
        run(&mut t, steps);
        let ix = t.tabs.iter().position(|x| x.key == closed).unwrap();
        t.close(ix);
        assert_eq!(
            state(&t),
            (active, want.to_vec()),
            "MRU {mru:?}, steps {steps}, close {closed}"
        );
        t.commit();
        assert_eq!(
            state(&t),
            (active, want.to_vec()),
            "release after the close changes nothing"
        );
    }
    let mut t = with_mru(&["a"]);
    t.close(0);
    assert_eq!(
        (t.tabs.len(), t.active, t.mru.len()),
        (0, 0, 0),
        "closing the last tab"
    );
}

#[test]
fn activating_moves_tab_to_mru_front() {
    // (MRU before, steps, tab order after, active after, MRU after); tabs [a, b, c]
    let cases: [(&[&'static str], &str, &[&str], &str, &[&str]); 11] = [
        (
            &["c", "b", "a"],
            "0",
            &["a", "b", "c"],
            "a",
            &["a", "c", "b"],
        ),
        (
            &["c", "b", "a"],
            "1",
            &["a", "b", "c"],
            "b",
            &["b", "c", "a"],
        ),
        (
            &["b", "a", "c"],
            "2",
            &["a", "b", "c"],
            "c",
            &["c", "b", "a"],
        ),
        (
            &["c", "b", "a"],
            "n0r",
            &["a", "b", "c"],
            "a",
            &["a", "c", "b"],
        ),
        (
            &["c", "b", "a"],
            "o",
            &["a", "b", "c", "d"],
            "d",
            &["d", "c", "b", "a"],
        ),
        (
            &["c", "b", "a"],
            "nor",
            &["a", "b", "c", "d"],
            "d",
            &["d", "c", "b", "a"],
        ),
        (
            &["c", "b", "a"],
            "e",
            &["a", "b", "c"],
            "a",
            &["a", "c", "b"],
        ),
        (
            &["c", "b", "a"],
            "wu",
            &["a", "b", "c"],
            "c",
            &["c", "b", "a"],
        ),
        (
            &["c", "b", "a"],
            "nnmr",
            &["a", "b", "c"],
            "a",
            &["a", "c", "b"],
        ),
        (
            &["c", "b", "a"],
            "nmr",
            &["b", "a", "c"],
            "b",
            &["b", "c", "a"],
        ),
        (
            &["c", "b", "a"],
            "1k",
            &["b", "a", "c"],
            "b",
            &["b", "c", "a"],
        ),
    ];
    for (mru, steps, order, active, want) in cases {
        let mut t = with_mru(mru);
        run(&mut t, steps);
        assert_eq!(
            (keys(&t), state(&t)),
            (order.to_vec(), (active, want.to_vec())),
            "MRU {mru:?}, steps {steps}"
        );
    }
    let mut t = with_mru(&["a"]);
    run(&mut t, "wu");
    assert_eq!(
        state(&t),
        ("a", vec!["a"]),
        "reopen after closing the last tab"
    );
    let mut t = with_mru(&["c", "b", "a"]);
    run(&mut t, "nwu");
    assert_eq!(
        state(&t),
        ("b", vec!["b", "c", "a"]),
        "reopen the tab closed mid-cycle"
    );

    let mut t = Tabs::new(vec![tab("a", false), tab("p", true)]);
    t.open(tab("q", true));
    assert_eq!(
        (keys(&t), state(&t)),
        (vec!["a", "q"], ("q", vec!["q", "a"])),
        "a replaced preview leaves the MRU"
    );
}
