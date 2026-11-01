use super::*;

#[test]
fn missing_user_theme_dir_loads_nothing_and_reports_nothing() {
    let dir = test_dir("missing").join("themes");
    let (themes, errors) = super::load_user_themes(&dir);
    assert!(themes.is_empty(), "themes from a missing dir");
    assert_eq!(
        errors,
        Vec::<String>::new(),
        "a missing dir is not an error"
    );
    assert!(!dir.exists(), "loading must not create the dir");
}

#[test]
fn unreadable_theme_dir_is_reported_unlike_a_missing_one() {
    let dir = test_dir("not-a-dir").join("themes");
    std::fs::write(&dir, "").unwrap(); // a file where the folder should be: ENOTDIR
    let (themes, errors) = super::load_user_themes(&dir);
    assert!(themes.is_empty());
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].starts_with(&format!("Couldn't read theme folder {}: ", dir.display())),
        "{errors:?}"
    );
}

#[test]
fn theme_file_that_is_not_text_is_skipped_and_reported() {
    let dir = test_dir("not-utf8");
    std::fs::write(dir.join("bad.json"), [0xFF, 0xFE]).unwrap();
    std::fs::write(dir.join("nord.json"), NORD).unwrap();
    let (themes, errors) = super::load_user_themes(&dir);
    assert_eq!(names(&themes), ["Nord"]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].starts_with("Couldn't load theme bad.json: ") && errors[0].len() > 30,
        "{errors:?}"
    );
}

const NORD: &str = r##"{"name": "Nord", "themes": [{"name": "Nord", "mode": "dark", "colors": {"background": "#2E3440"}}]}"##;
const PAPER: &str = r##"{"name": "Paper", "themes": [{"name": "Paper", "mode": "light", "colors": {"background": "#F7F3E8"}}]}"##;

fn names(themes: &[std::rc::Rc<gpui_kit::component::theme::ThemeConfig>]) -> Vec<String> {
    themes.iter().map(|t| t.name.to_string()).collect()
}

#[test]
fn valid_user_theme_file_is_listed() {
    let dir = test_dir("valid");
    std::fs::write(dir.join("nord.json"), NORD).unwrap();
    let (themes, errors) = super::load_user_themes(&dir);
    assert_eq!((names(&themes), errors), (vec!["Nord".to_string()], vec![]));
}

/// Each case is a fresh dir: (files written into it, theme names loaded, errors expected).
/// An expected error ending in ": " is matched as a prefix followed by the parse error.
#[test]
fn user_theme_files_load_skip_or_report() {
    const TWO: &str = r##"{"name": "Set", "themes": [{"name": "B", "mode": "dark"}, {"name": "A", "mode": "light"}]}"##;
    let bad = "Couldn't load theme bad.json: ";
    let cases: [(&[(&str, &str)], &[&str], &[&str]); 11] = [
        (&[("bad.json", "not json")], &[], &[bad]),
        (&[("x.txt", NORD)], &[], &[]), // valid theme, wrong extension
        (&[("nord", NORD)], &[], &[]),  // no extension
        (&[("NORD.JSON", NORD)], &[], &[]), // does not end in `.json`
        (&[("nord.json", NORD), ("bad.json", "{")], &["Nord"], &[bad]), // a broken file doesn't stop the rest
        (&[("bad.json", "")], &[], &[bad]),
        (&[("bad.json", r#"{"themes": 5}"#)], &[], &[bad]), // JSON, but not a theme set
        (&[("empty.json", "{}")], &[], &[]),                // a theme set with no themes
        (&[("two.json", TWO)], &["A", "B"], &[]),
        (
            &[("a.json", "["), ("b.json", "[")],
            &[],
            &[
                "Couldn't load theme a.json: ",
                "Couldn't load theme b.json: ",
            ],
        ),
        (&[], &[], &[]), // empty dir
    ];
    for (ix, (files, want_names, want_errors)) in cases.into_iter().enumerate() {
        let dir = test_dir(&format!("files-{ix}"));
        for (name, json) in files {
            std::fs::write(dir.join(name), json).unwrap();
        }
        let (themes, errors) = super::load_user_themes(&dir);
        assert_eq!(names(&themes), want_names, "themes from {files:?}");
        assert_eq!(
            errors.len(),
            want_errors.len(),
            "errors from {files:?}: {errors:?}"
        );
        for (got, want) in errors.iter().zip(want_errors) {
            assert!(
                got.starts_with(want) && got.len() > want.len(),
                "error {got:?} should be {want:?} + the parse error"
            );
        }
    }
    // A directory named like a theme file is not a file.
    let dir = test_dir("files-dir");
    std::fs::create_dir(dir.join("folder.json")).unwrap();
    let (themes, errors) = super::load_user_themes(&dir);
    assert_eq!(
        (names(&themes), errors),
        (vec![], vec![]),
        "a dir named folder.json"
    );
}

#[test]
fn user_theme_named_like_a_built_in_is_skipped_and_reported() {
    let set = |names: &[&str]| {
        let themes: Vec<String> = names
            .iter()
            .map(|n| format!(r#"{{"name": "{n}", "mode": "dark"}}"#))
            .collect();
        format!(r#"{{"name": "Set", "themes": [{}]}}"#, themes.join(","))
    };
    let clash =
        |theme: &str, file: &str| format!("Theme {theme} in {file} clashes with a built-in theme");
    // (file, themes in it, themes loaded, errors)
    let cases = [
        (
            "dark.json",
            vec!["QueryX Dark"],
            vec![],
            vec![clash("QueryX Dark", "dark.json")],
        ),
        (
            "light.json",
            vec!["QueryX Light"],
            vec![],
            vec![clash("QueryX Light", "light.json")],
        ),
        (
            "mixed.json",
            vec!["Nord", "QueryX Dark"],
            vec!["Nord"],
            vec![clash("QueryX Dark", "mixed.json")],
        ), // the rest of the file loads
        (
            "both.json",
            vec!["QueryX Dark", "QueryX Light"],
            vec![],
            vec![
                clash("QueryX Dark", "both.json"),
                clash("QueryX Light", "both.json"),
            ],
        ),
        (
            "case.json",
            vec!["queryx dark"],
            vec!["queryx dark"],
            vec![],
        ), // names match exactly
    ];
    for (ix, (file, inside, want_names, want_errors)) in cases.into_iter().enumerate() {
        let dir = test_dir(&format!("clash-{ix}"));
        std::fs::write(dir.join(file), set(&inside)).unwrap();
        let (themes, errors) = super::load_user_themes(&dir);
        assert_eq!(
            (names(&themes), errors),
            (
                want_names.iter().map(|n| n.to_string()).collect(),
                want_errors
            ),
            "{file} with {inside:?}"
        );
    }
}

/// Choosing a user theme applies it at once, in its own mode; choosing a built-in goes back.
#[gpui_kit::test]
fn choosing_a_user_theme_applies_it_immediately(cx: &mut gpui_kit::TestAppContext) {
    use super::ThemeChoice;
    use gpui_kit::component::ActiveTheme as _;
    let dir = test_dir("choose");
    std::fs::write(dir.join("nord.json"), NORD).unwrap();
    std::fs::write(dir.join("paper.json"), PAPER).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::init_themes(cx);
    });
    let (themes, _) = super::load_user_themes(&dir);
    let cx = cx.add_empty_window();
    // (choice, theme name after applying it, dark?, background)
    let cases = [
        (
            ThemeChoice::User("Nord".into()),
            "Nord",
            true,
            Some(0x2E3440),
        ),
        (ThemeChoice::Light, "QueryX Light", false, None),
        (
            ThemeChoice::User("Paper".into()),
            "Paper",
            false,
            Some(0xF7F3E8),
        ),
        (
            ThemeChoice::User("Nord".into()),
            "Nord",
            true,
            Some(0x2E3440),
        ), // light -> user dark
        (ThemeChoice::Dark, "QueryX Dark", true, None), // same mode, back to the built-in
        (ThemeChoice::User("Gone".into()), "QueryX Dark", true, None), // removed theme falls back to Dark
    ];
    for (choice, name, dark, bg) in cases {
        let (got, is_dark, background) = cx.update(|window, cx| {
            choice.apply(&themes, window, cx);
            (
                cx.theme().theme_name().to_string(),
                cx.theme().is_dark(),
                cx.theme().background,
            )
        });
        assert_eq!((got.as_str(), is_dark), (name, dark), "after {choice:?}");
        if let Some(bg) = bg {
            assert_eq!(
                background,
                gpui_kit::Hsla::from(super::hex(bg)),
                "background after {choice:?}"
            );
        }
    }
}

#[test]
fn theme_choice_cycles_through_all_four() {
    use super::ThemeChoice::*;
    assert_eq!(
        [
            Dark.next(),
            Light.next(),
            System.next(),
            Darcula.next(),
            User("Nord".into()).next()
        ],
        [Light, System, Darcula, Dark, Dark]
    );
}
