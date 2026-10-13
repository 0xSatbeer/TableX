mod assets;
mod db;
mod mock;
mod model;
mod shell;
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
        RefreshNode
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
            shell::bind_overflow_keys(cx);
            shell::bind_panel_keys(cx);
            shell::bind_zoom_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("ctrl-b", ToggleExplorer, None),
                KeyBinding::new("ctrl-j", ToggleBottomPanel, None),
                KeyBinding::new("ctrl-shift-f", MaximizeEditor, None),
                KeyBinding::new("ctrl-alt-shift-d", ToggleGallery, None),
                KeyBinding::new("ctrl-alt-f", FocusExplorer, None),
                KeyBinding::new("ctrl-w", CloseTab, None),
                KeyBinding::new("ctrl-shift-t", ReopenTab, None),
                KeyBinding::new("ctrl-tab", NextTab, None),
                KeyBinding::new("ctrl-shift-tab", PrevTab, None),
                KeyBinding::new("ctrl-k ctrl-t", CycleTheme, None),
            ]);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
                window_min_size: Some(size(px(900.), px(600.))),
                ..TitleBar::window_options()
            };
            // No home or config dir: built-in themes only.
            let user_themes = theme::UserThemeDir::resolve(|k| std::env::var_os(k));
            shell::open_shell(mode, options, user_themes.map(|t| t.dir), cx).expect("open window");
            cx.activate(true);
        });
}
