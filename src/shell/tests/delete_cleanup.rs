use super::support::*;
use super::*;

#[gpui_kit::test]
fn deleting_the_connection_the_logs_are_filtered_to_clears_that_filter(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let panel = shell.read_with(&cx, |this, _| this.bottom_panel.clone());
    let staging = shell.read_with(&cx, |this, cx| {
        this.workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| c.name == "staging-pg")
            .unwrap()
            .id
    });
    let mut remaining = panel.read_with(&cx, |p, _| p.shown_connections());
    assert!(
        remaining.remove(&staging) && !remaining.is_empty(),
        "the log has rows for staging-pg and others"
    );
    panel.update(&mut cx, |p, cx| {
        p.show_logs();
        p.set_connection_filter(&[staging], cx);
    });
    assert_eq!(
        panel.read_with(&cx, |p, cx| p.connection_filter_label(cx)),
        "staging-pg"
    );
    delete_row(&mut cx, &shell, "staging-pg");
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(&cx, |p, _| p.shown_connections()),
        remaining,
        "the log shows the remaining connections' rows"
    );
    assert_eq!(
        panel.read_with(&cx, |p, cx| p.connection_filter_label(cx)),
        "All connections",
        "the filter no longer counts staging-pg"
    );
}

#[gpui_kit::test]
fn deleting_a_connection_with_no_log_rows_the_logs_are_filtered_to_clears_that_filter(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let (workspace, panel) = shell.read_with(&cx, |this, _| {
        (this.workspace.clone(), this.bottom_panel.clone())
    });
    let all = panel.read_with(&cx, |p, _| p.logged_connections());
    let fresh = workspace.update(&mut cx, |w, cx| {
        cx.notify();
        w.add(
            "new.db".into(),
            0,
            false,
            crate::model::ConnectionParams::Sqlite {
                path: "new.db".into(),
            },
        )
    });
    cx.run_until_parked();
    panel.update(&mut cx, |p, cx| {
        p.show_logs();
        p.set_connection_filter(&[fresh], cx);
    });
    assert!(
        panel
            .read_with(&cx, |p, _| p.shown_connections())
            .is_empty(),
        "new.db has no log rows"
    );
    delete_row(&mut cx, &shell, "new.db");
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(&cx, |p, cx| p.connection_filter_label(cx)),
        "All connections",
        "the filter no longer counts new.db"
    );
    assert_eq!(
        panel.read_with(&cx, |p, _| p.shown_connections()),
        all,
        "every remaining row is shown"
    );
}

#[gpui_kit::test]
fn deleting_the_last_connection_shows_no_connections_yet_with_an_add_connection_button(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (filter typed once the workspace is empty)
    for filter in ["", "zzz"] {
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let (workspace, explorer) = shell.read_with(&cx, |this, _| {
            (this.workspace.clone(), this.explorer.clone())
        });
        for row in ["local.db", "staging-pg"] {
            delete_row(&mut cx, &shell, row);
        }
        assert!(
            cx.debug_bounds("explorer-empty").is_none(),
            "{filter:?}: no empty state while one is left"
        );
        delete_row(&mut cx, &shell, "prod-pg");
        assert!(
            workspace.read_with(&cx, |ws, _| ws.connections.is_empty()),
            "{filter:?}: workspace []"
        );
        if !filter.is_empty() {
            explorer.update_in(&mut cx, |e, window, cx| e.focus_field(window, cx));
            cx.simulate_input(filter);
        }
        assert!(
            cx.debug_bounds("explorer-empty").is_some(),
            "{filter:?}: No connections yet"
        );
        let dialog = {
            use std::{cell::RefCell, rc::Rc};
            let opened = Rc::new(RefCell::new(None));
            let sink = opened.clone();
            let _watch = cx.update(|_, cx| {
                cx.observe_new(
                    move |_: &mut crate::ui::connection_dialog::ConnectionDialog, _, cx| {
                        *sink.borrow_mut() = Some(cx.entity())
                    },
                )
            });
            click(&mut cx, "empty-add-connection");
            opened
                .take()
                .unwrap_or_else(|| panic!("{filter:?}: Add connection opened a dialog"))
        };
        finish(
            &mut cx,
            &dialog,
            &[("host", "db.local"), ("user", "u"), ("name", "first")],
            "enter",
        );
        assert_eq!(
            workspace.read_with(&cx, |ws, _| ws
                .connections
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()),
            ["first"],
            "{filter:?}: the Add dialog added it"
        );
        assert!(
            cx.debug_bounds("explorer-empty").is_none(),
            "{filter:?}: the empty state gives way to the row"
        );
        assert_eq!(
            explorer.read_with(&cx, |e, _| e.selected_label()),
            Some("first".into()),
            "{filter:?}: new row selected"
        );
    }
}
