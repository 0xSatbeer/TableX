use super::*;

#[test]
fn demo_mode_needs_the_exact_demo_flag_and_ignores_every_other_argument() {
    use crate::shell::Mode::{self, *};
    // (args after the program name, mode)
    let cases: [(&[&str], Mode); 9] = [
        (&[], Normal),
        (&["--demo"], Demo),
        (&["--unknown"], Normal),
        (&["--unknown", "--demo"], Demo),
        (&["--demo", "--unknown"], Demo),
        (&["--demo", "--demo"], Demo),
        (&["--demo=1"], Normal),
        (&["demo"], Normal),
        (&["--DEMO"], Normal),
    ];
    for (args, want) in cases {
        assert_eq!(
            Mode::from_args(args.iter().map(|a| a.to_string())),
            Ok(want),
            "{args:?}"
        );
    }
}

#[test]
fn bench_mode_needs_a_known_scenario_after_the_flag() {
    use crate::bench::Scenario::*;
    use crate::shell::Mode::{self, *};
    // (args after the program name, mode or error)
    let cases: [(&[&str], Result<Mode, &str>); 7] = [
        (&["--bench-scenario", "startup"], Ok(Bench(Startup))),
        (&["--bench-scenario", "idle"], Ok(Bench(Idle))),
        (&["--bench-scenario", "big_result"], Ok(Bench(BigResult))),
        (&["--demo", "--bench-scenario", "idle"], Ok(Bench(Idle))),
        (
            &["--bench-scenario", "warm"],
            Err("unknown bench scenario: warm"),
        ),
        (&["--bench-scenario"], Err("unknown bench scenario: ")),
        (&["--bench-scenario=idle"], Ok(Normal)),
    ];
    for (args, want) in cases {
        assert_eq!(
            Mode::from_args(args.iter().map(|a| a.to_string())),
            want.map_err(String::from),
            "{args:?}"
        );
    }
}

#[test]
fn the_mock_latency_variable_applies_in_demo_mode_only() {
    use crate::shell::Mode::{self, *};
    // (mode, QUERYX_MOCK_LATENCY_MS, latency in ms)
    let cases: [(Mode, Option<&str>, u64); 10] = [
        (Demo, Some("500"), 500),
        (Normal, Some("500"), 0),
        (Demo, None, 0),
        (Normal, None, 0),
        (Demo, Some("0"), 0),
        (Demo, Some("2000"), 2000),
        (Demo, Some("5000"), 2000), // capped
        (Demo, Some("abc"), 0),
        (Demo, Some(""), 0),
        (Demo, Some("-5"), 0),
    ];
    for (mode, value, want) in cases {
        crate::mock::set_latency(0);
        crate::shell::apply_mock_latency(mode, value.map(String::from));
        assert_eq!(
            crate::mock::latency(),
            std::time::Duration::from_millis(want),
            "{mode:?} {value:?}"
        );
    }
    crate::mock::set_latency(0);
}

#[gpui_kit::test]
fn a_normal_start_shows_no_connections_one_empty_console_and_empty_logs(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_started(cx, crate::shell::Mode::Normal);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(
        shell.read_with(&cx, |this, cx| this
            .workspace
            .read(cx)
            .connections
            .is_empty()),
        "no connections"
    );
    assert!(
        cx.debug_bounds("explorer-empty").is_some(),
        "the explorer shows No connections yet"
    );
    let tabs = shell.read_with(&cx, |this, _| {
        this.tabs
            .tabs
            .iter()
            .map(|t| {
                (
                    t.title.to_string(),
                    matches!(t.content, crate::shell::TabContent::Console(_)),
                    t.origin.is_none(),
                )
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(
        tabs,
        [("console-1".to_string(), true, true)],
        "one console tab, on no connection"
    );
    assert_eq!(
        shell.read_with(&cx, |this, cx| super::console_1(this, cx).read(cx).value()),
        "",
        "the console is empty"
    );
    assert!(
        shell.read_with(&cx, |this, cx| this
            .bottom_panel
            .read(cx)
            .logged_connections()
            .is_empty()),
        "Logs are empty"
    );
}

#[gpui_kit::test]
fn a_normal_start_reads_connections_through_the_real_driver_not_the_samples(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::model::ConnectionStatus;
    // The real driver wakes the test's tasks from its own threads.
    cx.executor().allow_parking();
    let (window, shell) = open_started(cx, crate::shell::Mode::Normal);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let (workspace, explorer) = shell.read_with(&cx, |this, _| {
        (this.workspace.clone(), this.explorer.clone())
    });
    // The sample local.db: the mock driver would serve its tree, the real one finds no file there.
    let mut local = crate::mock::workspaces().remove(0).connections.remove(2);
    (local.status, local.databases) = (ConnectionStatus::Disconnected, None);
    let path = match &local.params {
        crate::model::ConnectionParams::Sqlite { path } => path.display().to_string(),
        other => panic!("local.db is SQLite, got {other:?}"),
    };
    assert!(
        !std::path::Path::new(&path).exists(),
        "{path} must not exist"
    );
    workspace.update(&mut cx, |ws, cx| {
        ws.connections.push(local);
        cx.notify();
    });
    cx.run_until_parked();
    let at = cx
        .debug_bounds("tree-label-0")
        .expect("local.db row drawn")
        .center();
    cx.simulate_click(at, Default::default());
    cx.simulate_keystrokes("right");
    // The real driver answers from its own threads.
    let status = |cx: &gpui_kit::VisualTestContext| {
        workspace.read_with(cx, |ws, _| ws.connections[0].status.clone())
    };
    for _ in 0..250 {
        cx.run_until_parked();
        if status(&cx) != ConnectionStatus::Connecting {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        status(&cx),
        ConnectionStatus::Error(format!("No file at {path}")),
        "the real driver's answer"
    );
    let labels: Vec<String> = explorer.read_with(&cx, |e, _| {
        e.row_labels().iter().map(|l| l.to_string()).collect()
    });
    assert!(
        !labels.iter().any(|l| l == "main"),
        "no sample databases, got {labels:?}"
    );
}

#[gpui_kit::test]
fn a_demo_start_shows_the_acme_workspace_the_sample_tabs_and_the_mock_log(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (window, shell) = open_started(cx, crate::shell::Mode::Demo);
    let cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let (name, connections) = shell.read_with(&cx, |this, cx| {
        let ws = this.workspace.read(cx);
        (
            ws.name.clone(),
            ws.connections
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(
        (name.as_str(), connections),
        (
            "Acme",
            vec![
                "prod-pg".to_string(),
                "staging-pg".into(),
                "local.db".into()
            ]
        )
    );
    let tabs = shell.read_with(&cx, |this, _| {
        this.tabs
            .tabs
            .iter()
            .map(|t| t.title.to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        tabs,
        ["big_events", "wide_table", "edge_cases", "console-1"]
    );
    let console = shell.read_with(&cx, |this, _| {
        this.tabs.tabs[3].origin.as_ref().map(|o| o.connection)
    });
    assert_eq!(
        console,
        Some(crate::model::ConnectionId(1)),
        "the console runs on prod-pg"
    );
    let logged = shell.read_with(&cx, |this, cx| {
        this.bottom_panel.read(cx).logged_connections()
    });
    assert!(!logged.is_empty(), "the mock query log");
}

#[gpui_kit::test]
fn opening_a_table_loads_from_the_driver_in_normal_and_demo_mode(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::AppContext as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    // Demo tables read through the Mock driver like any other, so a Fake keeps both loading.
    for mode in [crate::shell::Mode::Normal, crate::shell::Mode::Demo] {
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| {
                    crate::shell::Shell::with_driver(
                        mode,
                        crate::mock::workspaces().remove(0),
                        fake(),
                        window,
                        cx,
                    )
                })
            })
            .expect("open test window")
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.run_until_parked();
        let explorer = shell.read_with(&vcx, |this, _| this.explorer.clone());
        explorer.update_in(&mut vcx, |e, window, cx| {
            e.reveal("1/app_db/public/table/users", window, cx)
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        let (title, rows) = shell.read_with(&vcx, |this, cx| {
            let tab = this.tabs.active().expect("a tab opened");
            let rows = match &tab.content {
                crate::shell::TabContent::Grid(grid, _) => grid.read(cx).row_count(),
                crate::shell::TabContent::Console(_) => panic!("{mode:?}: a grid tab opens"),
            };
            (tab.title.to_string(), rows)
        });
        assert_eq!(title, "users", "{mode:?}");
        assert_eq!(rows, 0, "{mode:?}: an empty grid");
        assert!(
            vcx.debug_bounds("table-loading").is_some(),
            "{mode:?}: the loading state"
        );
    }
}

/// Bench mode handed a config root still writes nothing there: no config, workspaces,
/// sessions or query log.
#[gpui_kit::test]
fn bench_mode_writes_nothing_under_the_config_root(cx: &mut gpui_kit::TestAppContext) {
    let dir = crate::theme::tests::test_dir("bench-no-writes");
    cx.update(gpui_kit::init);
    let (window, _) = cx
        .update(|cx| {
            crate::shell::open_shell(
                crate::shell::Mode::Bench(crate::bench::Scenario::Idle),
                Default::default(),
                None,
                Some(dir.clone()),
                cx,
            )
        })
        .expect("open test window");
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    vcx.dispatch_action(crate::CycleTheme);
    vcx.dispatch_action(crate::NewConsole);
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    vcx.run_until_parked();
    let written: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert!(written.is_empty(), "{written:?}");
    crate::theme::set_ui_scale(crate::theme::DEFAULT_UI_SCALE, &mut vcx.app.borrow_mut());
}

/// `big_result` opens its 5 tabs and isn't settled while any of them is still reading.
#[gpui_kit::test]
fn big_result_opens_five_tabs_and_waits_for_them_to_settle(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::AppContext as _;
    let workspace = crate::bench::workspace(
        Some("postgres://u:p@localhost:1/db".into()),
        Some("/nowhere/bench.sqlite".into()),
    )
    .unwrap();
    cx.update(gpui_kit::init);
    // (scenario, tabs, the bench rows once parked)
    let cases = [
        (crate::bench::Scenario::BigResult, 5, None),
        (crate::bench::Scenario::Idle, 1, Some(0)),
    ];
    for (scenario, want_tabs, want) in cases {
        let (workspace, mode) = (workspace.clone(), crate::shell::Mode::Bench(scenario));
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| {
                    crate::shell::Shell::with_driver(mode, workspace, super::fake(), window, cx)
                })
            })
            .expect("open test window")
        });
        let vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.run_until_parked();
        let (tabs, rows) =
            shell.read_with(&vcx, |this, cx| (this.tabs.tabs.len(), this.bench_rows(cx)));
        assert_eq!((tabs, rows), (want_tabs, want), "{scenario:?}");
    }
}

#[gpui_kit::test]
fn a_demo_table_reads_its_rows_through_the_mock_driver(cx: &mut gpui_kit::TestAppContext) {
    let (window, shell) = open_started(cx, crate::shell::Mode::Demo);
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
    explorer.update_in(&mut cx, |e, window, cx| {
        e.reveal("1/app_db/public/table/users", window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    // The mock driver answers after its simulated latency.
    for _ in 0..20 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
    }
    let rows = shell.read_with(&cx, |this, cx| {
        match &this.tabs.active().expect("a tab").content {
            crate::shell::TabContent::Grid(grid, _) => grid.read(cx).row_count(),
            crate::shell::TabContent::Console(_) => panic!("a grid tab opens"),
        }
    });
    assert!(rows > 0, "mock rows");
    assert!(cx.debug_bounds("table-loading").is_none());
}
