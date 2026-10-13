use super::support::*;
use super::*;

#[gpui_kit::test]
fn the_edit_dialog_prefills_the_saved_password_masked_and_test_uses_it(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::{ConnectionParams, Secret};
    let fake = std::sync::Arc::new(crate::db::fake::Fake::default());
    let (window, shell) = open_plain_with(cx, fake.clone());
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
    workspace.update(&mut cx, |ws, _| {
        let conn = ws
            .connections
            .iter_mut()
            .find(|c| c.name == "prod-pg")
            .unwrap();
        let ConnectionParams::Postgres { password, .. } = &mut conn.params else {
            panic!("prod-pg is Postgres")
        };
        *password = Some(Secret::new("s3cret"));
    });
    let dialog = open_edit_dialog(&mut cx, &shell, "prod-pg");
    assert_eq!(
        dialog.read_with(&cx, |d, cx| d.form(cx).password),
        "s3cret",
        "pre-filled"
    );
    assert!(
        dialog.read_with(&cx, |d, cx| d.password.read(cx).presentation().is_masked()),
        "masked"
    );
    click(&mut cx, "dialog-test");
    let tested = fake.tested();
    assert_eq!(tested.len(), 1, "Test went through the driver");
    assert!(
        matches!(&tested[0], ConnectionParams::Postgres { password: Some(p), .. } if p.expose() == "s3cret"),
        "{tested:?}"
    );
}

#[gpui_kit::test]
fn testing_a_connected_connection_leaves_its_status_and_sessions_alone(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionStatus::Connected;
    // (the driver's answer to Test)
    let cases: [Result<(), String>; 2] = [Ok(()), Err("password authentication failed".into())];
    for answer in cases {
        let (fake, window, shell) = open_with_fake(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        workspace.update(&mut cx, |ws, _| {
            ws.connections
                .iter_mut()
                .find(|c| c.name == "prod-pg")
                .unwrap()
                .status = Connected
        });
        let dialog = open_edit_dialog(&mut cx, &shell, "prod-pg");
        click(&mut cx, "dialog-test");
        assert_eq!(
            fake.pending().len(),
            1,
            "{answer:?}: only the test call, no connect or disconnect"
        );
        let status = |cx: &gpui_kit::VisualTestContext| {
            workspace.read_with(cx, |ws, _| {
                ws.connections
                    .iter()
                    .find(|c| c.name == "prod-pg")
                    .unwrap()
                    .status
                    .clone()
            })
        };
        assert_eq!(status(&cx), Connected, "{answer:?}: while testing");
        answer_tests(&fake, answer.clone());
        cx.run_until_parked();
        assert_eq!(status(&cx), Connected, "{answer:?}: after the result");
        assert!(
            fake.pending().is_empty(),
            "{answer:?}: nothing else was called"
        );
        assert!(
            dialog.read_with(&cx, |d, cx| d.test_line(cx)).is_some(),
            "{answer:?}: result shown"
        );
    }
}

#[gpui_kit::test]
fn test_connection_is_enabled_only_while_the_connection_fields_are_valid(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (edits, Test connection enabled)
    let cases: Vec<(Vec<(&str, &str)>, bool)> = vec![
        (vec![], false),
        (vec![("host", "h")], false),
        (vec![("host", "h"), ("user", "u")], true),
        (
            vec![("host", "h"), ("user", "u"), ("name", "prod-pg")],
            true,
        ),
        (vec![("host", "h"), ("user", "u"), ("name", "  ")], true),
        (vec![("host", "h"), ("user", "u"), ("name", "new")], true),
        (
            vec![("host", "h"), ("user", "u"), ("name", "new"), ("port", "0")],
            false,
        ),
        (
            vec![("host", "h"), ("user", "u"), ("port", " 65535 ")],
            true,
        ),
        (vec![("host", "h"), ("user", "u"), ("database", " ")], false),
        (vec![("kind-sqlite", "")], false),
        (vec![("kind-sqlite", ""), ("path", "/nope/x.db")], true),
        (
            vec![
                ("kind-sqlite", ""),
                ("path", "/nope/x.db"),
                ("name", "prod-pg"),
            ],
            true,
        ),
    ];
    let (window, _shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    for (edits, enabled) in cases {
        cx.update(gpui_kit::component::WindowExt::close_all_dialogs);
        let dialog = open_add_dialog(&mut cx);
        fill(&mut cx, &dialog, &edits);
        assert!(
            cx.debug_bounds("dialog-test").is_some(),
            "{edits:?}: Test connection drawn"
        );
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.test_button(cx)),
            ("Test connection", enabled),
            "{edits:?}"
        );
    }
}

#[gpui_kit::test]
fn test_connection_reads_testing_while_it_runs_then_shows_the_outcome_line(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::component::ActiveTheme as _;
    let pg = vec![("host", "db.local"), ("user", "u")];
    let sqlite = |path: &'static str| vec![("kind-sqlite", ""), ("path", path)];
    let wrong = r#"password authentication failed for user "u""#;
    // (edits, driver's answer, result line, success)
    let cases: Vec<(Vec<(&str, &str)>, Result<(), String>, String, bool)> = vec![
        (pg.clone(), Ok(()), "Connection OK".into(), true),
        (
            pg,
            Err(wrong.into()),
            format!("Could not connect: {wrong}"),
            false,
        ),
        (sqlite("/data/app.db"), Ok(()), "Connection OK".into(), true),
        (
            sqlite(" /data/app.db "),
            Ok(()),
            "Connection OK".into(),
            true,
        ),
        (
            sqlite("/nope/x.db"),
            Err("No file at /nope/x.db".into()),
            "Could not connect: No file at /nope/x.db".into(),
            false,
        ),
        (
            sqlite("/data/notes.txt"),
            Err("file is not a database".into()),
            "Could not connect: file is not a database".into(),
            false,
        ),
    ];
    let (fake, window, _shell) = open_with_fake(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let (success, danger) = cx.update(|_, cx| (cx.theme().success, cx.theme().danger));
    for (edits, answer, line, ok) in cases {
        cx.update(gpui_kit::component::WindowExt::close_all_dialogs);
        let dialog = open_add_dialog(&mut cx);
        fill(&mut cx, &dialog, &edits);
        click(&mut cx, "dialog-test");
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.test_button(cx)),
            ("Testing…", false),
            "{edits:?}: running"
        );
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.test_line(cx)),
            None,
            "{edits:?}: no line yet"
        );
        answer_tests(&fake, answer);
        cx.run_until_parked();
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.test_button(cx)),
            ("Test connection", true),
            "{edits:?}: finished"
        );
        assert_eq!(
            dialog.read_with(&cx, |d, cx| d.test_line(cx)),
            Some((line, if ok { success } else { danger })),
            "{edits:?}"
        );
        assert!(
            cx.debug_bounds("test-result").is_some(),
            "{edits:?}: line drawn"
        );
    }
}
