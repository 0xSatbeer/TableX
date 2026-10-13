use super::*;

#[gpui_kit::test]
fn f2_rename_shows_in_the_tree_the_panel_and_that_connections_tabs_only(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::{ConnectionId, ConnectionStatus};
    let (window, shell) = open_with_connection_tabs(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let before = tab_origins(&shell, &cx);
    let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
    explorer.update_in(&mut cx, |e, window, cx| e.reveal("1", window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("f2");
    for _ in 0.."prod-pg".len() {
        cx.simulate_keystrokes("backspace");
    }
    cx.simulate_keystrokes("l i v e enter");
    cx.run_until_parked();

    assert_eq!(
        explorer.read_with(&cx, |e, _| e.row_labels()[0].clone()),
        "live",
        "tree row"
    );
    let panel = shell.read_with(&cx, |this, cx| {
        this.bottom_panel.read(cx).connection(ConnectionId(1), cx).0
    });
    assert_eq!(panel, "live", "bottom panel connection label");
    let conn = shell.read_with(&cx, |this, cx| {
        this.workspace.read(cx).connections[0].clone()
    });
    assert_eq!(
        (conn.name.as_str(), conn.status),
        ("live", ConnectionStatus::Connected),
        "shared workspace: renamed, still connected"
    );
    let after = tab_origins(&shell, &cx);
    assert_eq!(after.len(), before.len());
    for ((key, b), (_, a)) in before.iter().zip(&after) {
        let path = match b.path.strip_prefix("prod-pg") {
            Some(tail) if b.connection == ConnectionId(1) => format!("live{tail}"),
            _ => b.path.to_string(),
        };
        assert_eq!(
            a,
            &crate::shell::tabs::Origin {
                path: path.into(),
                ..b.clone()
            },
            "tab {key}"
        );
    }
}

#[gpui_kit::test]
fn recolor_and_production_from_outside_the_explorer_rerender_the_tree_the_panel_and_tabs(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionId;
    use std::{cell::Cell, rc::Rc};
    let (window, shell) = open_with_connection_tabs(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let (workspace, explorer, panel) = shell.read_with(&cx, |this, _| {
        (
            this.workspace.clone(),
            this.explorer.clone(),
            this.bottom_panel.clone(),
        )
    });
    let renders = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
    cx.update(|_, cx| {
        let hits = renders[0].clone();
        cx.observe(&explorer, move |_, _| hits.set(hits.get() + 1))
            .detach();
        let hits = renders[1].clone();
        cx.observe(&panel, move |_, _| hits.set(hits.get() + 1))
            .detach();
    });
    let before = tab_origins(&shell, &cx);
    // (change, new color, new production flag)
    let cases: [(&str, u8, bool); 3] = [
        ("recolor", 5, true),
        ("production off", 5, false),
        ("production on", 5, true),
    ];
    for (change, color, production) in cases {
        renders.iter().for_each(|r| r.set(0));
        workspace.update(&mut cx, |ws, cx| {
            let conn = &mut ws.connections[0];
            (conn.color, conn.is_production) = (color, production);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(renders[0].get() > 0, "{change}: explorer re-rendered");
        assert!(renders[1].get() > 0, "{change}: bottom panel re-rendered");
        assert_eq!(
            panel.read_with(&cx, |p, cx| p.connection(ConnectionId(1), cx)),
            ("prod-pg".to_string(), color, production),
            "{change}: panel row"
        );
        for ((key, b), (_, a)) in before.iter().zip(&tab_origins(&shell, &cx)) {
            let expected = if b.connection == ConnectionId(1) {
                crate::shell::tabs::Origin {
                    color,
                    production,
                    ..b.clone()
                }
            } else {
                b.clone()
            };
            assert_eq!(a, &expected, "{change}: tab {key} stripe and tint");
        }
    }
}
