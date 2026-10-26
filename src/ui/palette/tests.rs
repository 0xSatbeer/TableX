use super::{Command, ranked};
use crate::keymap::{DEFAULTS, NAMES};

fn commands() -> Vec<Command> {
    NAMES
        .iter()
        .map(|&(action, name, area)| Command {
            action,
            name,
            area,
            keys: None,
        })
        .collect()
}

fn names(all: &[Command], query: &str, recent: &[&str]) -> Vec<&'static str> {
    ranked(all, query, recent)
        .into_iter()
        .map(|(ix, _)| all[ix].name)
        .collect()
}

#[test]
fn every_action_in_the_keymap_table_is_listed() {
    let all = commands();
    let listed = names(&all, "", &[]);
    for &(_, name, _) in NAMES {
        assert!(listed.contains(&name), "{name} missing");
    }
    for (action, ..) in DEFAULTS.iter().filter(|(a, ..)| *a != "zed::NoAction") {
        assert!(
            NAMES.iter().any(|(a, ..)| a == action),
            "{action} has no palette name"
        );
    }
}

#[gpui_kit::test]
fn every_app_action_has_a_palette_name(cx: &mut gpui_kit::TestAppContext) {
    let ours = ["queryx::", "grid::", "console::", "theme::"];
    let missing: Vec<_> = cx.update(|cx| {
        cx.all_action_names()
            .iter()
            .filter(|a| ours.iter().any(|ns| a.starts_with(ns)))
            .filter(|a| !NAMES.iter().any(|(n, ..)| n == *a))
            .copied()
            .collect()
    });
    assert!(missing.is_empty(), "{missing:?}");
}

#[gpui_kit::test]
fn the_gallery_is_listed_only_in_demo_mode(cx: &mut gpui_kit::TestAppContext) {
    let cases = [(false, false), (true, true)];
    for (demo, listed) in cases {
        let commands = cx.update(|cx| super::Commands::new(demo, cx));
        let shown = (0..commands.shown.len()).filter_map(|row| commands.action(row));
        assert_eq!(
            shown.into_iter().any(|a| a == "queryx::ToggleGallery"),
            listed,
            "demo: {demo}"
        );
    }
}

#[test]
fn an_empty_filter_lists_recent_first_then_by_name() {
    let all = commands();
    let listed = names(&all, "", &["queryx::Quit", "grid::Paste"]);
    assert_eq!(listed[..2], ["Quit", "Paste"]);
    let rest = listed[2..].to_vec();
    let mut sorted = rest.clone();
    sorted.sort();
    assert_eq!(rest, sorted);
}

#[test]
fn typing_ranks_the_closest_name_first() {
    let all = commands();
    assert_eq!(names(&all, "cls tab", &[])[0], "Close tab");
    assert!(names(&all, "zzqx", &[]).is_empty());
}
