use super::support::*;
use super::*;

#[gpui_kit::test]
fn right_click_edit_opens_the_dialog_prefilled_with_the_type_locked(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::{DbKind, SslMode};
    use crate::ui::connection_dialog::Form;
    let blank = Form {
        name: "".into(),
        color: 0,
        is_production: false,
        kind: DbKind::Postgres,
        host: "".into(),
        port: "5432".into(),
        user: "".into(),
        password: "".into(),
        database: "postgres".into(),
        ssl: SslMode::Prefer,
        path: "".into(),
    };
    // (row, its form, the other type's button)
    let cases = [
        (
            "prod-pg",
            Form {
                name: "prod-pg".into(),
                color: 0,
                is_production: true,
                host: "db.acme.internal".into(),
                user: "app".into(),
                database: "app_db".into(),
                ..blank.clone()
            },
            "kind-sqlite",
        ),
        (
            "staging-pg",
            Form {
                name: "staging-pg".into(),
                color: 2,
                host: "staging.acme.internal".into(),
                user: "app".into(),
                database: "app_db".into(),
                ..blank.clone()
            },
            "kind-sqlite",
        ),
        (
            "local.db",
            Form {
                name: "local.db".into(),
                color: 4,
                kind: DbKind::Sqlite,
                path: "~/acme/local.db".into(),
                ..blank.clone()
            },
            "kind-postgres",
        ),
    ];
    for (row, want, other_kind) in cases {
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let dialog = open_edit_dialog(&mut cx, &shell, row);
        assert!(
            cx.debug_bounds("dialog-layer").is_some(),
            "{row}: dialog shown"
        );
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.form(cx)),
            want,
            "{row}: prefilled"
        );
        click(&mut cx, other_kind);
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.form(cx).kind),
            want.kind,
            "{row}: type locked"
        );
    }
}

#[gpui_kit::test]
fn edit_name_validation_excludes_the_connection_being_edited(cx: &mut gpui_kit::TestAppContext) {
    // (row, name typed or None to leave it, error shown, Save enabled)
    let cases: [(&str, Option<&str>, Option<&str>, bool); 8] = [
        ("prod-pg", None, None, true),
        ("prod-pg", Some("prod-pg"), None, true),
        ("prod-pg", Some(" prod-pg "), None, true),
        (
            "prod-pg",
            Some("staging-pg"),
            Some("Another connection is already named staging-pg"),
            false,
        ),
        (
            "prod-pg",
            Some(" local.db "),
            Some("Another connection is already named local.db"),
            false,
        ),
        ("prod-pg", Some("Staging-pg"), None, true),
        ("prod-pg", Some("  "), Some("Name can't be empty"), false),
        ("local.db", Some("local.db"), None, true),
    ];
    for (row, name, error, save) in cases {
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let dialog = open_edit_dialog(&mut cx, &shell, row);
        if let Some(name) = name {
            fill(&mut cx, &dialog, &[("name", name)]);
        }
        let errors = dialog.read_with(&cx, |d, cx| d.errors(cx));
        let want: Vec<(&str, String)> =
            error.map(|e| ("name", e.to_string())).into_iter().collect();
        assert_eq!(errors, want, "{row} {name:?}");
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.save_enabled(cx)),
            save,
            "{row} {name:?}: Save enabled"
        );
    }
}

#[gpui_kit::test]
fn edit_with_no_changes_or_cancelled_leaves_the_workspace_as_it_was(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionStatus;
    // (row, edits, press, the name it ends with)
    let cases: [(&str, &[(&str, &str)], &str, &str); 7] = [
        ("prod-pg", &[], "enter", "prod-pg"),
        ("prod-pg", &[], "click dialog-save", "prod-pg"),
        ("prod-pg", &[("name", " prod-pg ")], "enter", "prod-pg"),
        (
            "prod-pg",
            &[("name", "live"), ("port", "5433")],
            "escape",
            "prod-pg",
        ),
        (
            "prod-pg",
            &[("name", "live"), ("port", "5433")],
            "click dialog-cancel",
            "prod-pg",
        ),
        ("local.db", &[], "enter", "local.db"),
        ("staging-pg", &[("name", "staging-2")], "enter", "staging-2"),
    ];
    for (row, edits, press, name) in cases {
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        let mut want = workspace.read_with(&cx, |ws, _| ws.clone());
        want.connections
            .iter_mut()
            .find(|c| c.name == row)
            .unwrap()
            .name = name.into();
        let dialog = open_edit_dialog(&mut cx, &shell, row);
        finish(&mut cx, &dialog, edits, press);
        let label = format!("{row} {edits:?} {press}");
        assert!(
            cx.debug_bounds("dialog-layer").is_none(),
            "{label}: dialog closed"
        );
        assert_eq!(
            workspace.read_with(&cx, |ws, _| ws.clone()),
            want,
            "{label}: workspace"
        );
        if row == "staging-pg" {
            let status = workspace.read_with(&cx, |ws, _| ws.connections[1].status.clone());
            assert!(
                matches!(status, ConnectionStatus::Error(_)),
                "{label}: still Error"
            );
        }
    }
}

#[gpui_kit::test]
fn after_an_edit_the_selection_stays_on_that_connection(cx: &mut gpui_kit::TestAppContext) {
    // (filter typed first, row, edits, its label after)
    let cases: [(&str, &str, &[(&str, &str)], &str); 5] = [
        ("", "prod-pg", &[("name", "live")], "live"),
        ("", "staging-pg", &[("name", "z")], "z"),
        ("", "local.db", &[("path", "~/acme/other.db")], "local.db"),
        ("", "prod-pg", &[("port", "5433")], "prod-pg"),
        // The filter would hide the renamed row, so it is cleared.
        ("local", "local.db", &[("name", "z.db")], "z.db"),
    ];
    for (filter, row, edits, label) in cases {
        let (window, shell) = open_with_connection_tabs(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
        explorer.update_in(&mut cx, |e, window, cx| e.reveal("1", window, cx));
        cx.run_until_parked();
        if !filter.is_empty() {
            cx.simulate_input(filter);
        }
        let dialog = open_edit_dialog(&mut cx, &shell, row);
        finish(&mut cx, &dialog, edits, "enter");
        assert_eq!(
            explorer.read_with(&cx, |e, _| e.selected_label()),
            Some(label.into()),
            "{filter:?} {row} {edits:?}"
        );
    }
}

#[gpui_kit::test]
fn the_postgres_form_shows_a_masked_password_field_whose_text_reaches_the_params(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::Secret;
    let (window, _shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let dialog = open_add_dialog(&mut cx);
    assert!(
        cx.debug_bounds("field-password").is_some(),
        "Postgres: Password field drawn"
    );
    assert!(
        dialog.read_with(&cx, |d, cx| d.password.read(cx).presentation().is_masked()),
        "masked"
    );
    fill(
        &mut cx,
        &dialog,
        &[("host", "h"), ("user", "u"), ("password", "hunter2")],
    );
    let password = dialog.read_with(&cx, |d, cx| match d.form(cx).params() {
        Ok(crate::model::ConnectionParams::Postgres { password, .. }) => password,
        other => panic!("{other:?}"),
    });
    assert_eq!(password, Some(Secret::new("hunter2")));
    click(&mut cx, "kind-sqlite");
    assert!(
        cx.debug_bounds("field-password").is_none(),
        "SQLite: no Password field"
    );
}
