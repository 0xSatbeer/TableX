use super::support::*;
use super::*;

#[gpui_kit::test]
fn in_demo_mode_expanding_notes_db_or_prod_pg_brings_the_sample_tree_from_the_mock_driver_after_the_latency(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionStatus;
    use std::time::Duration;
    crate::mock::set_latency(500);
    let (window, shell) = open_started(cx, crate::shell::Mode::Demo);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let (workspace, explorer) = shell.read_with(&cx, |this, _| {
        (this.workspace.clone(), this.explorer.clone())
    });
    // notes.db lives in the second sample workspace; add it here as a fresh connection.
    let mut notes = crate::mock::workspaces().remove(1).connections.remove(1);
    (notes.status, notes.databases) = (ConnectionStatus::Disconnected, None);
    workspace.update(&mut cx, |ws, cx| {
        ws.connections.push(notes);
        cx.notify();
    });
    cx.run_until_parked();
    let sample = |ws: usize, conn: usize| -> Vec<String> {
        crate::mock::workspaces()[ws].connections[conn]
            .databases
            .iter()
            .flatten()
            .map(|d| d.name.clone())
            .collect()
    };
    // (connection, its sample databases)
    let cases = [("prod-pg", sample(0, 0)), ("notes.db", sample(1, 1))];
    for (name, databases) in cases {
        let under = |cx: &gpui_kit::VisualTestContext| {
            let labels: Vec<String> = explorer.read_with(cx, |e, _| {
                e.row_labels().iter().map(|l| l.to_string()).collect()
            });
            let at = labels
                .iter()
                .position(|l| l == name)
                .unwrap_or_else(|| panic!("{name} row"));
            labels[at + 1..]
                .iter()
                .take_while(|l| {
                    !["prod-pg", "staging-pg", "local.db", "notes.db"].contains(&l.as_str())
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let row = explorer
            .read_with(&cx, |e, _| e.row_labels().iter().position(|l| l == name))
            .unwrap();
        let at = cx
            .debug_bounds(format!("tree-label-{row}").leak())
            .unwrap_or_else(|| panic!("{name} row drawn"))
            .center();
        cx.simulate_click(at, Default::default());
        cx.simulate_keystrokes("right");
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(499));
        cx.run_until_parked();
        assert!(
            !databases.iter().any(|d| under(&cx).contains(d)),
            "{name}: nothing before the latency, got {:?}",
            under(&cx)
        );
        // Each call (connect, then the listing) waits out the latency in turn.
        for _ in 0..4 {
            cx.executor().advance_clock(Duration::from_millis(500));
            cx.run_until_parked();
        }
        assert_eq!(under(&cx), databases, "{name}: the sample databases");
        // One level down, the first database's schemas come from the same sample tree.
        let first = &databases[0];
        let row = explorer
            .read_with(&cx, |e, _| e.row_labels().iter().position(|l| l == first))
            .unwrap();
        let at = cx
            .debug_bounds(format!("tree-label-{row}").leak())
            .unwrap_or_else(|| panic!("{first} row drawn"))
            .center();
        cx.simulate_click(at, Default::default());
        cx.simulate_keystrokes("right");
        for _ in 0..4 {
            cx.executor().advance_clock(Duration::from_millis(500));
            cx.run_until_parked();
        }
        let schemas: Vec<String> = crate::mock::workspaces()
            .into_iter()
            .flat_map(|ws| ws.connections)
            .find(|c| c.name == name)
            .unwrap()
            .databases
            .unwrap()[0]
            .schemas
            .iter()
            .flatten()
            .map(|s| s.name.clone())
            .collect();
        let shown = under(&cx);
        assert_eq!(
            &shown[1..=schemas.len()],
            &schemas[..],
            "{name}: {first}'s sample schemas"
        );
        let status = workspace.read_with(&cx, |ws, _| {
            ws.connections
                .iter()
                .find(|c| c.name == name)
                .unwrap()
                .status
                .clone()
        });
        assert_eq!(status, ConnectionStatus::Connected, "{name}");
    }
    crate::mock::set_latency(0);
}

#[gpui_kit::test]
fn connect_or_retry_in_the_status_bar_connects_the_active_tabs_connection_as_the_trees_retry_does(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionStatus;
    // (connection, its status, button, the connect the driver sees)
    let cases = [
        (
            "local.db",
            ConnectionStatus::Disconnected,
            "Connect",
            "connect local.db",
        ),
        (
            "staging-pg",
            ConnectionStatus::Error("timeout expired after 10 s".into()),
            "Retry",
            "connect app_db",
        ),
    ];
    for (name, status, button, connect) in cases {
        let fake = std::sync::Arc::new(crate::db::fake::Fake::default());
        let (window, shell) = open_plain_with(cx, fake.clone());
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
        shell.update_in(&mut cx, |this, window, cx| {
            let conn = this
                .workspace
                .read(cx)
                .connections
                .iter()
                .find(|c| c.name == name)
                .unwrap()
                .clone();
            assert_eq!(conn.status, status, "{name}");
            this.tabs
                .open(console_on(&conn, "console-x", false, window, cx));
            this.tab_changed(cx);
        });
        cx.run_until_parked();
        let selector: &'static str = format!("status-button:{button}").leak();
        click(&mut cx, selector);
        let status_of = |cx: &gpui_kit::VisualTestContext| {
            workspace.read_with(cx, |ws, _| {
                ws.connections
                    .iter()
                    .find(|c| c.name == name)
                    .unwrap()
                    .status
                    .clone()
            })
        };
        assert_eq!(
            status_of(&cx),
            ConnectionStatus::Connecting,
            "{name}: {button} starts a connect"
        );
        assert_eq!(fake.pending(), [connect], "{name}");
        assert!(
            cx.debug_bounds("status-text:Connecting…".to_string().leak())
                .is_some(),
            "{name}: the bar follows"
        );
        // As after the tree's Retry: once connected, the connection opens and lists its databases.
        fake.resolve(
            connect,
            Ok(crate::db::fake::session(
                connect.trim_start_matches("connect "),
            )),
        );
        cx.run_until_parked();
        assert_eq!(status_of(&cx), ConnectionStatus::Connected, "{name}");
        assert_eq!(
            fake.pending(),
            [format!(
                "databases {}",
                connect.trim_start_matches("connect ")
            )],
            "{name}"
        );
        assert!(
            cx.debug_bounds("tx-mode:AUTO-COMMIT").is_some(),
            "{name}: the console's mode"
        );
    }
}
