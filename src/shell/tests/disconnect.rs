use super::support::*;
use super::table_data::{PAGE_1, answer, column, grid_rows, open_users, rows};
use crate::db::fake;
use crate::model::{ConnectionStatus, QueryError, QueryErrorKind};
use gpui_kit::component::WindowExt as _;

fn disconnected() -> QueryError {
    QueryError {
        kind: QueryErrorKind::Disconnected,
        message: "server closed the connection unexpectedly".into(),
        position: None,
        statement: 0,
    }
}

/// `users` read, then `orders` opened beside it with its page-1 query pending.
fn two_tabs(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    std::sync::Arc<fake::Fake>,
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<crate::shell::Shell>,
) {
    let (fake, window, shell) = open_users(cx);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    answer(&fake, Ok(rows(3)), &mut cx);
    let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
    explorer.update_in(&mut cx, |e, window, cx| {
        e.reveal("1/app_db/public/table/orders", window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    fake.resolve("columns app_db/public/orders", Ok(vec![column("id", true)]));
    cx.run_until_parked();
    assert_eq!(fake.pending(), ["query app_db"]);
    (fake, cx, shell)
}

fn reconnect_buttons(
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    cx: &mut gpui_kit::VisualTestContext,
) -> usize {
    let n = shell.read_with(cx, |this, _| this.tabs.tabs.len());
    (0..n)
        .filter(|&ix| {
            shell.update(cx, |this, cx| {
                this.tabs.activate(ix);
                cx.notify();
            });
            cx.run_until_parked();
            cx.debug_bounds("table-reconnect").is_some()
        })
        .count()
}

#[gpui_kit::test]
fn a_disconnected_table_read_marks_every_tab_of_the_connection_and_toasts_once(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, shell) = two_tabs(cx);
    answer(&fake, Err(disconnected()), &mut cx);
    let status = shell.read_with(&cx, |this, cx| {
        this.workspace.read(cx).connections[0].status.clone()
    });
    assert_eq!(status, ConnectionStatus::Disconnected);
    assert_eq!(
        reconnect_buttons(&shell, &mut cx),
        2,
        "both tabs show Reconnect"
    );
    let toasts = cx.update(|window, cx| window.notifications(cx).len());
    assert_eq!(toasts, 1, "one toast");

    // A second failure while already lost doesn't toast again.
    cx.simulate_keystrokes("f5");
    cx.run_until_parked();
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    answer(&fake, Err(disconnected()), &mut cx);
    let toasts = cx.update(|window, cx| window.notifications(cx).len());
    assert_eq!(toasts, 1, "no repeat while lost");
}

#[gpui_kit::test]
fn reconnect_connects_again_and_reads_the_page(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, shell) = two_tabs(cx);
    answer(&fake, Err(disconnected()), &mut cx);
    let before = fake.queried().len();
    click(&mut cx, "table-reconnect");
    let mut pending = fake.pending();
    pending.sort();
    assert_eq!(
        pending,
        ["connect app_db", "connect app_db"],
        "the tree and the tab connect"
    );
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    let mut read = fake.queried()[before..].to_vec();
    read.sort();
    let orders = PAGE_1.replace("users", "orders");
    assert_eq!(
        read,
        [orders.as_str(), PAGE_1],
        "both lost tabs read their page"
    );
    while fake.pending().iter().any(|p| p != "query app_db") {
        let call = fake
            .pending()
            .into_iter()
            .find(|p| p != "query app_db")
            .unwrap();
        fake.resolve(&call, Ok(Vec::<String>::new()));
        cx.run_until_parked();
    }
    answer(&fake, Ok(rows(2)), &mut cx);
    answer(&fake, Ok(rows(2)), &mut cx);
    assert_eq!(
        reconnect_buttons(&shell, &mut cx),
        0,
        "no tab shows Reconnect"
    );
    assert_eq!(grid_rows(&shell, &cx), 2);
    let status = shell.read_with(&cx, |this, cx| {
        this.workspace.read(cx).connections[0].status.clone()
    });
    assert_eq!(status, ConnectionStatus::Connected);
}
