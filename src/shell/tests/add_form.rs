use super::support::*;
use super::*;

#[gpui_kit::test]
fn plus_button_or_add_connection_action_opens_the_dialog(cx: &mut gpui_kit::TestAppContext) {
    for via in ["action", "plus button"] {
        let (window, _shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        assert!(
            cx.debug_bounds("dialog-layer").is_none(),
            "{via}: no dialog before"
        );
        if via == "action" {
            cx.dispatch_action(crate::AddConnection);
        } else {
            let plus = cx.debug_bounds("add-connection");
            assert!(plus.is_some(), "explorer header has a + button");
            cx.simulate_click(plus.unwrap().center(), Default::default());
        }
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("dialog-layer").is_some(),
            "{via}: the dialog layer renders"
        );
    }
}

#[gpui_kit::test]
fn add_dialog_opens_with_the_spec_defaults(cx: &mut gpui_kit::TestAppContext) {
    use crate::model::{DbKind, SslMode};
    use crate::ui::connection_dialog::Form;
    let (window, _shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let dialog = open_add_dialog(&mut cx);
    // The mock workspace uses colors 0, 2 and 4.
    let want = Form {
        name: "".into(),
        color: 1,
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
    assert_eq!(dialog.read_with(&cx, |d, cx| d.form(cx)), want);
}

#[gpui_kit::test]
fn switching_to_sqlite_shows_only_an_empty_path_field(cx: &mut gpui_kit::TestAppContext) {
    use crate::model::DbKind;
    let (window, _shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let dialog = open_add_dialog(&mut cx);
    let postgres = [
        "field-host",
        "field-port",
        "field-user",
        "field-database",
        "field-ssl",
    ];
    // (click, kind, fields shown, fields hidden)
    let cases: [(&str, DbKind, &[&str], &[&str]); 2] = [
        ("kind-sqlite", DbKind::Sqlite, &["field-path"], &postgres),
        (
            "kind-postgres",
            DbKind::Postgres,
            &postgres,
            &["field-path"],
        ),
    ];
    for (button, kind, shown, hidden) in cases {
        click(&mut cx, button);
        let form = dialog.read_with(&cx, |d, cx| d.form(cx));
        assert_eq!((form.kind, form.path.as_str()), (kind, ""), "{button}");
        for field in shown {
            assert!(cx.debug_bounds(field).is_some(), "{button}: {field} shown");
        }
        for field in hidden {
            assert!(cx.debug_bounds(field).is_none(), "{button}: {field} hidden");
        }
    }
}

#[gpui_kit::test]
fn invalid_fields_show_their_error_and_keep_save_disabled(cx: &mut gpui_kit::TestAppContext) {
    let valid = [("name", "new"), ("host", "h"), ("user", "u")];
    let port = "Port must be a number from 1 to 65535";
    // (edits after `valid`, errors shown, Save enabled)
    let cases: Vec<(Vec<(&str, &str)>, Vec<(&str, &str)>, bool)> = vec![
        (vec![], vec![], true),
        (
            vec![("name", "prod-pg")],
            vec![("name", "Another connection is already named prod-pg")],
            false,
        ),
        (
            vec![("name", " prod-pg ")],
            vec![("name", "Another connection is already named prod-pg")],
            false,
        ),
        (vec![("name", "Prod-pg")], vec![], true),
        (
            vec![("name", "   ")],
            vec![("name", "Name can't be empty")],
            false,
        ),
        (
            vec![("name", "")],
            vec![("name", "Name can't be empty")],
            false,
        ),
        (
            vec![("host", "  ")],
            vec![("host", "Host can't be empty")],
            false,
        ),
        (
            vec![("user", "")],
            vec![("user", "User can't be empty")],
            false,
        ),
        (
            vec![("database", "")],
            vec![("database", "Database can't be empty")],
            false,
        ),
        (vec![("port", "0")], vec![("port", port)], false),
        (vec![("port", "65536")], vec![("port", port)], false),
        (vec![("port", "54a")], vec![("port", port)], false),
        (vec![("port", "")], vec![("port", port)], false),
        (vec![("port", " 65535 ")], vec![], true),
        (
            vec![("name", "prod-pg"), ("port", "x")],
            vec![
                ("name", "Another connection is already named prod-pg"),
                ("port", port),
            ],
            false,
        ),
        (
            vec![("kind-sqlite", ""), ("path", " ")],
            vec![("path", "Path can't be empty")],
            false,
        ),
        (
            vec![("kind-sqlite", ""), ("path", "/nope/x.db")],
            vec![],
            true,
        ),
        // Junk in a Postgres field doesn't block a SQLite save.
        (
            vec![("port", "x"), ("kind-sqlite", ""), ("path", "/nope/x.db")],
            vec![],
            true,
        ),
    ];
    let (window, _shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    for (edits, want_errors, want_save) in cases {
        cx.update(gpui_kit::component::WindowExt::close_all_dialogs);
        let dialog = open_add_dialog(&mut cx);
        fill(&mut cx, &dialog, &valid);
        fill(&mut cx, &dialog, &edits);
        let errors = dialog.read_with(&cx, |d, cx| d.errors(cx));
        let want: Vec<(&str, String)> = want_errors
            .iter()
            .map(|(f, m)| (*f, m.to_string()))
            .collect();
        assert_eq!(errors, want, "{edits:?}");
        for (field, _) in &want_errors {
            assert!(
                cx.debug_bounds(format!("error-{field}").leak()).is_some(),
                "{edits:?}: {field} error drawn"
            );
        }
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.save_enabled(cx)),
            want_save,
            "{edits:?}: Save enabled"
        );
    }
}

#[gpui_kit::test]
fn untouched_fields_show_no_error_yet(cx: &mut gpui_kit::TestAppContext) {
    let (window, _shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let dialog = open_add_dialog(&mut cx);
    fill(&mut cx, &dialog, &[("host", "h"), ("host", "")]);
    let errors = dialog.read_with(&cx, |d, cx| d.errors(cx));
    assert_eq!(
        errors,
        vec![("host", "Host can't be empty".to_string())],
        "only the field the user touched"
    );
    assert!(
        !dialog.read_with(&cx, |d, cx| d.save_enabled(cx)),
        "Save disabled: name, host and user are empty"
    );
}

#[gpui_kit::test]
fn enter_on_an_invalid_form_shows_every_error(cx: &mut gpui_kit::TestAppContext) {
    let (window, _shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let dialog = open_add_dialog(&mut cx);
    cx.simulate_keystrokes("enter");
    let fields: Vec<&str> = dialog
        .read_with(&cx, |d, cx| d.errors(cx))
        .into_iter()
        .map(|(f, _)| f)
        .collect();
    assert_eq!(
        fields,
        ["name", "host", "user"],
        "untouched fields show why Enter did nothing"
    );
}
