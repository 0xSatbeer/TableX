use super::{GROUPS, Sheet, rows};
use crate::keymap::effective;

fn names(sheet: &Sheet) -> Vec<(&'static str, &'static str, String)> {
    sheet
        .shown
        .iter()
        .flat_map(|(group, ixs)| {
            ixs.iter().map(|&ix| {
                (
                    GROUPS[*group],
                    sheet.all[ix].name,
                    sheet.all[ix].keys.clone(),
                )
            })
        })
        .collect()
}

#[test]
fn a_user_override_shows_its_keystroke_not_the_default() {
    let overrides = [("queryx::CloseTab".to_string(), "ctrl-alt-x".to_string())].into();
    let sheet = Sheet::new(rows(&effective(&overrides), true), String::new());
    let close = names(&sheet)
        .into_iter()
        .find(|(_, name, _)| *name == "Close tab")
        .expect("listed");
    assert_eq!(close, ("Tabs", "Close tab", "ctrl-alt-x".to_string()));
}

#[test]
fn filtering_by_keystroke_finds_close_tab() {
    let mut sheet = Sheet::new(rows(&effective(&Default::default()), true), String::new());
    sheet.filter("ctrl-w");
    let shown = names(&sheet);
    assert!(
        shown.iter().any(|(_, name, _)| *name == "Close tab"),
        "{shown:?}"
    );
    assert!(
        shown
            .iter()
            .all(|(_, name, keys)| keys.contains("ctrl-w") || name.contains("ctrl-w"))
    );
}

#[test]
fn unbound_actions_show_a_dash_and_every_group_is_known() {
    let overrides = [("grid::CopyCsv".to_string(), String::new())].into();
    let sheet = Sheet::new(rows(&effective(&overrides), true), String::new());
    let shown = names(&sheet);
    assert!(shown.contains(&("Grid", "Copy as CSV", "—".to_string())));
    assert!(
        shown
            .iter()
            .any(|(_, name, _)| *name == "Keyboard shortcuts")
    );
    assert_eq!(shown.len(), crate::keymap::NAMES.len());
}

#[test]
fn filtering_by_name_is_case_insensitive() {
    let mut sheet = Sheet::new(rows(&effective(&Default::default()), true), String::new());
    sheet.filter("ZOOM IN");
    assert_eq!(names(&sheet), [("App", "Zoom in", "ctrl-=".to_string())]);
}

#[test]
fn the_gallery_is_listed_only_in_demo_mode() {
    let cases = [(false, false), (true, true)];
    for (demo, listed) in cases {
        let sheet = Sheet::new(rows(&effective(&Default::default()), demo), String::new());
        assert_eq!(
            names(&sheet)
                .iter()
                .any(|(_, name, _)| *name == "Toggle component gallery"),
            listed,
            "demo: {demo}"
        );
    }
}
