use super::support::*;
use super::*;

#[gpui_kit::test]
fn after_a_delete_the_selection_moves_to_the_row_that_took_its_place_or_the_previous_one(
    cx: &mut gpui_kit::TestAppContext,
) {
    // (rows expanded first, row deleted, row selected after). staging-pg always shows its error row.
    let cases: [(&[&str], &str, &str); 5] = [
        (&[], "staging-pg", "local.db"),
        (&[], "local.db", "staging-pg"),
        (&[], "prod-pg", "staging-pg"),
        // Its open subtree goes with it, so the next connection takes its place.
        (&["1/app_db/public/table"], "prod-pg", "staging-pg"),
        // The previous connection's row, not the last row above (prod-pg's open tables).
        (&["1/app_db/public/table"], "staging-pg", "local.db"),
    ];
    for (open, row, want) in cases {
        let label = format!("{open:?} delete {row}");
        let (window, shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
        for key in open {
            explorer.update_in(&mut cx, |e, window, cx| e.reveal(key, window, cx));
        }
        cx.run_until_parked();
        delete_row(&mut cx, &shell, row);
        assert_eq!(
            explorer.read_with(&cx, |e, _| e.selected_label()),
            Some(want.into()),
            "{label}"
        );
    }
    // Deleting the last of several: the previous row.
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
    for (row, want) in [("local.db", "staging-pg"), ("staging-pg", "prod-pg")] {
        delete_row(&mut cx, &shell, row);
        assert_eq!(
            explorer.read_with(&cx, |e, _| e.selected_label()),
            Some(want.into()),
            "delete {row} in turn"
        );
    }
}

#[gpui_kit::test]
fn after_a_delete_the_connections_below_keep_their_open_rows_and_their_tabs_point_at_them(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
    // Connect local.db, open its first table in a tab, and open prod-pg's tables above it.
    let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
    workspace.update(&mut cx, |ws, cx| {
        ws.connections[2].status = crate::model::ConnectionStatus::Connected;
        cx.notify();
    });
    cx.run_until_parked();
    explorer.update_in(&mut cx, |e, window, cx| {
        e.reveal("3/main/main/table/users", window, cx)
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    explorer.update_in(&mut cx, |e, window, cx| {
        e.reveal("1/app_db/public/table", window, cx)
    });
    cx.run_until_parked();
    let labels = |cx: &gpui_kit::VisualTestContext| explorer.read_with(cx, |e, _| e.row_labels());
    let under_local = |rows: Vec<gpui_kit::SharedString>| {
        rows.into_iter()
            .skip_while(|l| l != "local.db")
            .collect::<Vec<_>>()
    };
    let local_before = under_local(labels(&cx));
    assert!(
        local_before.len() > 3,
        "local.db is open down to its tables: {local_before:?}"
    );
    let tab_keys = |cx: &gpui_kit::VisualTestContext| {
        shell.read_with(cx, |this, _| {
            this.tabs
                .tabs
                .iter()
                .filter(|t| t.tooltip.is_some())
                .map(|t| t.key.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        tab_keys(&cx),
        ["3/main/main/table/users"],
        "local.db's table tab"
    );
    let table = shell.read_with(&cx, |this, _| {
        this.tabs
            .tabs
            .iter()
            .find(|t| t.key == "3/main/main/table/users")
            .unwrap()
            .title
            .clone()
    });

    delete_row(&mut cx, &shell, "prod-pg");

    assert_eq!(
        under_local(labels(&cx)),
        local_before,
        "local.db stays open as it was"
    );
    assert_eq!(
        tab_keys(&cx),
        ["3/main/main/table/users"],
        "its tab keeps its key"
    );
    explorer.update_in(&mut cx, |e, window, cx| {
        e.reveal("3/main/main/table/users", window, cx)
    });
    assert_eq!(
        explorer.read_with(&cx, |e, _| e.selected_label()),
        Some(table.clone()),
        "reveal in explorer finds the tab's table"
    );
    let open = shell.read_with(&cx, |this, _| this.tabs.tabs.len());
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        shell.read_with(&cx, |this, _| this.tabs.tabs.len()),
        open,
        "opening the table again activates its tab, no duplicate"
    );
}

#[gpui_kit::test]
fn deleting_a_connection_leaves_no_ctrl_clicked_row_marked_on_the_connection_that_moves_into_its_key(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_plain(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let (workspace, explorer) = shell.read_with(&cx, |this, _| {
        (this.workspace.clone(), this.explorer.clone())
    });
    workspace.update(&mut cx, |w, cx| {
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
    let at = |cx: &mut gpui_kit::VisualTestContext, name: &str| {
        let row = explorer
            .read_with(cx, |e, _| e.row_labels().iter().position(|l| l == name))
            .unwrap();
        cx.debug_bounds(format!("tree-label-{row}").leak())
            .unwrap_or_else(|| panic!("{name} row drawn"))
            .center()
    };
    // Select staging-pg, Ctrl+click local.db below it, delete staging-pg: new.db takes local.db's old key.
    let staging = at(&mut cx, "staging-pg");
    cx.simulate_click(staging, Default::default());
    let local = at(&mut cx, "local.db");
    cx.simulate_click(local, gpui_kit::Modifiers::secondary_key());
    cx.run_until_parked();
    cx.dispatch_action(crate::DeleteConnection);
    cx.run_until_parked();
    click(&mut cx, "delete-confirm");
    assert!(
        !explorer
            .read_with(&cx, |e, _| e.row_labels())
            .iter()
            .any(|l| l == "staging-pg"),
        "staging-pg deleted"
    );
    explorer.update_in(&mut cx, |e, window, cx| {
        window.focus(&gpui_kit::Focusable::focus_handle(e, cx), cx)
    });
    cx.simulate_keystrokes("ctrl-c");
    let copied = cx
        .read_from_clipboard()
        .and_then(|c| c.text())
        .unwrap_or_default();
    assert_eq!(
        copied, "local.db",
        "only the selected row is copied, not new.db"
    );
}
