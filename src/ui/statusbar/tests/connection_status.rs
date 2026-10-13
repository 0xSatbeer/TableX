use super::*;

#[gpui_kit::test]
fn the_status_bar_shows_the_active_connections_status_by_dot_text_button_hint_and_auto_commit(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionStatus::{self, *};
    let pg_hint = "password authentication failed for user \"app\"\nCheck that db.acme.internal:5433 is reachable (VPN, firewall), then retry.";
    // (connection, status, solid dot, dot tooltip, text, in danger colour, button, hover hint, AUTO-COMMIT)
    let cases: [(
        usize,
        ConnectionStatus,
        bool,
        &str,
        Option<&str>,
        bool,
        Option<&str>,
        Option<&str>,
        bool,
    ); 9] = [
        (
            0,
            Connected,
            true,
            "Connected",
            None,
            false,
            None,
            None,
            true,
        ),
        (
            0,
            Connecting,
            false,
            "Connecting…",
            Some("Connecting…"),
            false,
            None,
            None,
            false,
        ),
        (
            0,
            Disconnected,
            false,
            "Disconnected",
            Some("Not connected"),
            false,
            Some("Connect"),
            None,
            false,
        ),
        (
            0,
            Error("password authentication failed for user \"app\"".into()),
            false,
            "Could not connect: password authentication failed for user \"app\"",
            Some("Could not connect"),
            true,
            Some("Retry"),
            Some(pg_hint),
            false,
        ),
        (
            1,
            Error("No file at /x/local.db".into()),
            false,
            "Could not connect: No file at /x/local.db",
            Some("Could not connect"),
            true,
            Some("Retry"),
            Some("No file at /x/local.db\nCheck that the file exists and is readable."),
            false,
        ),
        (
            1,
            Connected,
            true,
            "Connected",
            None,
            false,
            None,
            None,
            true,
        ),
        (
            1,
            Connecting,
            false,
            "Connecting…",
            Some("Connecting…"),
            false,
            None,
            None,
            false,
        ),
        (
            1,
            Disconnected,
            false,
            "Disconnected",
            Some("Not connected"),
            false,
            Some("Connect"),
            None,
            false,
        ),
        // Our own timeout reason leads the hint the same way.
        (
            0,
            Error("timeout expired after 10 s".into()),
            false,
            "Could not connect: timeout expired after 10 s",
            Some("Could not connect"),
            true,
            Some("Retry"),
            Some(
                "timeout expired after 10 s\nCheck that db.acme.internal:5433 is reachable (VPN, firewall), then retry.",
            ),
            false,
        ),
    ];
    for (ix, status, solid, tip, text, danger, button, hint, auto_commit) in cases {
        let (mut cx, bar, workspace) = open_bar(cx, 1200., false);
        let case = format!("{ix} {status:?}");
        show(&mut cx, &bar, &workspace, ix, status);
        assert!(
            drawn(
                &mut cx,
                format!("status-dot-{}", if solid { "solid" } else { "ring" })
            ),
            "{case}: dot shape"
        );
        for known in ["Connecting…", "Not connected", "Could not connect"] {
            let shown = text == Some(known);
            assert_eq!(
                drawn(&mut cx, format!("status-text:{known}")),
                shown && !danger,
                "{case}: text {known:?}"
            );
            assert_eq!(
                drawn(&mut cx, format!("status-text-danger:{known}")),
                shown && danger,
                "{case}: text {known:?} in danger colour"
            );
        }
        for known in ["Connect", "Retry"] {
            assert_eq!(
                drawn(&mut cx, format!("status-button:{known}")),
                button == Some(known),
                "{case}: button {known:?}"
            );
        }
        assert_eq!(
            drawn(&mut cx, "auto-commit".into()),
            auto_commit,
            "{case}: AUTO-COMMIT"
        );
        let dot = if solid {
            "status-dot-solid"
        } else {
            "status-dot-ring"
        };
        assert!(
            hover_shows(&mut cx, dot, format!("status-tip:{tip}")),
            "{case}: dot tooltip {tip:?}"
        );
        if let (Some(hint), Some(text)) = (hint, text) {
            assert!(
                hover_shows(
                    &mut cx,
                    &format!("status-text-danger:{text}"),
                    format!("status-hint:{hint}")
                ),
                "{case}: hint {hint:?}"
            );
        }
    }
}
