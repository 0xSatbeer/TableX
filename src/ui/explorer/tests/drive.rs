//! Helpers that drive the explorer under test and answer or read back what it shows.

use super::*;

pub(super) fn status_of(
    workspace: &gpui_kit::Entity<crate::model::Workspace>,
    cx: &gpui_kit::VisualTestContext,
) -> ConnectionStatus {
    workspace.read_with(cx, |ws, _| ws.connections[0].status.clone())
}

pub(super) fn labels(
    explorer: &gpui_kit::Entity<super::Explorer>,
    cx: &gpui_kit::VisualTestContext,
) -> Vec<String> {
    explorer.read_with(cx, |e, _| {
        e.row_labels().iter().map(|l| l.to_string()).collect()
    })
}

/// Expands the first row, the connection.
pub(super) fn expand_connection(
    explorer: &gpui_kit::Entity<super::Explorer>,
    cx: &mut gpui_kit::VisualTestContext,
) {
    explorer.update_in(cx, |e, _, cx| e.set_expanded(0, true, cx));
}

/// Expands the connection and answers its connect and databases; returns a handle that dies with the session.
pub(super) fn connect_answered(
    explorer: &gpui_kit::Entity<super::Explorer>,
    fake: &crate::db::fake::Fake,
    cx: &mut gpui_kit::VisualTestContext,
    databases: &[&str],
) -> std::sync::Weak<crate::db::fake::FakeSession> {
    expand_connection(explorer, cx);
    let session = crate::db::fake::session("app");
    let crate::db::Session::Fake(alive) = &session else {
        unreachable!()
    };
    let alive = Arc::downgrade(alive);
    fake.resolve("connect app", Ok(session));
    cx.run_until_parked();
    fake.resolve(
        "databases app",
        Ok(databases.iter().map(|d| d.to_string()).collect::<Vec<_>>()),
    );
    cx.run_until_parked();
    alive
}

/// Expands the row labelled `label`.
pub(super) fn expand(
    explorer: &gpui_kit::Entity<super::Explorer>,
    cx: &mut gpui_kit::VisualTestContext,
    label: &str,
) {
    explorer.update_in(cx, |e, _, cx| {
        let ix = e
            .nodes
            .iter()
            .position(|n| n.label.as_ref() == label)
            .unwrap_or_else(|| panic!("{label} row"));
        e.set_expanded(ix, true, cx);
    });
    cx.run_until_parked();
}

/// The labels of the rows under the row labelled `label`.
pub(super) fn under(
    explorer: &gpui_kit::Entity<super::Explorer>,
    cx: &gpui_kit::VisualTestContext,
    label: &str,
) -> Vec<String> {
    explorer.read_with(cx, |e, _| {
        let ix = e
            .nodes
            .iter()
            .position(|n| n.label.as_ref() == label)
            .unwrap_or_else(|| panic!("{label} row"));
        e.nodes[ix + 1..]
            .iter()
            .take_while(|n| n.depth > e.nodes[ix].depth)
            .map(|n| n.label.to_string())
            .collect()
    })
}

/// Answers the pending call `label` with a sample of its level, or with `Err(reason)`.
pub(super) fn answer(fake: &crate::db::fake::Fake, label: &str, result: Result<(), &str>) {
    let err = |reason: &str| reason.to_string();
    match label.split(' ').next() {
        Some("schemas") => fake.resolve(
            label,
            result
                .map(|()| strings(&["public", "sibling"]))
                .map_err(err),
        ),
        Some("objects") => fake.resolve(
            label,
            result.map(|()| vec![table("t"), table("u")]).map_err(err),
        ),
        Some("columns") => fake.resolve(label, result.map(|()| vec![column("id")]).map_err(err)),
        Some("databases") => fake.resolve(label, result.map(|()| strings(&["app"])).map_err(err)),
        _ => fake.resolve(
            label,
            result
                .map(|()| crate::db::fake::session(label.split(' ').nth(1).unwrap_or_default()))
                .map_err(err),
        ),
    }
}

/// Collapses the row labelled `label`.
pub(super) fn collapse(
    explorer: &gpui_kit::Entity<super::Explorer>,
    cx: &mut gpui_kit::VisualTestContext,
    label: &str,
) {
    explorer.update_in(cx, |e, _, cx| {
        let ix = e
            .nodes
            .iter()
            .position(|n| n.label.as_ref() == label)
            .unwrap_or_else(|| panic!("{label} row"));
        e.set_expanded(ix, false, cx);
    });
    cx.run_until_parked();
}

/// Right-clicks the row labelled `label` and picks Refresh from its menu.
pub(super) fn refresh(
    explorer: &gpui_kit::Entity<super::Explorer>,
    cx: &mut gpui_kit::VisualTestContext,
    label: &str,
) {
    let ix = labels(explorer, cx)
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("{label} row"));
    let at = cx
        .debug_bounds(format!("tree-label-{ix}").leak())
        .unwrap_or_else(|| panic!("{label} row drawn"))
        .center();
    cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
    cx.run_until_parked();
    let item = cx
        .debug_bounds("menu-refresh")
        .unwrap_or_else(|| panic!("Refresh in {label}'s menu"));
    cx.simulate_click(item.center(), Default::default());
    cx.run_until_parked();
}

/// Whether the row labelled `label` is expanded.
pub(super) fn is_expanded(
    explorer: &gpui_kit::Entity<super::Explorer>,
    cx: &gpui_kit::VisualTestContext,
    label: &str,
) -> bool {
    explorer.read_with(cx, |e, _| {
        e.nodes
            .iter()
            .find(|n| n.label.as_ref() == label)
            .unwrap_or_else(|| panic!("{label} row"))
            .expanded
    })
}

/// Answers every pending call with its sample until none is left; returns the calls made.
pub(super) fn answer_all(
    fake: &crate::db::fake::Fake,
    cx: &mut gpui_kit::VisualTestContext,
    fail: &str,
) -> Vec<String> {
    let mut calls = Vec::new();
    while let Some(call) = fake.pending().first().cloned() {
        answer(
            fake,
            &call,
            if call == fail {
                Err("permission denied")
            } else {
                Ok(())
            },
        );
        cx.run_until_parked();
        calls.push(call);
    }
    calls
}
