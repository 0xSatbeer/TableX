use gpui_kit::component::{
    ActiveTheme as _, Icon,
    button::{Button, ButtonVariants as _}, IconName, Sizable as _, TitleBar, h_flex,
    input::{Editor, EditorState},
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::base::{ResizeHandleRenderer, ResizeHandleState};

use std::rc::Rc;

mod assets;
mod explorer;
mod gallery;
mod grid;
#[allow(dead_code)] // ponytail: most of the model is read by UI tickets QX-005..QX-015; drop once they land
mod mock;
#[allow(dead_code)]
mod model;
mod panel;
mod split;
mod statusbar;
mod tabs;
mod theme;

actions!(queryx, [Quit, ToggleExplorer, ToggleBottomPanel, MaximizeEditor, ToggleGallery, FocusExplorer, CloseTab, ReopenTab, NextTab, PrevTab, CloseOtherTabs, CloseTabsToRight, CloseAllTabs, TogglePinTab, CopyTabName, RevealTab, CycleTheme]);

enum TabContent {
    Grid(Entity<grid::Grid>),
    Console,
}

/// 1px hairline that turns accent while dragged. Base owns the hit area, cursor and drag.
fn splitter() -> ResizeHandleRenderer {
    Rc::new(|handle, _, cx| {
        let dragging = handle.state() == ResizeHandleState::Dragging;
        let line = div().flex_none().bg(if dragging { cx.theme().primary } else { cx.theme().border });
        Some(match handle.axis() {
            Axis::Horizontal => line.w(px(1.)).h_full(),
            Axis::Vertical => line.h(px(1.)).w_full(),
        }
        .into_any_element())
    })
}

/// Panel visibility and split sizes in one place, so QX-020 can persist it.
/// Hidden panels keep their size because the split state is not touched while hidden.
struct LayoutState {
    explorer_visible: bool,
    bottom_visible: bool,
    /// Visibility of (explorer, bottom) from before `MaximizeEditor`, restored by the next one.
    maximized_from: Option<(bool, bool)>,
    /// [explorer, editor column]
    main_split: Entity<ResizableState>,
    /// [editor, bottom panel]
    editor_split: Entity<ResizableState>,
}

impl LayoutState {
    fn toggle_maximize(&mut self) {
        match self.maximized_from.take() {
            Some((explorer, bottom)) => (self.explorer_visible, self.bottom_visible) = (explorer, bottom),
            None => {
                self.maximized_from = Some((self.explorer_visible, self.bottom_visible));
                (self.explorer_visible, self.bottom_visible) = (false, false);
            }
        }
    }
}

struct Shell {
    explorer: Entity<explorer::Explorer>,
    editor: Entity<EditorState>,
    tabs: tabs::Tabs<TabContent>,
    tab_scroll: ScrollHandle,
    layout: LayoutState,
    status_bar: Entity<statusbar::StatusBar>,
    bottom_panel: Entity<panel::BottomPanel>,
    gallery_visible: bool,
    /// Set when a tab is activated; the next render moves keyboard focus into its content.
    focus_content: bool,
    theme: theme::ThemeChoice,
    _appearance: Subscription,
}

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = &mock::workspaces()[0];
        let explorer = cx.new(|cx| explorer::Explorer::new(workspace.clone(), cx));
        cx.subscribe(&explorer, |this, _, ev: &explorer::OpenObject, cx| {
            // ponytail: every table shows a mock query result until drivers land (Phase 5)
            let rows = match mock::run_query(&format!("SELECT * FROM {}", ev.name)) {
                model::QueryOutcome::Rows(rs) => rs,
                _ => mock::edge_cases(),
            };
            let mut tab = grid_tab(ev.key.clone(), Rc::new(rows), cx);
            (tab.title, tab.origin, tab.preview) = (ev.name.clone(), Some(ev.origin.clone()), ev.preview);
            tab.tooltip = Some(ev.path.clone());
            this.tabs.open(tab);
            this.tab_changed(cx);
            // A preview (single click, Space) leaves focus in the tree for the next arrow key.
            this.focus_content = !ev.preview;
        })
        .detach();
        let mut this = Self {
            explorer,
            editor: cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("sql")
                    .default_value("SELECT id, email\nFROM users\nWHERE created_at > now() - interval '7 days';\n")
            }),
            tab_scroll: ScrollHandle::new(),
            tabs: tabs::Tabs::new(vec![
                grid_tab("big_events", Rc::new(grid::BigEvents::new()), cx),
                grid_tab("wide_table", Rc::new(mock::wide_table(2_000)), cx),
                grid_tab("edge_cases", Rc::new(mock::edge_cases()), cx),
                tabs::EditorTab {
                    key: "console-1".into(),
                    title: "console-1".into(),
                    // The console runs against production, so the tint shows up in the demo.
                    origin: workspace.connections.iter().find(|c| c.is_production).map(tabs::Origin::of),
                    tooltip: None,
                    preview: false,
                    pinned: false,
                    content: TabContent::Console,
                },
            ]),
            layout: LayoutState {
                explorer_visible: true,
                bottom_visible: true,
                maximized_from: None,
                main_split: cx.new(|_| ResizableState::default()),
                editor_split: cx.new(|_| ResizableState::default()),
            },
            status_bar: cx.new(|cx| statusbar::StatusBar::new(window, cx)),
            bottom_panel: cx.new(|cx| panel::BottomPanel::new(workspace, window, cx)),
            gallery_visible: false,
            focus_content: false,
            theme: theme::ThemeChoice::default(),
            _appearance: cx.observe_window_appearance(window, |this, window, cx| this.theme.apply(window, cx)),
        };
        this.tab_changed(cx);
        this
    }
}

fn grid_tab(key: impl Into<SharedString>, source: Rc<dyn grid::GridSource>, cx: &mut Context<Shell>) -> tabs::EditorTab<TabContent> {
    let key = key.into();
    tabs::EditorTab {
        title: key.clone(),
        key,
        origin: None,
        tooltip: None,
        preview: false,
        pinned: false,
        content: TabContent::Grid(cx.new(|cx| grid::Grid::new(source, cx))),
    }
}

impl Shell {
    /// Points the status bar at the active tab: its origin, and the cursor only for the console.
    fn tab_changed(&mut self, cx: &mut Context<Self>) {
        let console = matches!(self.tabs.active().map(|t| &t.content), Some(TabContent::Console));
        let editor = console.then_some(&self.editor);
        let (origin, has_tab) = (self.tabs.active().and_then(|t| t.origin.clone()), self.tabs.active().is_some());
        self.status_bar.update(cx, |bar, cx| {
            bar.set_editor(editor, cx);
            bar.set_origin(origin, has_tab, cx);
        });
        self.focus_content = true;
        cx.notify();
    }

    /// ⚙ in the titlebar. Holds the theme until there is a settings view (QX-020);
    /// `Ctrl+K Ctrl+T` still cycles it from the keyboard.
    fn settings_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let current = self.theme;
        Button::new("settings")
            .ghost()
            .small()
            .icon(IconName::Settings)
            .tooltip("Settings")
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.label("Theme");
                for choice in theme::ThemeChoice::ALL {
                    let shell = shell.clone();
                    menu = menu.item(PopupMenuItem::new(choice.label()).checked(choice == current).on_click(
                        move |_, window, cx| {
                            _ = shell.update(cx, |this, cx| {
                                this.theme = choice;
                                this.theme.apply(window, cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            })
            .anchor(Anchor::TopRight)
    }

    /// Our own strip, not the library `TabBar`: its `Tab` repaints background and borders over
    /// ours, which hid the connection stripe and the production tint.
    fn tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<_> = self.tabs.tabs.iter().enumerate().map(|(ix, t)| self.tab(ix, t, cx)).collect();
        let theme = cx.theme();
        let shell = cx.entity().downgrade();
        let titles: Vec<SharedString> = self.tabs.tabs.iter().map(|t| t.title.clone()).collect();
        let active_ix = self.tabs.active;
        h_flex()
            .flex_none()
            .h(theme::TAB)
            .bg(theme.tab_bar)
            // The hairline under the strip is drawn per piece, so the active tab can leave a gap
            // and join the content below it.
            .child(
                h_flex()
                    .id("editor-tabs")
                    .flex_1()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs)
                    .child(div().flex_1().h_full().border_b_1().border_color(theme.border)),
            )
            .child(
                h_flex().flex_none().h_full().px(theme::SPACE_4).border_l_1().border_b_1().border_color(theme.border).child(
                    Button::new("all-tabs")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronDown)
                        .tooltip("All open tabs")
                        .dropdown_menu(move |mut menu, _, _| {
                            menu = menu.scrollable(true);
                            for (ix, title) in titles.iter().enumerate() {
                                let shell = shell.clone();
                                menu = menu.item(PopupMenuItem::new(title.clone()).checked(ix == active_ix).on_click(
                                    move |_, _, cx| {
                                        _ = shell.update(cx, |this, cx| {
                                            this.tabs.activate(ix);
                                            this.tab_changed(cx);
                                        });
                                    },
                                ));
                            }
                            menu
                        })
                        .anchor(Anchor::TopRight),
                ),
            )
    }

    fn tab(&self, ix: usize, t: &tabs::EditorTab<TabContent>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let icon = match t.content {
            TabContent::Grid(_) => Icon::empty().path(assets::object_icon("table", false)),
            TabContent::Console => Icon::new(IconName::SquareTerminal),
        };
        let active = ix == self.tabs.active;
        let group: SharedString = format!("tab-{ix}").into();
        let pinned = t.pinned;
        let production = t.origin.as_ref().is_some_and(|o| o.production);
        h_flex()
            .id(("tab", ix))
            .group(group.clone())
            .relative()
            .flex_none()
            .h_full()
            .pl(theme::SPACE_12)
            .pr(if pinned { theme::SPACE_12 } else { theme::SPACE_6 })
            .gap(theme::SPACE_6)
            .border_r_1()
            .when(!active, |d| d.border_b_1())
            .border_color(theme.border)
            .text_size(theme::UI_SM)
            .map(|d| if active {
                d.bg(theme.tab_active).text_color(theme.tab_active_foreground)
            } else {
                d.text_color(theme.tab_foreground).hover(|d| d.bg(theme.secondary_hover))
            })
            // Tint and stripe are overlays so they sit on top of the active/hover background.
            .when(production, |d| d.child(div().absolute().inset_0().bg(theme::production_tint(theme))))
            .when_some(t.origin.as_ref(), |d, o| {
                d.child(div().absolute().top_0().left_0().right_0().h(theme::STRIPE).bg(theme::connection_color(o.color, theme)))
            })
            .when_some(t.tooltip.clone(), |d, text| d.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.tabs.activate(ix);
                this.tab_changed(cx);
            }))
            .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, _, cx| {
                this.tabs.close(ix);
                this.tab_changed(cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| {
                this.tabs.activate(ix);
                this.tab_changed(cx);
            }))
            .context_menu(move |menu, _, _| {
                menu.menu("Close", Box::new(CloseTab))
                    .menu("Close others", Box::new(CloseOtherTabs))
                    .menu("Close to the right", Box::new(CloseTabsToRight))
                    .menu("Close all", Box::new(CloseAllTabs))
                    .separator()
                    .menu(if pinned { "Unpin" } else { "Pin" }, Box::new(TogglePinTab))
                    .separator()
                    .menu("Copy name", Box::new(CopyTabName))
                    .menu("Reveal in explorer", Box::new(RevealTab))
            })
            .child(icon.small())
            .when(!pinned, |d| {
                d.child(div().when(t.preview, |d| d.italic()).child(t.title.clone()))
                    // Text, not only the red wash, so production reads without color vision.
                    .when(production, |d| d.child(theme::prod_badge(theme)))
                    .child(
                    div()
                        .id(("close-tab", ix))
                        .rounded(theme::RADIUS_SM)
                        .p(theme::SPACE_2)
                        .text_color(if active { theme.muted_foreground } else { transparent_black() })
                        .group_hover(group, |d| d.text_color(theme.muted_foreground))
                        .hover(|d| d.bg(theme.secondary_hover).text_color(theme.foreground))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.tabs.close(ix);
                            this.tab_changed(cx);
                        }))
                        .child(Icon::new(IconName::Close).xsmall()),
                )
            })
            .into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A vertical wheel scrolls the tab strip sideways, like VS Code.
        let tabs = div().on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
            let delta = ev.delta.pixel_delta(window.line_height());
            let max = this.tab_scroll.max_offset().x.max(Pixels::ZERO);
            let mut offset = this.tab_scroll.offset();
            offset.x = (offset.x + delta.y).clamp(-max, Pixels::ZERO);
            this.tab_scroll.set_offset(offset);
            cx.notify();
        }))
        .child(self.tab_bar(cx));
        if std::mem::take(&mut self.focus_content) {
            let handle = match self.tabs.active().map(|t| &t.content) {
                Some(TabContent::Grid(grid)) => Some(grid.focus_handle(cx)),
                Some(TabContent::Console) => Some(self.editor.focus_handle(cx)),
                None => None,
            };
            if let Some(handle) = handle {
                window.focus(&handle, cx);
            }
        }
        let theme = cx.theme();
        let content = match self.tabs.active().map(|t| &t.content) {
            Some(TabContent::Grid(grid)) => grid.clone().into_any_element(),
            Some(TabContent::Console) => {
                let production = self.tabs.active().and_then(|t| t.origin.as_ref()).is_some_and(|o| o.production);
                // ponytail: a rule, not the full background tint QX-012 brings with the console toolbar
                h_flex()
                    .size_full()
                    .when(production, |d| d.child(div().flex_none().w(theme::STRIPE).h_full().bg(theme.danger)))
                    .child(div().flex_1().h_full().child(Editor::new(&self.editor).bordered(false).h_full()))
                    .into_any_element()
            }
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(theme::SPACE_8)
                .text_size(theme::UI_SM)
                .text_color(theme.muted_foreground)
                .child(div().text_size(theme::UI).text_color(theme::text_secondary(theme)).child("Nothing open."))
                .child("Pick a table in the explorer (Ctrl+Alt+F) · Ctrl+Shift+T brings the last tab back")
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_size(theme::UI)
            .text_color(theme.foreground)
            .on_action(cx.listener(|this, _: &ToggleExplorer, _, cx| {
                this.layout.maximized_from = None;
                this.layout.explorer_visible = !this.layout.explorer_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleBottomPanel, _, cx| {
                this.layout.maximized_from = None;
                this.layout.bottom_visible = !this.layout.bottom_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MaximizeEditor, _, cx| {
                this.layout.toggle_maximize();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                this.tabs.close(this.tabs.active);
                this.tab_changed(cx);
            }))
            .on_action(cx.listener(|this, _: &CloseOtherTabs, _, cx| {
                let active = this.tabs.active;
                this.tabs.close_where(|ix| ix != active);
                this.tab_changed(cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTabsToRight, _, cx| {
                let active = this.tabs.active;
                this.tabs.close_where(|ix| ix > active);
                this.tab_changed(cx);
            }))
            .on_action(cx.listener(|this, _: &CloseAllTabs, _, cx| {
                this.tabs.close_where(|_| true);
                this.tab_changed(cx);
            }))
            .on_action(cx.listener(|this, _: &TogglePinTab, _, cx| {
                this.tabs.toggle_pin(this.tabs.active);
                this.tab_changed(cx);
            }))
            .on_action(cx.listener(|this, _: &CopyTabName, _, cx| {
                if let Some(tab) = this.tabs.active() {
                    cx.write_to_clipboard(ClipboardItem::new_string(tab.title.to_string()));
                }
            }))
            .on_action(cx.listener(|this, _: &RevealTab, window, cx| {
                // Only explorer-opened tabs have a tooltip path; their key is the tree key.
                let Some(key) = this.tabs.active().filter(|t| t.tooltip.is_some()).map(|t| t.key.clone()) else { return };
                this.layout.explorer_visible = true;
                this.explorer.update(cx, |explorer, cx| explorer.reveal(&key, window, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ReopenTab, _, cx| {
                this.tabs.reopen();
                this.tab_changed(cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                this.tabs.cycle(true);
                this.tab_changed(cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| {
                this.tabs.cycle(false);
                this.tab_changed(cx);
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let m = ev.keystroke.modifiers;
                if let Some(n) = ev.keystroke.key.parse::<usize>().ok().filter(|n| m.control && !m.shift && (1..=9).contains(n)) {
                    this.tabs.activate(n - 1);
                    this.tab_changed(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusExplorer, window, cx| {
                this.layout.explorer_visible = true;
                window.focus(&this.explorer.focus_handle(cx), cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CycleTheme, window, cx| {
                this.theme = this.theme.next();
                this.theme.apply(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleGallery, _, cx| {
                this.gallery_visible = !this.gallery_visible;
                cx.notify();
            }))
            .child(
                TitleBar::new().h(theme::TITLEBAR).child(
                    h_flex()
                        .w_full()
                        .pr(theme::SPACE_8)
                        .justify_between()
                        // Wordmark: a console prompt with its cursor parked. Static, nothing moves when idle.
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(1.))
                                .font_family(theme.mono_font_family.clone())
                                .font_weight(FontWeight::BOLD)
                                .text_size(theme::UI)
                                .child("queryx")
                                .child(div().w(theme::SPACE_6 + px(1.)).h(theme::UI).bg(theme.primary)),
                        )
                        .child(self.settings_menu(cx)),
                ),
            )
            .child(div().flex_1().min_h_0().map(|main| if self.gallery_visible {
                main.child(gallery::gallery(cx))
            } else { main.child(
                h_resizable("main")
                    .with_state(&self.layout.main_split)
                    .with_handle_appearance(splitter())
                    .child(
                        resizable_panel()
                            .visible(self.layout.explorer_visible)
                            .size(px(260.))
                            .size_range(px(180.)..px(600.))
                            .flex_none()
                            .child(self.explorer.clone()),
                    )
                    .child(
                        resizable_panel().child(
                            v_resizable("editor-and-panel")
                                .with_state(&self.layout.editor_split)
                                .with_handle_appearance(splitter())
                                .child(resizable_panel().child(v_flex().size_full().child(tabs).child(content)))
                                .child(
                                    resizable_panel()
                                        .visible(self.layout.bottom_visible)
                                        .size(px(240.))
                                        // ponytail: 70% of the window, not of the editor column; close enough at the 600px minimum
                                        .size_range(px(80.)..window.viewport_size().height * 0.7)
                                        .flex_none()
                                        .child(self.bottom_panel.clone()),
                                ),
                        ),
                    ),
            )}))
            .child(self.status_bar.clone())
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            // Mock round-trip time for every fake connect, load and query, to see loading states.
            if let Ok(ms) = std::env::var("QUERYX_MOCK_LATENCY_MS") {
                match ms.parse() {
                    Ok(ms) => mock::set_latency(ms),
                    Err(_) => eprintln!("QUERYX_MOCK_LATENCY_MS={ms:?} is not a whole number of milliseconds; using 0"),
                }
            }
            theme::init(cx);
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
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(window, cx)))
            .expect("open window");
            cx.activate(true);
        });
}
