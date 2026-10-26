use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};

use super::{Connect, ConnectionLost, ConnectionRestored, Sessions};
use crate::db::Driver;
use crate::db::fake::{self, Fake};
use crate::model::{ConnectionId, ConnectionStatus, Workspace};

/// Sessions over the sample workspace through a Fake: prod-pg (1) is Connected on app_db,
/// staging-pg (2) failed.
fn open(
    cx: &mut TestAppContext,
) -> (
    Arc<Fake>,
    VisualTestContext,
    Entity<Sessions>,
    Entity<Workspace>,
) {
    cx.update(gpui_kit::init);
    let fake = Arc::new(Fake::default());
    let driver: Arc<dyn Driver> = fake.clone();
    let workspace = cx.new(|_| crate::mock::workspaces().remove(0));
    let window = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|_| Blank))
            .expect("open test window")
    });
    let handle = window.0;
    let ws = workspace.clone();
    let sessions = cx.new(|cx| Sessions::new(ws, driver, handle, cx));
    (
        fake,
        VisualTestContext::from_window(handle, cx),
        sessions,
        workspace,
    )
}

/// A window for the lost-connection toast to show in.
struct Blank;

impl gpui_kit::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

fn status(
    workspace: &Entity<Workspace>,
    id: ConnectionId,
    cx: &VisualTestContext,
) -> ConnectionStatus {
    workspace.read_with(cx, |ws, _| {
        ws.connections
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .status
            .clone()
    })
}

const PROD: ConnectionId = ConnectionId(1);
const STAGING: ConnectionId = ConnectionId(2);

#[gpui_kit::test]
fn a_kept_session_is_reused_until_the_server_closes_it(cx: &mut TestAppContext) {
    let (fake, mut cx, sessions, _) = open(cx);
    let first = sessions.update(&mut cx, |s, cx| s.session(PROD, "app_db", cx));
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    let kept = futures::executor::block_on(first).expect("connected");
    let again = sessions.update(&mut cx, |s, cx| s.session(PROD, "app_db", cx));
    assert!(fake.pending().is_empty(), "the kept session is reused");
    futures::executor::block_on(again).expect("the kept session");
    fake::close(&kept);
    assert!(
        sessions
            .read_with(&cx, |s, _| s.open(PROD, "app_db"))
            .is_none()
    );
    let _reopen = sessions.update(&mut cx, |s, cx| s.session(PROD, "app_db", cx));
    assert_eq!(
        fake.pending(),
        ["connect app_db"],
        "a closed session is replaced"
    );
}

#[gpui_kit::test]
fn a_session_connecting_across_a_disconnect_is_not_kept(cx: &mut TestAppContext) {
    let (fake, mut cx, sessions, workspace) = open(cx);
    let lost = Rc::new(RefCell::new(Vec::new()));
    let seen = lost.clone();
    cx.update(|_, cx| {
        cx.subscribe(&sessions, move |_, ev: &ConnectionLost, _| {
            seen.borrow_mut().push(ev.id)
        })
        .detach()
    });
    let _late = sessions.update(&mut cx, |s, cx| s.session(PROD, "other_db", cx));
    let name = sessions.update(&mut cx, |s, cx| s.disconnect(PROD, cx));
    assert_eq!(name.as_deref(), Some("prod-pg"));
    fake.resolve("connect other_db", Ok(fake::session("other_db")));
    cx.run_until_parked();
    assert!(
        sessions
            .read_with(&cx, |s, _| s.open(PROD, "other_db"))
            .is_none()
    );
    assert_eq!(
        status(&workspace, PROD, &cx),
        ConnectionStatus::Disconnected
    );
    assert_eq!(*lost.borrow(), [PROD]);
    let again = sessions.update(&mut cx, |s, cx| s.disconnect(PROD, cx));
    assert_eq!(again, None, "a disconnected connection is not closed twice");
}

#[gpui_kit::test]
fn connect_goes_connecting_then_connected_and_ignores_a_second_connect(cx: &mut TestAppContext) {
    let (fake, mut cx, sessions, workspace) = open(cx);
    let restored = Rc::new(RefCell::new(Vec::new()));
    let seen = restored.clone();
    cx.update(|_, cx| {
        cx.subscribe(&sessions, move |_, ev: &ConnectionRestored, _| {
            seen.borrow_mut().push(ev.id)
        })
        .detach()
    });
    assert_eq!(
        sessions.update(&mut cx, |s, cx| s.connect(STAGING, cx)),
        Connect::Started
    );
    assert_eq!(
        status(&workspace, STAGING, &cx),
        ConnectionStatus::Connecting
    );
    assert_eq!(
        sessions.update(&mut cx, |s, cx| s.connect(STAGING, cx)),
        Connect::Busy
    );
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    assert_eq!(
        status(&workspace, STAGING, &cx),
        ConnectionStatus::Connected
    );
    assert_eq!(*restored.borrow(), [STAGING]);
    assert!(
        sessions
            .read_with(&cx, |s, _| s.open(STAGING, "app_db"))
            .is_some()
    );
}

#[gpui_kit::test]
fn a_failed_read_on_a_closed_session_loses_the_connection_once(cx: &mut TestAppContext) {
    let (fake, mut cx, sessions, workspace) = open(cx);
    let first = sessions.update(&mut cx, |s, cx| s.session(PROD, "app_db", cx));
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    fake::close(&futures::executor::block_on(first).unwrap());
    assert!(sessions.update(&mut cx, |s, cx| s.read_failed(PROD, "app_db", cx)));
    assert_eq!(
        status(&workspace, PROD, &cx),
        ConnectionStatus::Disconnected
    );
    assert!(!sessions.update(&mut cx, |s, cx| s.read_failed(PROD, "app_db", cx)));
}
