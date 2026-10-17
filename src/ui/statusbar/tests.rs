use super::StatusBar;

mod aggregate;
mod connection_status;
mod context;

#[test]
fn reads_private_memory_from_proc_status() {
    let status =
        "Name:\tqueryx\nVmRSS:\t   92160 kB\nRssAnon:\t   35840 kB\nRssFile:\t   56320 kB\n";
    assert_eq!(super::rss_anon_mb(status), Some(35));
    assert_eq!(super::rss_anon_mb("Name:\tqueryx\n"), None);
}

/// A bar `width` px wide over a workspace with Postgres connection 1 (db.acme.internal:5433)
/// and SQLite connection 2 (/x/local.db), started with or without `demo`.
fn open_bar(
    cx: &mut gpui_kit::TestAppContext,
    width: f32,
    demo: bool,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<super::StatusBar>,
    gpui_kit::Entity<crate::model::Workspace>,
) {
    use crate::model::{ConnectionParams, SslMode, Workspace};
    use gpui_kit::{AppContext as _, Bounds, WindowBounds, WindowOptions, point, px, size};
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let mut ws = Workspace {
        id_floor: 0,
        id: 1,
        name: "w".into(),
        connections: Vec::new(),
    };
    let pg = ConnectionParams::Postgres {
        host: "db.acme.internal".into(),
        port: 5433,
        user: "app".into(),
        password: None,
        database: "app".into(),
        ssl: SslMode::Prefer,
    };
    ws.add("app-pg".into(), 0, false, pg);
    ws.add(
        "local.db".into(),
        4,
        false,
        ConnectionParams::Sqlite {
            path: "/x/local.db".into(),
        },
    );
    let workspace = cx.new(|_| ws);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(width), px(100.)),
        ))),
        ..Default::default()
    };
    let (window, bar) = cx.update(|cx| {
        let workspace = workspace.clone();
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| super::StatusBar::new(workspace, demo, window, cx))
        })
        .expect("open test window")
    });
    (
        gpui_kit::VisualTestContext::from_window(window, cx),
        bar,
        workspace,
    )
}

/// Points `bar` at connection `ix` of `workspace` with `status`, as the Shell does for the active tab.
fn show(
    cx: &mut gpui_kit::VisualTestContext,
    bar: &gpui_kit::Entity<super::StatusBar>,
    workspace: &gpui_kit::Entity<crate::model::Workspace>,
    ix: usize,
    status: crate::model::ConnectionStatus,
) {
    let origin = workspace.update(cx, |ws, cx| {
        ws.connections[ix].status = status;
        cx.notify();
        crate::ui::tabs::Origin::of(&ws.connections[ix])
    });
    bar.update(cx, |bar, cx| bar.set_origin(Some(origin), true, cx));
    cx.run_until_parked();
}

fn drawn(cx: &mut gpui_kit::VisualTestContext, selector: String) -> bool {
    cx.debug_bounds(selector.leak()).is_some()
}

/// Hovers the element drawn as `selector` past the tooltip delay and says whether `tip` came up.
fn hover_shows(cx: &mut gpui_kit::VisualTestContext, selector: &str, tip: String) -> bool {
    use gpui_kit::{Modifiers, point, px};
    cx.simulate_mouse_move(point(px(1.), px(1.)), None, Modifiers::none());
    cx.run_until_parked();
    let target = cx
        .debug_bounds(selector.to_string().leak())
        .unwrap_or_else(|| panic!("{selector} drawn"));
    cx.simulate_mouse_move(target.center(), None, Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    drawn(cx, tip)
}
