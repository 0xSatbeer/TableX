use super::support::*;
use super::*;

#[gpui_kit::test]
fn changing_a_connection_field_clears_the_test_line_and_name_color_or_production_keep_it(
    cx: &mut gpui_kit::TestAppContext,
) {
    let file = std::env::current_exe()
        .unwrap()
        .display()
        .to_string()
        .leak() as &str;
    let pg: &[(&str, &str)] = &[("name", "x"), ("host", "h"), ("user", "u")];
    let sqlite: &[(&str, &str)] = &[("name", "x"), ("kind-sqlite", ""), ("path", file)];
    // (form tested, then the change, line kept). "click <selector>" clicks; "ssl" picks the next SSL mode.
    let cases: Vec<(&[(&str, &str)], (&str, &str), bool)> = vec![
        (pg, ("host", "h2"), false),
        (pg, ("port", "5433"), false),
        (pg, ("user", "u2"), false),
        (pg, ("database", "d2"), false),
        (pg, ("ssl", ""), false),
        (pg, ("click", "kind-sqlite"), false),
        (sqlite, ("path", "/nope/x.db"), false),
        (sqlite, ("click", "kind-postgres"), false),
        (pg, ("name", "y"), true),
        (pg, ("name", "prod-pg"), true),
        (pg, ("click", "swatch-6"), true),
        (pg, ("click", "production"), true),
        (pg, ("click", "kind-postgres"), true),
        (sqlite, ("click", "kind-sqlite"), true),
    ];
    let (fake, window, _shell) = open_with_fake(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    for (form, change, kept) in cases {
        cx.update(gpui_kit::component::WindowExt::close_all_dialogs);
        let dialog = open_add_dialog(&mut cx);
        fill(&mut cx, &dialog, form);
        click(&mut cx, "dialog-test");
        answer_tests(&fake, Ok(()));
        cx.run_until_parked();
        let line = dialog.read_with(&cx, |d, cx| d.test_line(cx));
        assert!(line.is_some(), "{change:?}: line shown before the change");
        match change {
            ("click", selector) => click(&mut cx, selector),
            ("ssl", _) => {
                // The select sits under the field's label.
                let field = cx.debug_bounds("field-ssl").expect("SSL field drawn");
                cx.simulate_click(
                    gpui_kit::point(field.center().x, field.bottom() - gpui_kit::px(8.)),
                    Default::default(),
                );
                cx.run_until_parked();
                cx.simulate_keystrokes("down enter");
                cx.run_until_parked();
                assert_ne!(
                    dialog.read_with(&cx, |d, cx| d.form(cx).ssl),
                    crate::model::SslMode::Prefer,
                    "SSL changed"
                );
            }
            edit => fill(&mut cx, &dialog, &[edit]),
        }
        let want = if kept { line } else { None };
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.test_line(cx)),
            want,
            "{change:?}"
        );
        assert_eq!(
            cx.debug_bounds("test-result").is_some(),
            kept,
            "{change:?}: line drawn"
        );
    }
}

#[gpui_kit::test]
fn a_running_test_never_shows_its_result_after_a_field_change_save_or_close(
    cx: &mut gpui_kit::TestAppContext,
) {
    let form = [("name", "c"), ("host", "h"), ("user", "u")];
    // (what happens mid-test, then test again, line shown at the end, dialog open, connections added)
    let cases: [(&str, bool, Option<&str>, bool, usize); 10] = [
        ("host", false, None, true, 0),
        ("port", false, None, true, 0),
        ("click kind-sqlite", false, None, true, 0),
        (
            "host",
            true,
            Some("Could not connect: Could not resolve host db.invalid"),
            true,
            0,
        ),
        ("name", false, Some("Connection OK"), true, 0),
        ("click production", false, Some("Connection OK"), true, 0),
        ("click dialog-save", false, None, false, 1),
        ("enter", false, None, false, 1),
        ("escape", false, None, false, 0),
        ("click dialog-cancel", false, None, false, 0),
    ];
    for (step, again, line, open, added) in cases {
        let label = format!("{step} again {again}");
        let (fake, window, shell) = open_with_fake(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        let before = workspace.read_with(&cx, |ws, _| ws.connections.len());
        let dialog = open_add_dialog(&mut cx);
        fill(&mut cx, &dialog, &form);
        click(&mut cx, "dialog-test");
        match step {
            "host" => fill(&mut cx, &dialog, &[("host", "db.invalid")]),
            "port" => fill(&mut cx, &dialog, &[("port", "5433")]),
            "name" => fill(&mut cx, &dialog, &[("name", "d")]),
            _ => finish(&mut cx, &dialog, &[], step),
        }
        if open && line.is_none() {
            assert_eq!(
                dialog.read_with(&cx, |d, cx| d.test_button(cx).0),
                "Test connection",
                "{label}: no longer testing"
            );
        }
        if again {
            click(&mut cx, "dialog-test");
        }
        // A closed dialog's body is dropped with it, unless something else holds it.
        let held = if open || added > 0 {
            Some(dialog.clone())
        } else {
            None
        };
        let weak = dialog.downgrade();
        drop(dialog);
        answer_tests(
            &fake,
            if again {
                Err("Could not resolve host db.invalid".into())
            } else {
                Ok(())
            },
        );
        cx.run_until_parked();
        let shown = weak
            .upgrade()
            .and_then(|d| d.read_with(&cx, |d, cx| d.test_line(cx).map(|(text, _)| text)));
        assert_eq!(shown.as_deref(), line, "{label}: line");
        assert_eq!(
            cx.debug_bounds("test-result").is_some(),
            line.is_some(),
            "{label}: line drawn"
        );
        assert_eq!(
            cx.debug_bounds("dialog-layer").is_some(),
            open,
            "{label}: dialog open"
        );
        assert_eq!(
            workspace.read_with(&cx, |ws, _| ws.connections.len()),
            before + added,
            "{label}: connections added"
        );
        drop(held);
    }
}

#[gpui_kit::test]
fn save_works_with_no_test_run_in_the_add_and_edit_dialogs(cx: &mut gpui_kit::TestAppContext) {
    // (edit this row, or None to add; edits; press; the name saved)
    let cases: [(Option<&str>, &[(&str, &str)], &str, &str); 4] = [
        (
            None,
            &[("name", "c"), ("host", "db.invalid"), ("user", "u")],
            "click dialog-save",
            "c",
        ),
        (
            None,
            &[("name", "c"), ("kind-sqlite", ""), ("path", "/nope/x.db")],
            "enter",
            "c",
        ),
        (
            Some("prod-pg"),
            &[("name", "live")],
            "click dialog-save",
            "live",
        ),
        (
            Some("local.db"),
            &[("path", "/nope/x.db")],
            "enter",
            "local.db",
        ),
    ];
    for (row, edits, press, name) in cases {
        let label = format!("{row:?} {edits:?} {press}");
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        let dialog = match row {
            Some(row) => open_edit_dialog(&mut cx, &shell, row),
            None => open_add_dialog(&mut cx),
        };
        assert!(
            cx.debug_bounds("dialog-test").is_some(),
            "{label}: Test connection offered"
        );
        finish(&mut cx, &dialog, edits, press);
        assert!(
            cx.debug_bounds("dialog-layer").is_none(),
            "{label}: dialog closed"
        );
        assert!(
            workspace.read_with(&cx, |ws, _| ws.connections.iter().any(|c| c.name == name)),
            "{label}: saved"
        );
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.test_line(cx)),
            None,
            "{label}: no test line"
        );
    }
}
