// Release builds on Windows are GUI-subsystem so no console window opens behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod bench;
mod complete;
mod db;
mod ddl;
mod export;
mod fuzzy;
mod keymap;
mod mock;
mod model;
mod persist;
mod sessions;
mod shell;
mod sql;
mod theme;
mod ui;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use shell::Mode;

actions!(
    queryx,
    [
        Quit,
        ToggleExplorer,
        ToggleBottomPanel,
        MaximizeEditor,
        MaximizePanel,
        RestorePanel,
        ToggleGallery,
        FocusExplorer,
        CloseTab,
        ReopenTab,
        NextTab,
        PrevTab,
        CloseOtherTabs,
        CloseTabsToRight,
        CloseAllTabs,
        TogglePinTab,
        CopyTabName,
        RevealTab,
        CycleTheme,
        OpenSettings,
        AddConnection,
        EditConnection,
        DeleteConnection,
        RefreshNode,
        Disconnect,
        ModifyObject,
        NewTable,
        NewDatabase,
        NewRole,
        NewUser,
        OpenConsole,
        NewConsole,
        OpenSelected,
        CopyName,
        CopyQualifiedName,
        ShowDdl,
        DropObject,
        CommandPalette,
        CheatSheet,
        QuickOpen
    ]
);

fn main() {
    let mode = match Mode::from_args(std::env::args().skip(1)) {
        Ok(mode) => mode,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let bench = match mode {
        Mode::Bench(scenario) => Some(scenario),
        _ => None,
    };
    if bench.is_some() && !cfg!(target_os = "linux") {
        eprintln!("--bench-scenario reads /proc/self/status and runs on Linux only");
        std::process::exit(1);
    }
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            shell::apply_mock_latency(mode, std::env::var("QUERYX_MOCK_LATENCY_MS").ok());
            theme::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            let options = WindowOptions {
                // Restore size is the 1280x800 centered window when unmaximized.
                window_bounds: Some(WindowBounds::Maximized(Bounds::centered(
                    None,
                    size(px(1280.), px(800.)),
                    cx,
                ))),
                window_min_size: Some(size(px(900.), px(600.))),
                // Matches io.github._0xSatbeer.QueryX.desktop so Linux docks show the app icon.
                app_id: Some("io.github._0xSatbeer.QueryX".into()),
                // The OS title (overview, Alt+Tab, taskbar); the bar itself shows no name, like GNOME apps.
                titlebar: Some(TitlebarOptions {
                    title: Some("QueryX".into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            };
            let keychain: std::sync::Arc<dyn persist::Keychain> = match bench {
                Some(_) => std::sync::Arc::new(persist::MemKeychain::default()),
                None => std::sync::Arc::new(persist::OsKeychain),
            };
            cx.set_global(persist::Keys::new(keychain));
            // No home or config dir: built-in themes only, nothing saved. Bench mode reads none either.
            let config =
                persist::ConfigDir::resolve(|k| std::env::var_os(k)).filter(|_| bench.is_none());
            let (overrides, mut notice) = match &config {
                Some(c) => keymap::load(&c.root),
                None => Default::default(),
            };
            let skipped = keymap::bind(cx, &overrides);
            if !skipped.is_empty() {
                notice = Some(format!(
                    "keymap.toml entries skipped: {}",
                    skipped.join(", ")
                ));
            }
            let themes = config.as_ref().map(|c| c.root.join("themes"));
            let (window, shell) =
                shell::open_shell(mode, options, themes, config.map(|c| c.root), cx)
                    .expect("open window");
            if let Some(scenario) = bench {
                bench::run(scenario, shell, cx);
            }
            if let Some(notice) = notice {
                let _ = window.update(cx, |_, window, cx| ui::toast::error(notice, window, cx));
            }
            cx.activate(true);
        });
}
