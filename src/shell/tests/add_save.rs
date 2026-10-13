use super::support::*;
use super::*;

#[gpui_kit::test]
fn saving_a_valid_form_appends_the_connection_and_selects_its_row(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::{Connection, ConnectionParams, ConnectionStatus, SslMode};
    use std::path::PathBuf;
    let postgres = ConnectionParams::Postgres {
        host: "db.local".into(),
        port: 5433,
        user: "u".into(),
        password: None,
        database: "d".into(),
        ssl: SslMode::Prefer,
    };
    // (empty the workspace first, edits, clicks, want name, color, production, params)
    let cases: Vec<(
        bool,
        Vec<(&str, &str)>,
        Vec<&'static str>,
        &str,
        u8,
        bool,
        ConnectionParams,
    )> = vec![
        (
            false,
            vec![
                ("name", " c "),
                ("host", " db.local "),
                ("port", " 5433 "),
                ("user", " u "),
                ("database", " d "),
            ],
            vec![],
            "c",
            1,
            false,
            postgres.clone(),
        ),
        (
            false,
            vec![
                ("name", "c"),
                ("host", "db.local"),
                ("port", "5433"),
                ("user", "u"),
                ("database", "d"),
            ],
            vec!["swatch-6", "production"],
            "c",
            6,
            true,
            postgres.clone(),
        ),
        (
            false,
            vec![
                ("kind-sqlite", ""),
                ("name", "notes"),
                ("path", " /tmp/notes.db "),
            ],
            vec![],
            "notes",
            1,
            false,
            ConnectionParams::Sqlite {
                path: PathBuf::from("/tmp/notes.db"),
            },
        ),
        (
            true,
            vec![
                ("name", "c"),
                ("host", "db.local"),
                ("port", "5433"),
                ("user", "u"),
                ("database", "d"),
            ],
            vec![],
            "c",
            0,
            false,
            postgres.clone(),
        ),
    ];
    for (empty, edits, clicks, name, color, production, params) in cases {
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let (workspace, explorer, panel) = shell.read_with(&cx, |this, _| {
            (
                this.workspace.clone(),
                this.explorer.clone(),
                this.bottom_panel.clone(),
            )
        });
        if empty {
            workspace.update(&mut cx, |ws, cx| {
                ws.connections.clear();
                cx.notify();
            });
        }
        let before = workspace.read_with(&cx, |ws, _| ws.connections.clone());
        let dialog = open_add_dialog(&mut cx);
        fill(&mut cx, &dialog, &edits);
        for selector in clicks {
            click(&mut cx, selector);
        }
        click(&mut cx, "dialog-save");

        let after = workspace.read_with(&cx, |ws, _| ws.connections.clone());
        assert_eq!(
            after.len(),
            before.len() + 1,
            "{name}: one connection added"
        );
        assert_eq!(
            after[..before.len()],
            before[..],
            "{name}: the others unchanged, in order"
        );
        let added = after.last().unwrap();
        assert!(
            !before.iter().any(|c| c.id == added.id),
            "{name}: fresh id {:?}",
            added.id
        );
        let want = Connection {
            id: added.id,
            name: name.into(),
            color,
            is_production: production,
            status: ConnectionStatus::Disconnected,
            params,
            databases: None,
        };
        assert_eq!(
            added, &want,
            "{name}: trimmed values, Disconnected, no databases"
        );
        assert_eq!(
            explorer.read_with(&cx, |e, _| e.row_labels().last().cloned()),
            Some(name.into()),
            "{name}: last tree row"
        );
        assert_eq!(
            explorer.read_with(&cx, |e, _| e.selected_label()),
            Some(name.into()),
            "{name}: new row selected"
        );
        assert_eq!(
            panel.read_with(&cx, |p, cx| p.connection(added.id, cx).0),
            name,
            "{name}: bottom panel knows it"
        );
        assert!(
            cx.debug_bounds("dialog-layer").is_none(),
            "{name}: dialog closed"
        );
    }
}

#[gpui_kit::test]
fn enter_saves_a_valid_form_and_esc_or_cancel_close_without_changes(
    cx: &mut gpui_kit::TestAppContext,
) {
    let valid = [("host", "db.local"), ("user", "u"), ("name", "c")];
    let invalid = [("host", "db.local"), ("user", "u"), ("name", "prod-pg")];
    // (edits, then press, dialog still open, connections added)
    let cases: [(&[(&str, &str)], &str, bool, usize); 5] = [
        (&valid, "enter", false, 1),
        (&invalid, "enter", true, 0),
        (&valid, "escape", false, 0),
        (&invalid, "escape", false, 0),
        (&valid, "click dialog-cancel", false, 0),
    ];
    for (edits, press, open, added) in cases {
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        let before = workspace.read_with(&cx, |ws, _| ws.clone());
        let dialog = open_add_dialog(&mut cx);
        fill(&mut cx, &dialog, edits);
        match press.strip_prefix("click ") {
            Some(selector) => click(&mut cx, selector),
            None => cx.simulate_keystrokes(press),
        }
        cx.run_until_parked();
        let label = format!("{:?} {press}", edits.last().unwrap());
        assert_eq!(
            cx.debug_bounds("dialog-layer").is_some(),
            open,
            "{label}: dialog open"
        );
        let after = workspace.read_with(&cx, |ws, _| ws.clone());
        assert_eq!(
            after.connections.len(),
            before.connections.len() + added,
            "{label}: connections added"
        );
        if added == 0 {
            assert_eq!(after, before, "{label}: workspace unchanged");
        }
    }
}
