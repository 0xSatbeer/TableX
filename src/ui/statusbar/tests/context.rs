use super::*;

#[gpui_kit::test]
fn a_tab_without_a_connection_shows_no_connection_or_in_demo_mode_sample_data(
    cx: &mut gpui_kit::TestAppContext,
) {
    const SAMPLE: &str = "Sample data · not from a connection";
    // (demo, a tab is open, text shown)
    let cases = [
        (false, true, Some("No connection")),
        (true, true, Some(SAMPLE)),
        (false, false, None),
        (true, false, None),
    ];
    for (demo, has_tab, text) in cases {
        let (mut cx, bar, _) = open_bar(cx, 1200., demo);
        bar.update(&mut cx, |bar, cx| bar.set_origin(None, has_tab, cx));
        cx.run_until_parked();
        let case = format!("demo {demo}, tab {has_tab}");
        for known in ["No connection", SAMPLE] {
            assert_eq!(
                drawn(&mut cx, format!("status-origin:{known}")),
                text == Some(known),
                "{case}: {known:?}"
            );
        }
        for status in [
            "status-dot-solid",
            "status-dot-ring",
            "auto-commit",
            "status-button:Connect",
            "status-button:Retry",
        ] {
            assert!(!drawn(&mut cx, status.into()), "{case}: no {status}");
        }
    }
}

#[gpui_kit::test]
fn in_a_narrow_window_the_path_truncates_while_the_status_text_and_button_stay_whole(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionStatus::{self, *};
    let long =
        "app-pg › analytics_warehouse_replica_eu_west › shipment_tracking_archive_partitioned_2026";
    // (status, text selector, button selector)
    let cases: [(ConnectionStatus, &str, Option<&str>); 3] = [
        (
            Disconnected,
            "status-text:Not connected",
            Some("status-button:Connect"),
        ),
        (
            Error("connection refused".into()),
            "status-text-danger:Could not connect",
            Some("status-button:Retry"),
        ),
        (Connecting, "status-text:Connecting…", None),
    ];
    for (status, text, button) in cases {
        // (window width) -> widths of the path, text and button
        let mut widths = Vec::new();
        for (width, production) in [(1600., false), (420., false), (420., true)] {
            let (mut cx, bar, workspace) = open_bar(cx, width, false);
            show(&mut cx, &bar, &workspace, 0, status.clone());
            let origin = crate::ui::tabs::Origin {
                connection: crate::model::ConnectionId(1),
                color: 0,
                production,
                path: long.into(),
                tail: "".into(),
            };
            bar.update(&mut cx, |bar, cx| bar.set_origin(Some(origin), true, cx));
            cx.run_until_parked();
            let mut width_of = |selector: &str| {
                cx.debug_bounds(selector.to_string().leak())
                    .unwrap_or_else(|| panic!("{selector} drawn at {width}"))
                    .size
                    .width
            };
            widths.push((
                width_of("status-path"),
                width_of(text),
                button.map(&mut width_of),
            ));
        }
        let (wide, narrow, prod) = (widths[0], widths[1], widths[2]);
        assert!(
            narrow.0 < wide.0,
            "{status:?}: the path truncates ({:?} < {:?})",
            narrow.0,
            wide.0
        );
        assert_eq!(
            (narrow.1, narrow.2),
            (wide.1, wide.2),
            "{status:?}: the text and button keep their width"
        );
        assert_eq!(
            (prod.1, prod.2),
            (wide.1, wide.2),
            "{status:?}: with the PROD badge too"
        );
    }
}

#[gpui_kit::test]
fn truncated_status_path_shows_a_tooltip(cx: &mut gpui_kit::TestAppContext) {
    use crate::{theme::tests::hover_shows_truncation_tip, ui::tabs::Origin};
    use gpui_kit::{AppContext as _, Bounds, Entity, WindowBounds, WindowOptions, point, px, size};
    cx.update(gpui_kit::init);
    let long = "prod-pg › analytics_warehouse_replica_eu_west › shipment_tracking_archive_partitioned_2026";
    // (path, tooltip shown)
    for (path, tip) in [("prod-pg › app_db › public", false), (long, true)] {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(420.), px(100.)),
            ))),
            ..Default::default()
        };
        let (window, bar): (_, Entity<super::StatusBar>) = cx.update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    super::StatusBar::new(
                        cx.new(|_| crate::mock::workspaces().remove(0)),
                        true,
                        window,
                        cx,
                    )
                })
            })
            .expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        bar.update(&mut cx, |bar, cx| {
            bar.set_origin(
                Some(Origin {
                    connection: crate::model::ConnectionId(0),
                    color: 0,
                    production: false,
                    path: path.into(),
                    tail: "".into(),
                }),
                true,
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(
            hover_shows_truncation_tip(&mut cx, "status-path".into()),
            tip,
            "{path}"
        );
    }
}
