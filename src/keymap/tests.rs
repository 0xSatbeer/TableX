use super::{Binding, Bindings, DEFAULTS, Overrides, bind, effective, from_toml};

fn overrides(pairs: &[(&str, &str)]) -> Overrides {
    pairs
        .iter()
        .map(|(a, k)| (a.to_string(), k.to_string()))
        .collect()
}

fn keys_of<'a>(bindings: &'a [Binding], action: &str) -> Vec<&'a str> {
    bindings
        .iter()
        .filter(|b| b.action == action)
        .map(|b| b.keys.as_str())
        .collect()
}

#[test]
fn no_overrides_is_the_defaults() {
    assert_eq!(effective(&Overrides::new()).len(), DEFAULTS.len());
}

#[test]
fn an_override_replaces_every_default_of_its_action_in_the_same_context() {
    let bindings = effective(&overrides(&[("console::Cancel", "ctrl-c")]));
    let cancel: Vec<_> = bindings
        .iter()
        .filter(|b| b.action == "console::Cancel")
        .collect();
    assert_eq!(cancel.len(), 1);
    assert_eq!(cancel[0].keys, "ctrl-c");
    assert_eq!(cancel[0].context, Some("Console > Input"));
}

#[test]
fn an_empty_override_unbinds() {
    let bindings = effective(&overrides(&[("tablex::CloseTab", "")]));
    assert!(keys_of(&bindings, "tablex::CloseTab").is_empty());
    assert_eq!(bindings.len(), DEFAULTS.len() - 1);
}

#[test]
fn reads_the_bindings_table() {
    let parsed = from_toml(
        "version = 1\n[bindings]\n\"tablex::CloseTab\" = \"ctrl-q\"\n\"grid::CopyCsv\" = \"\"\n",
    )
    .unwrap();
    assert_eq!(
        parsed,
        overrides(&[("tablex::CloseTab", "ctrl-q"), ("grid::CopyCsv", "")])
    );
}

#[gpui_kit::test]
fn unknown_actions_and_bad_keystrokes_are_skipped_and_named(cx: &mut gpui_kit::TestAppContext) {
    let skipped = cx.update(|cx| {
        bind(
            cx,
            &overrides(&[
                ("nope::Missing", "ctrl-x"),
                ("nope::Gone", ""),
                ("tablex::ReopenTab", "ctrl-foo-x"),
                ("tablex::CloseTab", "ctrl-q"),
            ]),
        )
    });
    assert_eq!(skipped.len(), 3, "{skipped:?}");
    assert!(skipped.iter().any(|s| s.contains("nope::Missing")));
    assert!(skipped.iter().any(|s| s.contains("nope::Gone")));
    assert!(skipped.iter().any(|s| s.contains("tablex::ReopenTab")));
    let bound = cx.update(|cx| cx.global::<Bindings>().0.clone());
    assert_eq!(keys_of(&bound, "tablex::CloseTab"), ["ctrl-q"]);
    assert_eq!(keys_of(&bound, "tablex::ReopenTab"), ["ctrl-shift-t"]);
}
