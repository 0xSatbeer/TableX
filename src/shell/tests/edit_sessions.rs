use super::support::*;
use super::*;

#[gpui_kit::test]
fn edit_keeps_tabs_open_and_disconnects_only_on_a_params_change(cx: &mut gpui_kit::TestAppContext) {
    use crate::model::{ConnectionId, ConnectionStatus};
    // (row, edits, clicks, stays Connected, then name, color, production)
    let cases: Vec<(
        &str,
        Vec<(&str, &str)>,
        Vec<&'static str>,
        bool,
        &str,
        u8,
        bool,
    )> = vec![
        (
            "prod-pg",
            vec![("name", "live")],
            vec![],
            true,
            "live",
            0,
            true,
        ),
        (
            "prod-pg",
            vec![],
            vec!["swatch-5"],
            true,
            "prod-pg",
            5,
            true,
        ),
        (
            "prod-pg",
            vec![],
            vec!["production"],
            true,
            "prod-pg",
            0,
            false,
        ),
        (
            "prod-pg",
            vec![("host", "  db.acme.internal ")],
            vec![],
            true,
            "prod-pg",
            0,
            true,
        ),
        (
            "prod-pg",
            vec![("port", " 5432 ")],
            vec![],
            true,
            "prod-pg",
            0,
            true,
        ),
        (
            "prod-pg",
            vec![("name", "live"), ("port", "5433")],
            vec![],
            false,
            "live",
            0,
            true,
        ),
        (
            "prod-pg",
            vec![],
            vec!["ssl Require"],
            false,
            "prod-pg",
            0,
            true,
        ),
        (
            "prod-pg",
            vec![("host", "db2.acme.internal")],
            vec![],
            false,
            "prod-pg",
            0,
            true,
        ),
        (
            "prod-pg",
            vec![("user", "admin")],
            vec![],
            false,
            "prod-pg",
            0,
            true,
        ),
        (
            "prod-pg",
            vec![("database", "other_db")],
            vec![],
            false,
            "prod-pg",
            0,
            true,
        ),
        (
            "prod-pg",
            vec![("name", "live")],
            vec!["swatch-5", "production"],
            true,
            "live",
            5,
            false,
        ),
        (
            "local.db",
            vec![("path", "~/acme/other.db")],
            vec![],
            false,
            "local.db",
            4,
            false,
        ),
        (
            "local.db",
            vec![("path", " ~/acme/local.db ")],
            vec![],
            true,
            "local.db",
            4,
            false,
        ),
        (
            "local.db",
            vec![],
            vec!["production"],
            true,
            "local.db",
            4,
            true,
        ),
    ];
    for (row, edits, clicks, stays, name, color, production) in cases {
        let label = format!("{row} {edits:?} {clicks:?}");
        let (window, shell) = open_with_connection_tabs(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        let ci = workspace.update(&mut cx, |ws, cx| {
            let ci = ws.connections.iter().position(|c| c.name == row).unwrap();
            ws.connections[ci].status = ConnectionStatus::Connected;
            cx.notify();
            ci
        });
        let id = workspace.read_with(&cx, |ws, _| ws.connections[ci].id);
        let had_databases = workspace.read_with(&cx, |ws, _| {
            ws.connections[ci].databases.as_ref().map_or(0, Vec::len)
        });
        assert!(had_databases > 0, "{label}: fixture has databases");
        let before = tab_origins(&shell, &cx);
        let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
        // Scrolls the row into view; prod-pg is open down to a table.
        explorer.update_in(&mut cx, |e, window, cx| {
            e.reveal(&id.0.to_string(), window, cx)
        });
        cx.run_until_parked();
        let dialog = open_edit_dialog(&mut cx, &shell, row);
        fill(&mut cx, &dialog, &edits);
        for selector in &clicks {
            match selector.strip_prefix("ssl ") {
                Some("Require") => {
                    let ssl = dialog.read_with(&cx, |d, _| d.ssl.clone());
                    ssl.update_in(&mut cx, |s, window, cx| {
                        s.set_selected_index(
                            Some(gpui_kit::component::IndexPath::new(2)),
                            window,
                            cx,
                        )
                    });
                    cx.run_until_parked();
                }
                _ => click(&mut cx, selector),
            }
        }
        click(&mut cx, "dialog-save");

        let conn = workspace.read_with(&cx, |ws, _| ws.connections[ci].clone());
        assert_eq!(
            (conn.name.as_str(), conn.color, conn.is_production),
            (name, color, production),
            "{label}: saved in place"
        );
        let (status, databases) = if stays {
            (ConnectionStatus::Connected, had_databases)
        } else {
            (ConnectionStatus::Disconnected, 0)
        };
        assert_eq!(
            (conn.status, conn.databases.as_ref().map_or(0, Vec::len)),
            (status, databases),
            "{label}: status and databases"
        );
        let after = tab_origins(&shell, &cx);
        assert_eq!(after.len(), before.len(), "{label}: tab count unchanged");
        for ((key, b), (_, a)) in before.iter().zip(&after) {
            let want = if b.connection == id {
                crate::shell::tabs::Origin {
                    path: crate::shell::tabs::join_path(name, &b.tail),
                    color,
                    production,
                    ..b.clone()
                }
            } else {
                b.clone()
            };
            assert_eq!(a, &want, "{label}: tab {key}");
        }
        assert!(
            after.iter().any(|(_, o)| o.connection == ConnectionId(1)),
            "{label}: prod-pg tabs still open"
        );
        let row = explorer
            .read_with(&cx, |e, _| e.row_labels().iter().position(|l| l == name))
            .unwrap();
        assert_eq!(
            cx.debug_bounds(format!("prod-badge-{row}").leak())
                .is_some(),
            production,
            "{label}: PROD badge in the tree"
        );
        if !stays {
            // Disconnected by the edit, it connects again from its row.
            explorer.update_in(&mut cx, |e, window, cx| {
                e.reveal(&id.0.to_string(), window, cx)
            });
            cx.simulate_keystrokes("enter");
            let status = workspace.read_with(&cx, |ws, _| ws.connections[ci].status.clone());
            assert_eq!(
                status,
                ConnectionStatus::Connecting,
                "{label}: Enter reconnects"
            );
        }
    }
}
