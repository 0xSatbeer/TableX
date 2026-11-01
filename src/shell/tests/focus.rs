use super::*;

#[gpui_kit::test]
fn zoom_keys_change_only_the_focused_grid_or_editor(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{AppContext as _, Focusable as _, px};
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::shell::bind_zoom_keys(cx);
    });
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    let grid = shell.read_with(&cx, |this, _| {
        match this.tabs.active().map(|t| &t.content) {
            Some(crate::shell::TabContent::Grid(grid, _)) => grid.clone(),
            _ => panic!("launch opens on a grid"),
        }
    });
    let sizes = |cx: &gpui_kit::VisualTestContext| {
        shell.read_with(cx, |this, cx| (grid.read(cx).mono(), this.console_mono))
    };
    let steps: [(&str, &str, (f32, f32)); 7] = [
        ("grid", "ctrl-=", (14., 12.5)),
        ("grid", "ctrl-0", (13., 12.5)),
        ("grid", "ctrl--", (12., 12.5)),
        ("editor", "ctrl-=", (12., 13.5)),
        ("editor", "ctrl-0", (12., 12.5)),
        ("explorer", "ctrl-=", (12., 12.5)),
        ("nothing", "ctrl-=", (12., 12.5)),
    ];
    for (focus, keys, (grid_mono, console_mono)) in steps {
        if focus == "editor" {
            // The console is only mounted while its tab is active.
            shell.update(&mut cx, |this, cx| {
                let console = this
                    .tabs
                    .tabs
                    .iter()
                    .position(|t| matches!(t.content, crate::shell::TabContent::Console(_)))
                    .expect("launch opens a console");
                this.tabs.activate(console);
                this.tab_changed(cx);
            });
            cx.run_until_parked();
        }
        let handle = shell.read_with(&cx, |this, cx| match focus {
            "grid" => Some(grid.focus_handle(cx)),
            "editor" => Some(super::console_1(this, cx).focus_handle(cx)),
            "explorer" => Some(this.explorer.focus_handle(cx)),
            _ => None,
        });
        cx.update(|window, cx| match &handle {
            Some(handle) => window.focus(handle, cx),
            None => window.blur(cx),
        });
        cx.simulate_keystrokes(keys);
        assert_eq!(
            sizes(&cx),
            (px(grid_mono), px(console_mono)),
            "{keys} with {focus} focused"
        );
    }
}

#[gpui_kit::test]
fn console_editor_follows_ui_scale_like_the_grid(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{AppContext as _, Focusable as _};
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::shell::bind_zoom_keys(cx);
        crate::theme::set_ui_scale(130, cx);
    });
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    shell.update(&mut cx, |this, cx| {
        let console = this
            .tabs
            .tabs
            .iter()
            .position(|t| matches!(t.content, crate::shell::TabContent::Console(_)))
            .expect("launch opens a console");
        this.tabs.activate(console);
        this.tab_changed(cx);
    });
    cx.run_until_parked();
    let editor = shell.read_with(&cx, |this, cx| super::console_1(this, cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&editor, cx));
    for (keys, want) in [("", 16.25), ("ctrl-=", 17.55)] {
        if !keys.is_empty() {
            cx.simulate_keystrokes(keys);
        }
        let got: f32 = shell
            .read_with(&cx, |this, _| this.console_text_size())
            .into();
        assert!(
            (got - want).abs() < 0.001,
            "after {keys:?} at 130%: console {got}px, want {want}px"
        );
    }
}

#[gpui_kit::test]
fn focus_ring_follows_the_focused_pane(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{AppContext as _, Focusable as _};
    cx.update(gpui_kit::init);
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    let grid = shell.read_with(&cx, |this, _| {
        match this.tabs.active().map(|t| &t.content) {
            Some(crate::shell::TabContent::Grid(grid, _)) => grid.clone(),
            _ => panic!("launch opens on a grid"),
        }
    });
    // Tree, then Tab over to the grid, the console and the panel list; then nothing.
    for (focus, pane) in [
        ("explorer", Some("pane-explorer")),
        ("grid", Some("pane-content")),
        ("console", Some("pane-content")),
        ("panel", Some("pane-panel")),
        ("nothing", None),
    ] {
        if focus == "console" {
            shell.update(&mut cx, |this, cx| {
                let console = this
                    .tabs
                    .tabs
                    .iter()
                    .position(|t| matches!(t.content, crate::shell::TabContent::Console(_)))
                    .expect("launch opens a console");
                this.tabs.activate(console);
                this.tab_changed(cx);
            });
            cx.run_until_parked();
        }
        let handle = shell.read_with(&cx, |this, cx| match focus {
            "explorer" => Some(this.explorer.focus_handle(cx)),
            "grid" => Some(grid.focus_handle(cx)),
            "console" => Some(super::console_1(this, cx).focus_handle(cx)),
            "panel" => Some(this.bottom_panel.focus_handle(cx)),
            _ => None,
        });
        cx.update(|window, cx| match &handle {
            Some(handle) => window.focus(handle, cx),
            None => window.blur(cx),
        });
        cx.run_until_parked();
        let ring = cx.debug_bounds("focus-ring");
        match pane {
            Some(pane) => {
                let bounds = cx.debug_bounds(pane);
                assert!(ring.is_some(), "{focus} focused: no focus ring drawn");
                assert_eq!(ring, bounds, "{focus} focused: the ring outlines {pane}");
                // The splitter lines paint after the panes; one lying on the ring would hide that edge.
                let ring = ring.expect("ring drawn");
                for splitter in ["splitter-explorer", "splitter-bottom"] {
                    let line = cx.debug_bounds(splitter).expect("splitter drawn");
                    assert!(
                        !ring.intersects(&line),
                        "{focus} focused: {splitter} {line:?} covers an edge of the ring {ring:?}"
                    );
                }
            }
            None => assert_eq!(ring, None, "nothing focused: no ring"),
        }
    }
}

#[gpui_kit::test]
fn aggregate_shows_only_while_the_grid_is_focused(cx: &mut gpui_kit::TestAppContext) {
    use crate::model::{ColumnMeta, ResultSet, Value};
    use gpui_kit::{AppContext as _, Focusable as _};
    cx.update(gpui_kit::init);
    let (window, shell) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| demo_shell(fake(), window, cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    let ints = ResultSet {
        columns: vec![ColumnMeta {
            name: "n".into(),
            data_type: "int".into(),
            is_pk: false,
            ..Default::default()
        }],
        rows: vec![[Value::Int(3)].into(), [Value::Int(4)].into()],
    };
    let grid = shell.update(&mut cx, |this, cx| {
        this.tabs.open(crate::shell::start::grid_tab(
            "ints",
            std::rc::Rc::new(ints),
            cx,
        ));
        this.tab_changed(cx);
        match this.tabs.active().map(|t| &t.content) {
            Some(crate::shell::TabContent::Grid(grid, _)) => grid.clone(),
            _ => panic!("opened tab is a grid"),
        }
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-down");
    let shown = Some("2 rows × 1 col · count 2 · sum 7 · avg 3.5".to_string());
    // Grid first, then each other pane and nothing, then back to the grid; the console editor shares the content pane.
    for (focus, expected) in [
        ("grid", &shown),
        ("explorer", &None),
        ("console", &None),
        ("panel", &None),
        ("nothing", &None),
        ("grid", &shown),
    ] {
        let handle = shell.read_with(&cx, |this, cx| match focus {
            "explorer" => Some(this.explorer.focus_handle(cx)),
            "grid" => Some(grid.focus_handle(cx)),
            "console" => Some(super::console_1(this, cx).focus_handle(cx)),
            "panel" => Some(this.bottom_panel.focus_handle(cx)),
            _ => None,
        });
        cx.update(|window, cx| match &handle {
            Some(handle) => window.focus(handle, cx),
            None => window.blur(cx),
        });
        cx.run_until_parked();
        let text = shell.read_with(&cx, |this, cx| this.status_bar.read(cx).aggregate_text());
        assert_eq!(&text, expected, "{focus} focused");
    }
}
