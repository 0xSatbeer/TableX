mod assets;
mod db;
mod ddl;
mod keymap;
mod mock;
mod model;
mod persist;
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
        AddConnection,
        EditConnection,
        DeleteConnection,
        RefreshNode,
        ModifyObject,
        NewTable,
        NewDatabase,
        NewRole,
        NewUser,
        OpenConsole
    ]
);

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let mode = Mode::from_args(std::env::args().skip(1));
            shell::apply_mock_latency(mode, std::env::var("QUERYX_MOCK_LATENCY_MS").ok());
            theme::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
                window_min_size: Some(size(px(900.), px(600.))),
                ..TitleBar::window_options()
            };
            cx.set_global(persist::Keys::new(std::sync::Arc::new(persist::OsKeychain)));
            // No home or config dir: built-in themes only, nothing saved.
            let config = persist::ConfigDir::resolve(|k| std::env::var_os(k));
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
            let (window, _) = shell::open_shell(mode, options, themes, config.map(|c| c.root), cx)
                .expect("open window");
            if let Some(notice) = notice {
                use gpui_kit::component::{WindowExt as _, notification::Notification};
                let _ = window.update(cx, |_, window, cx| {
                    window.push_notification(Notification::error(notice), cx)
                });
            }
            cx.activate(true);
        });
}
