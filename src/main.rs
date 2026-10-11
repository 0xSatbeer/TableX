use gpui_kit::component::{
    ActiveTheme as _, Icon,
    button::{Button, ButtonVariants as _}, IconName, Selectable as _, Sizable as _, TitleBar, h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    popover::Popover,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;

use std::rc::Rc;

mod assets;
mod connection_dialog;
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

actions!(queryx, [Quit, ToggleExplorer, ToggleBottomPanel, MaximizeEditor, MaximizePanel, RestorePanel, ToggleGallery, FocusExplorer, CloseTab, ReopenTab, NextTab, PrevTab, CloseOtherTabs, CloseTabsToRight, CloseAllTabs, TogglePinTab, CopyTabName, RevealTab, CycleTheme, AddConnection, EditConnection]);

enum TabContent {
    Grid(Entity<grid::Grid>),
    Console,
}

/// Grab band of a splitter on each side of its 1px line.
const SPLITTER_PAD: Pixels = px(4.);

/// The panel edge a splitter moves.
#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Explorer,
    Bottom,
}

/// A splitter drag in progress: where the pointer went down, the panel's drawn size at that
/// moment, and its stored size, which can be larger than drawn when the window is short.
struct Dragging {
    edge: Edge,
    from: Point<Pixels>,
    start: Pixels,
    stored: Pixels,
}

/// Panel visibility and pixel sizes in one place, so QX-020 can persist it.
/// Hidden panels keep their size, so showing one again brings it back as it was.
struct LayoutState {
    explorer_visible: bool,
    bottom_visible: bool,
    /// Visibility of (explorer, bottom) from before `MaximizeEditor`, restored by the next one.
    maximized_from: Option<(bool, bool)>,
    /// The bottom panel covers the editor area; sizes stay as they were, so un-maximizing restores them.
    panel_maximized: bool,
    explorer: split::Split,
    bottom: split::Split,
    dragging: Option<Dragging>,
}

impl LayoutState {
    fn split(&mut self, edge: Edge) -> &mut split::Split {
        match edge {
            Edge::Explorer => &mut self.explorer,
            Edge::Bottom => &mut self.bottom,
        }
    }

    /// The size `drag` asks for with the pointer at `to`, and the window extent along its axis.
    fn requested(drag: &Dragging, to: Point<Pixels>, window: Size<Pixels>) -> (Pixels, Pixels) {
        match drag.edge {
            Edge::Explorer => (drag.start + (to.x - drag.from.x), window.width),
            Edge::Bottom => (drag.start - (to.y - drag.from.y), window.height),
        }
    }

    /// Starts a drag of `edge` from the panel's drawn size.
    fn press(&mut self, edge: Edge, at: Point<Pixels>, window: Size<Pixels>) {
        let extent = if edge == Edge::Explorer { window.width } else { window.height };
        let split = self.split(edge);
        let (start, stored) = (split.shown(extent), split.size);
        self.dragging = Some(Dragging { edge, from: at, start, stored });
    }

    /// Follows the pointer, clamped; going below the minimum only hides on release.
    fn drag_to(&mut self, to: Point<Pixels>, window: Size<Pixels>) {
        let Some(drag) = &self.dragging else { return };
        let (size, extent) = Self::requested(drag, to, window);
        let edge = drag.edge;
        let split = self.split(edge);
        split.size = size.clamp(split.min, (split.max)(extent));
    }

    fn release(&mut self, at: Point<Pixels>, window: Size<Pixels>) {
        let Some(drag) = self.dragging.take() else { return };
        let (size, extent) = Self::requested(&drag, at, window);
        let split = self.split(drag.edge);
        // A click is not a resize: keep the stored size instead of the drawn one.
        if at == drag.from {
            split.size = drag.stored;
            return;
        }
        match split::resolve_drag(size, split.min, (split.max)(extent), drag.start) {
            split::Drag::Size(size) => split.size = size,
            split::Drag::Hidden(size) => {
                split.size = size;
                match drag.edge {
                    Edge::Explorer => self.explorer_visible = false,
                    Edge::Bottom => self.bottom_visible = false,
                }
            }
        }
    }

    fn toggle_explorer(&mut self) {
        self.maximized_from = None;
        self.explorer_visible = !self.explorer_visible;
    }

    /// Ctrl+Alt+F / reveal: shows the explorer if hidden; never hides it.
    fn show_explorer(&mut self) {
        self.maximized_from = None;
        self.explorer_visible = true;
    }

    fn toggle_bottom(&mut self) {
        self.maximized_from = None;
        self.panel_maximized = false;
        self.bottom_visible = !self.bottom_visible;
    }

    /// Maximize button on the panel header; does nothing while the panel is hidden.
    fn toggle_panel_maximize(&mut self) {
        self.panel_maximized = self.bottom_visible && !self.panel_maximized;
    }

    /// Esc: leaves panel maximize; does nothing otherwise.
    fn restore_panel(&mut self) {
        self.panel_maximized = false;
    }

    fn toggle_maximize(&mut self) {
        self.panel_maximized = false;
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
    /// The one copy of the workspace's connections; explorer, panel and tabs follow it.
    workspace: Entity<model::Workspace>,
    explorer: Entity<explorer::Explorer>,
    editor: Entity<EditorState>,
    tabs: tabs::Tabs<TabContent>,
    tab_scroll: ScrollHandle,
    layout: LayoutState,
    status_bar: Entity<statusbar::StatusBar>,
    bottom_panel: Entity<panel::BottomPanel>,
    /// Keeps focus under the shell's key context when maximize unmounts the focused editor, so Esc reaches it.
    focus_handle: FocusHandle,
    gallery_visible: bool,
    /// Set when a tab is activated; the next render moves keyboard focus into its content.
    focus_content: bool,
    theme: theme::ThemeChoice,
    /// Themes loaded from the `UserThemeDir`, listed under the built-ins.
    user_themes: Vec<Rc<gpui_kit::component::theme::ThemeConfig>>,
    /// Problems last notified for the `UserThemeDir`.
    user_theme_errors: Vec<String>,
    /// Overflow list: open state and its search field (shown above `tabs::SEARCH_MIN_TABS` tabs).
    overflow_open: bool,
    overflow_search: Entity<InputState>,
    /// The console editor's mono font size, before UI scale (Ctrl+= / Ctrl+- / Ctrl+0).
    console_mono: Pixels,
    _appearance: Subscription,
}

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = cx.new(|_| mock::workspaces().remove(0));
        let console_origin = workspace.read(cx).connections.iter().find(|c| c.is_production).map(tabs::Origin::of);
        let explorer = cx.new(|cx| explorer::Explorer::new(workspace.clone(), window, cx));
        cx.observe(&workspace, |this, workspace, cx| {
            this.tabs.refresh_origins(workspace.read(cx));
            let (origin, has_tab) = (this.tabs.active().and_then(|t| t.origin.clone()), this.tabs.active().is_some());
            this.status_bar.update(cx, |bar, cx| bar.set_origin(origin, has_tab, cx));
            cx.notify();
        })
        .detach();
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
        let overflow_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search tabs…"));
        cx.subscribe(&overflow_search, |this, search, ev: &InputEvent, cx| match ev {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => {
                let query = search.read(cx).value();
                if let Some(ix) = tabs::first_match(&this.tabs.tabs, &query) {
                    this.overflow_open = false;
                    this.tabs.activate(ix);
                    this.tab_changed(cx);
                }
            }
            _ => {}
        })
        .detach();
        let mut this = Self {
            overflow_open: false,
            overflow_search,
            workspace: workspace.clone(),
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
                    origin: console_origin,
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
                panel_maximized: false,
                explorer: split::EXPLORER,
                bottom: split::BOTTOM,
                dragging: None,
            },
            status_bar: cx.new(|cx| statusbar::StatusBar::new(window, cx)),
            bottom_panel: cx.new(|cx| panel::BottomPanel::new(workspace.clone(), window, cx)),
            focus_handle: cx.focus_handle(),
            gallery_visible: false,
            focus_content: false,
            theme: theme::ThemeChoice::default(),
            user_themes: Vec::new(),
            user_theme_errors: Vec::new(),
            console_mono: theme::MONO,
            _appearance: cx.observe_window_appearance(window, |this, window, cx| this.theme.apply(&this.user_themes, window, cx)),
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
        let grid = match self.tabs.active().map(|t| &t.content) {
            Some(TabContent::Grid(grid)) => Some(grid.clone()),
            _ => None,
        };
        let (origin, has_tab) = (self.tabs.active().and_then(|t| t.origin.clone()), self.tabs.active().is_some());
        self.status_bar.update(cx, |bar, cx| {
            bar.set_editor(editor, cx);
            bar.set_grid(grid.as_ref(), cx);
            bar.set_origin(origin, has_tab, cx);
        });
        self.focus_content = true;
        // Without a search field the list is a plain menu; don't re-mount open with a stale query.
        self.overflow_open &= tabs::show_search(self.tabs.tabs.len());
        cx.notify();
    }

    /// The size the console editor is given: its zoom at the current UI scale.
    fn console_text_size(&self) -> Pixels {
        theme::scaled(self.console_mono)
    }

    fn zoom_console(&mut self, step: theme::Zoom, cx: &mut Context<Self>) {
        self.console_mono = theme::zoom(self.console_mono, step);
        cx.notify();
    }

    /// Loads user themes from `dir` and reloads them when its files change. Never writes to `dir`.
    fn watch_user_themes(&mut self, dir: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // shortcut: polls the dir's listing once a second instead of a file watcher. gpui-kit's
        // `ThemeRegistry::watch_dir` creates the dir and drops our built-ins on reload, and `notify`
        // isn't a dependency; switch to a watcher if polling ever shows up in a profile.
        let mut stamp = theme::user_theme_stamp(&dir);
        // Deferred: this runs while the window is being built, and notifications need its Root.
        let first = dir.clone();
        cx.defer_in(window, move |this, window, cx| this.reload_user_themes(&first, window, cx));
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                let now = theme::user_theme_stamp(&dir);
                let changed = now != stamp;
                stamp = now;
                let alive = this.update_in(cx, |this, window, cx| {
                    if changed {
                        this.reload_user_themes(&dir, window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Lists the themes in `dir`, notifies about skipped files and re-applies the active theme.
    fn reload_user_themes(&mut self, dir: &std::path::Path, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        let (themes, errors) = theme::load_user_themes(dir);
        // Only new problems: an unrelated edit shouldn't repeat every notification.
        for error in errors.iter().filter(|e| !self.user_theme_errors.contains(e)) {
            window.push_notification(Notification::error(error.clone()), cx);
        }
        self.user_theme_errors = errors;
        self.user_themes = themes;
        if let theme::ThemeChoice::User(name) = &self.theme
            && !self.user_themes.iter().any(|t| t.name == *name)
        {
            self.theme = theme::ThemeChoice::Dark;
        }
        self.theme.apply(&self.user_themes, window, cx);
        cx.notify();
    }

    /// ⚙ in the titlebar. Holds the theme and UI scale until there is a settings view (QX-020);
    /// `Ctrl+K Ctrl+T` still cycles the theme from the keyboard.
    fn settings_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let current = self.theme.clone();
        // User themes from the `UserThemeDir`, under the built-ins.
        let user = self.user_themes.iter().map(|t| theme::ThemeChoice::User(t.name.clone()));
        let choices: Vec<_> = theme::ThemeChoice::ALL.into_iter().chain(user).collect();
        Button::new("settings")
            .ghost()
            .small()
            .icon(IconName::Settings)
            .tooltip("Settings")
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.label("Theme");
                for (ix, choice) in choices.iter().cloned().enumerate() {
                    if ix == theme::ThemeChoice::ALL.len() {
                        menu = menu.separator();
                    }
                    let shell = shell.clone();
                    menu = menu.item(PopupMenuItem::new(choice.label()).checked(choice == current).on_click(
                        move |_, window, cx| {
                            _ = shell.update(cx, |this, cx| {
                                this.theme = choice.clone();
                                this.theme.apply(&this.user_themes, window, cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu = menu.separator().label("UI scale");
                for percent in theme::UI_SCALES {
                    menu = menu.item(PopupMenuItem::new(format!("{percent}%")).checked(percent == theme::ui_scale()).on_click(
                        move |_, window, cx| {
                            theme::set_ui_scale(percent, cx);
                            window.refresh();
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
            .h(theme::tab())
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
                    .child(
                        div()
                            .id("tabs-end")
                            .flex_1()
                            .h_full()
                            .border_b_1()
                            .border_color(theme.border)
                            .drag_over::<TabDrag>(|d, _, _, cx| d.border_l_2().border_color(cx.theme().primary))
                            .on_drop(cx.listener(|this, drag: &TabDrag, _, cx| {
                                this.tabs.move_tab(drag.ix, this.tabs.tabs.len());
                                this.tab_changed(cx);
                            })),
                    ),
            )
            .child(
                h_flex().flex_none().h_full().px(theme::space_4()).border_l_1().border_b_1().border_color(theme.border).child(
                    if tabs::show_search(titles.len()) { self.overflow_search_list(cx).into_any_element() } else {
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
                            .anchor(Anchor::TopRight)
                            .into_any_element()
                    },
                ),
            )
    }

    /// The overflow list with a search field, for more than `tabs::SEARCH_MIN_TABS` tabs.
    fn overflow_search_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let search = self.overflow_search.clone();
        let query = search.read(cx).value();
        let empty = tabs::empty_state(&self.tabs.tabs, &query);
        let rows: Vec<_> = tabs::match_tabs(&self.tabs.tabs, &query).into_iter().map(|ix| (ix, self.tabs.tabs[ix].title.clone())).collect();
        let active_ix = self.tabs.active;
        Popover::new("all-tabs-popover")
            .anchor(Anchor::TopRight)
            .open(self.overflow_open)
            .on_open_change({
                let (shell, search) = (shell.downgrade(), search.clone());
                move |open, window, cx| {
                    _ = shell.update(cx, |this, cx| {
                        this.overflow_open = *open;
                        cx.notify();
                    });
                    if *open {
                        search.update(cx, |s, cx| {
                            s.set_value("", window, cx);
                            s.focus(window, cx);
                        });
                    }
                }
            })
            .trigger(Button::new("all-tabs").ghost().xsmall().icon(IconName::ChevronDown).tooltip("All open tabs"))
            .content(move |_, _, cx| {
                let list = if let Some(label) = empty {
                    div().p(theme::space_8()).text_color(cx.theme().muted_foreground).child(label).into_any_element()
                } else {
                    v_flex()
                        .id("overflow-list")
                        .max_h(theme::space_24() * 10.)
                        .overflow_y_scroll()
                        .children(rows.iter().map(|(ix, title)| {
                            let (ix, shell) = (*ix, shell.clone());
                            Button::new(("overflow-tab", ix))
                                .ghost()
                                .small()
                                .label(title.clone())
                                .selected(ix == active_ix)
                                .on_click(move |_, _, cx| {
                                    shell.update(cx, |this, cx| {
                                        this.overflow_open = false;
                                        this.tabs.activate(ix);
                                        this.tab_changed(cx);
                                    });
                                })
                        }))
                        .into_any_element()
                };
                v_flex().key_context(OVERFLOW_CONTEXT).w(theme::space_24() * 10.).gap(theme::space_4()).child(Input::new(&search).small()).child(list)
            })
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
            .pl(theme::space_12())
            .pr(if pinned { theme::space_12() } else { theme::space_6() })
            .gap(theme::space_6())
            .border_r_1()
            .when(!active, |d| d.border_b_1())
            .border_color(theme.border)
            .text_size(theme::ui_sm())
            .map(|d| if active {
                d.bg(theme.tab_active).text_color(theme.tab_active_foreground)
            } else {
                d.text_color(theme.tab_foreground).hover(|d| d.bg(theme.secondary_hover))
            })
            // Tint and stripe are overlays so they sit on top of the active/hover background.
            .when(production, |d| d.child(div().absolute().inset_0().bg(theme::production_tint(theme))))
            .when_some(t.origin.as_ref(), |d, o| {
                d.child(div().absolute().top_0().left_0().right_0().h(theme::stripe()).bg(theme::connection_color(o.color, theme)))
            })
            .tooltip({
                let text = SharedString::from(tabs::tooltip_text(&t.title, t.origin.as_ref()));
                move |window, cx| Tooltip::new(text.clone()).build(window, cx)
            })
            .on_drag(TabDrag { ix, title: t.title.clone() }, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .drag_over::<TabDrag>(|d, _, _, cx| d.border_l_2().border_color(cx.theme().primary))
            .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                this.tabs.move_tab(drag.ix, ix);
                this.tab_changed(cx);
            }))
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
                        .rounded(theme::radius_sm())
                        .p(theme::space_2())
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

/// Drag payload of a tab; also its drag ghost. Dropping on tab `j` means "before `j`".
#[derive(Clone)]
struct TabDrag {
    ix: usize,
    title: SharedString,
}

impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div().px(theme::space_12()).py(theme::space_6()).bg(theme.tab_active).border_1().border_color(theme.border).text_size(theme::ui_sm()).child(self.title.clone())
    }
}

impl Shell {
    /// Focus handle of the active tab's content: its grid or the console editor.
    fn content_focus(&self, cx: &App) -> Option<FocusHandle> {
        match self.tabs.active().map(|t| &t.content) {
            Some(TabContent::Grid(grid)) => Some(grid.focus_handle(cx)),
            Some(TabContent::Console) => Some(self.editor.focus_handle(cx)),
            None => None,
        }
    }

    /// Our own handle over the panel's edge, drawn after the panels so its band sits on top.
    /// `size` is the panel's drawn size; the 1px line covers the panel's own edge pixel.
    fn splitter(&self, edge: Edge, size: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let dragging = self.layout.dragging.as_ref().is_some_and(|d| d.edge == edge);
        let offset = size - px(1.) - SPLITTER_PAD;
        let name = if edge == Edge::Explorer { "splitter-explorer" } else { "splitter-bottom" };
        let line = div().debug_selector(move || name.into()).flex_none().bg(if dragging { theme.primary } else { theme.border });
        div()
            .absolute()
            .occlude()
            .map(|d| match edge {
                Edge::Explorer => d.top_0().h_full().left(offset).px(SPLITTER_PAD).cursor_col_resize().child(line.w(px(1.)).h_full()),
                Edge::Bottom => d.left_0().w_full().bottom(offset).py(SPLITTER_PAD).cursor_row_resize().child(line.h(px(1.)).w_full()),
            })
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if ev.click_count >= 2 {
                    this.layout.dragging = None;
                    this.layout.split(edge).reset();
                } else {
                    this.layout.press(edge, ev.position, window.viewport_size());
                }
                cx.notify();
            }))
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
        // Every way out of maximize refocuses the content: it is unmounted while maximized, so stay pending.
        if self.layout.panel_maximized {
            self.focus_content = true;
        } else if std::mem::take(&mut self.focus_content) {
            if let Some(handle) = self.content_focus(cx) {
                window.focus(&handle, cx);
            }
        }
        let focused = |handle: Option<FocusHandle>, cx: &App| handle.is_some_and(|h| h.contains_focused(window, cx));
        let content_focused = focused(self.content_focus(cx), cx);
        // The console editor shares the content pane; only the grid itself counts for the aggregate.
        let grid_focused = content_focused && matches!(self.tabs.active().map(|t| &t.content), Some(TabContent::Grid(_)));
        self.status_bar.update(cx, |bar, cx| bar.set_grid_focused(grid_focused, cx));
        let explorer_focused = focused(Some(self.explorer.focus_handle(cx)), cx);
        let panel_focused = focused(Some(self.bottom_panel.focus_handle(cx)), cx);
        let theme = cx.theme();
        let content = match self.tabs.active().map(|t| &t.content) {
            Some(TabContent::Grid(grid)) => grid.clone().into_any_element(),
            Some(TabContent::Console) => {
                let production = self.tabs.active().and_then(|t| t.origin.as_ref()).is_some_and(|o| o.production);
                // ponytail: a rule, not the full background tint QX-012 brings with the console toolbar
                h_flex()
                    .size_full()
                    .when(production, |d| d.child(div().flex_none().w(theme::stripe()).h_full().bg(theme.danger)))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .on_action(cx.listener(|this, _: &theme::ZoomIn, _, cx| this.zoom_console(theme::Zoom::In, cx)))
                            .on_action(cx.listener(|this, _: &theme::ZoomOut, _, cx| this.zoom_console(theme::Zoom::Out, cx)))
                            .on_action(cx.listener(|this, _: &theme::ZoomReset, _, cx| this.zoom_console(theme::Zoom::Reset, cx)))
                            .child(Editor::new(&self.editor).bordered(false).h_full().text_size(self.console_text_size())),
                    )
                    .into_any_element()
            }
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(theme::space_8())
                .text_size(theme::ui_sm())
                .text_color(theme.muted_foreground)
                .child(div().text_size(theme::ui()).text_color(theme::text_secondary(theme)).child("Nothing open."))
                .child("Pick a table in the explorer (Ctrl+Alt+F) · Ctrl+Shift+T brings the last tab back")
                .into_any_element(),
        };

        v_flex()
            .relative()
            .size_full()
            // Only while maximized: a tracked root focuses itself on every click on plain chrome.
            .when(self.layout.panel_maximized, |d| d.track_focus(&self.focus_handle).key_context(PANEL_MAXIMIZED_CONTEXT))
            .bg(theme.background)
            .text_size(theme::ui())
            .text_color(theme.foreground)
            .on_action(cx.listener(|this, _: &ToggleExplorer, _, cx| {
                this.layout.toggle_explorer();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleBottomPanel, _, cx| {
                this.layout.toggle_bottom();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MaximizePanel, window, cx| {
                this.layout.toggle_panel_maximize();
                if this.layout.panel_maximized {
                    window.focus(&this.focus_handle, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RestorePanel, _, cx| {
                this.layout.restore_panel();
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
                this.layout.show_explorer();
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
            .on_modifiers_changed(cx.listener(|this, ev: &ModifiersChangedEvent, _, _| {
                if !ev.modifiers.control {
                    this.tabs.commit();
                }
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let m = ev.keystroke.modifiers;
                if let Some(n) = ev.keystroke.key.parse::<usize>().ok().filter(|n| m.control && !m.shift && (1..=9).contains(n)) {
                    this.tabs.activate(n - 1);
                    this.tab_changed(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AddConnection, window, cx| {
                this.explorer.update(cx, |explorer, cx| explorer.add_connection(window, cx));
            }))
            .on_action(cx.listener(|this, _: &EditConnection, window, cx| {
                this.explorer.update(cx, |explorer, cx| explorer.edit_connection(window, cx));
            }))
            .on_action(cx.listener(|this, _: &FocusExplorer, window, cx| {
                this.layout.show_explorer();
                this.explorer.update(cx, |explorer, cx| explorer.focus_field(window, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CycleTheme, window, cx| {
                this.theme = this.theme.next();
                this.theme.apply(&this.user_themes, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleGallery, _, cx| {
                this.gallery_visible = !this.gallery_visible;
                cx.notify();
            }))
            .child(
                TitleBar::new().h(theme::titlebar()).child(
                    h_flex()
                        .w_full()
                        .pr(theme::space_8())
                        .justify_between()
                        // Wordmark: a console prompt with its cursor parked. Static, nothing moves when idle.
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(1.))
                                .font_family(theme.mono_font_family.clone())
                                .font_weight(FontWeight::BOLD)
                                .text_size(theme::ui())
                                .child("queryx")
                                .child(div().w(theme::space_6() + px(1.)).h(theme::ui()).bg(theme.primary)),
                        )
                        .child(self.settings_menu(cx)),
                ),
            )
            .child(div().flex_1().min_h_0().map(|main| if self.gallery_visible {
                main.child(gallery::gallery(cx))
            } else {
                let viewport = window.viewport_size();
                let explorer_w = self.layout.explorer.shown(viewport.width);
                let bottom_h = self.layout.bottom.shown(viewport.height);
                let maximized = self.layout.panel_maximized;
                self.bottom_panel.update(cx, |panel, _| panel.maximized = maximized);
                main.child(
                    h_flex()
                        .relative()
                        .size_full()
                        .when(self.layout.explorer_visible, |d| {
                            // The last 1px column is the splitter line's own, so it never paints over the ring.
                            let pane = div().flex_none().h_full().w(explorer_w - px(1.)).mr(px(1.)).overflow_hidden().child(self.explorer.clone());
                            d.child(theme::focus_ring("pane-explorer", pane, explorer_focused, cx.theme()))
                        })
                        .child(
                            v_flex()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .when(!maximized, |d| d.child(div().flex_1().min_h_0().overflow_hidden().child(v_flex().size_full().child(tabs).child(theme::focus_ring("pane-content", div().flex_1().min_h_0().child(content), content_focused, cx.theme())))))
                                .when(self.layout.bottom_visible, |d| {
                                    let panel = div().w_full().overflow_hidden().child(self.bottom_panel.clone());
                                    let panel = theme::focus_ring("pane-panel", panel, panel_focused, cx.theme());
                                    if maximized {
                                        d.child(panel.flex_1().min_h_0())
                                    } else {
                                        // The top 1px row is the splitter line's own, as with the explorer.
                                        d.child(panel.flex_none().h(bottom_h - px(1.)).mt(px(1.))).child(self.splitter(Edge::Bottom, bottom_h, cx))
                                    }
                                }),
                        )
                        .when(self.layout.explorer_visible, |d| d.child(self.splitter(Edge::Explorer, explorer_w, cx))),
                )
            }))
            .child(self.status_bar.clone())
            // While a splitter is dragged, one layer over everything takes the pointer, so the
            // panels underneath neither react nor swap the cursor.
            .when_some(self.layout.dragging.as_ref().map(|d| d.edge), |d, edge| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .map(|d| if edge == Edge::Explorer { d.cursor_col_resize() } else { d.cursor_row_resize() })
                        .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                            this.layout.drag_to(ev.position, window.viewport_size());
                            cx.notify();
                        }))
                        .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, window, cx| {
                            this.layout.release(ev.position, window.viewport_size());
                            cx.notify();
                        })),
                )
            })
    }
}

/// Key context of the shell while the bottom panel is maximized, so Esc restores it.
const PANEL_MAXIMIZED_CONTEXT: &str = "PanelMaximized";

/// Key context of the overflow list's content.
const OVERFLOW_CONTEXT: &str = "OverflowSearch";

/// `Popover` binds space to Confirm (closing it), which would swallow spaces typed in the search
/// field; a no-op binding in the content's own, deeper context lets the field type them.
fn bind_overflow_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("space", NoAction, Some(OVERFLOW_CONTEXT))]);
}

/// Zoom the focused grid's or editor's mono font; with neither focused nothing handles them.
fn bind_zoom_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-=", theme::ZoomIn, None),
        KeyBinding::new("ctrl--", theme::ZoomOut, None),
        KeyBinding::new("ctrl-0", theme::ZoomReset, None),
    ]);
}

fn bind_panel_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", RestorePanel, Some(PANEL_MAXIMIZED_CONTEXT))]);
}

/// Opens the main window and starts watching `user_themes` (the user theme dir), if any.
fn open_shell(options: WindowOptions, user_themes: Option<std::path::PathBuf>, cx: &mut App) -> Result<(AnyWindowHandle, Entity<Shell>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let mut shell = Shell::new(window, cx);
            if let Some(dir) = user_themes {
                shell.watch_user_themes(dir, window, cx);
            }
            shell
        })
    })
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
            bind_overflow_keys(cx);
            bind_panel_keys(cx);
            bind_zoom_keys(cx);
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
            open_shell(options, user_themes.map(|t| t.dir), cx).expect("open window");
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::{Edge, LayoutState, split};
    use gpui_kit::{point, px, size};

    fn layout() -> LayoutState {
        LayoutState {
            explorer_visible: true,
            bottom_visible: true,
            maximized_from: None,
            panel_maximized: false,
            explorer: split::EXPLORER,
            bottom: split::BOTTOM,
            dragging: None,
        }
    }

    #[test]
    fn explorer_released_below_min_hides_and_ctrl_b_shows_it_at_the_pre_drag_width() {
        let window = size(px(1200.), px(800.));
        let mut l = layout();
        l.press(Edge::Explorer, point(px(260.), px(300.)), window);
        l.drag_to(point(px(179.), px(300.)), window);
        l.release(point(px(179.), px(300.)), window);
        assert!(!l.explorer_visible, "explorer hidden after release at 179");
        assert_eq!(l.explorer.size, px(260.), "stored explorer width after release at 179");
        l.toggle_explorer();
        assert!(l.explorer_visible, "explorer visible after Ctrl+B");
        assert_eq!(l.explorer.shown(window.width), px(260.), "explorer width after Ctrl+B");
    }

    #[test]
    fn bottom_click_without_moving_keeps_its_stored_height() {
        let small = size(px(1200.), px(600.));
        let mut l = layout();
        l.bottom.size = px(500.);
        let at = point(px(600.), px(180.));
        l.press(Edge::Bottom, at, small);
        l.release(at, small);
        assert_eq!(l.bottom.size, px(500.), "stored bottom height after a click at window height 600");
        assert_eq!(l.bottom.shown(px(800.)), px(500.), "bottom height after the window grows to 800");
    }

    #[test]
    fn ctrl_alt_f_shows_a_hidden_explorer_and_never_hides_it() {
        let mut l = layout();
        l.explorer_visible = false;
        l.show_explorer();
        assert!(l.explorer_visible, "hidden -> shown");
        l.show_explorer();
        assert!(l.explorer_visible, "visible stays visible");
        l.toggle_maximize();
        assert!(!l.explorer_visible, "maximized hides it");
        l.show_explorer();
        assert!(l.explorer_visible, "maximized -> shown");
        assert!(l.maximized_from.is_none(), "maximize state dropped so the next toggle starts fresh");
    }

    #[test]
    fn panel_maximize_toggles_and_keeps_sizes() {
        let window = size(px(1200.), px(800.));
        let mut l = layout();
        l.explorer.size = px(300.);
        l.bottom.size = px(333.);
        l.toggle_panel_maximize();
        assert!(l.panel_maximized, "maximized after the first press");
        assert!(l.bottom_visible, "panel stays visible when maximized");
        l.toggle_panel_maximize();
        assert!(!l.panel_maximized, "restored after the second press");
        l.toggle_panel_maximize();
        l.restore_panel();
        assert!(!l.panel_maximized, "Esc restores");
        l.restore_panel();
        assert!(!l.panel_maximized, "Esc when not maximized does nothing");
        assert_eq!((l.explorer.shown(window.width), l.bottom.shown(window.height)), (px(300.), px(333.)), "previous sizes restored");
        assert!(l.explorer_visible, "explorer visibility untouched");
    }

    #[test]
    fn panel_maximize_needs_a_visible_panel_and_ends_when_it_hides() {
        let mut l = layout();
        l.bottom_visible = false;
        l.toggle_panel_maximize();
        assert!(!l.panel_maximized, "hidden panel can't be maximized");
        l.bottom_visible = true;
        l.toggle_panel_maximize();
        assert!(l.panel_maximized, "visible panel maximizes");
        l.toggle_bottom();
        assert!(!l.bottom_visible && !l.panel_maximized, "Ctrl+J while maximized hides the panel and ends maximize");
        l.toggle_bottom();
        assert!(l.bottom_visible && !l.panel_maximized, "panel comes back at its normal size");
        l.toggle_panel_maximize();
        assert!(l.panel_maximized, "maximized again");
        l.toggle_maximize();
        assert!(!l.bottom_visible && !l.panel_maximized, "Ctrl+Shift+F while maximized hides everything and ends maximize");
    }

    /// A Shell with the panel maximized, on the Logs tab with its filter field focused if `logs_focused`.
    fn open_maximized(cx: &mut gpui_kit::TestAppContext, logs_focused: bool) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<super::Shell>) {
        use gpui_kit::AppContext as _;
        cx.update(|cx| {
            gpui_kit::init(cx);
            super::bind_panel_keys(cx);
        });
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        if logs_focused {
            shell.update(&mut vcx, |this, cx| this.bottom_panel.update(cx, |p, _| p.show_logs()));
        }
        vcx.run_until_parked();
        // As the header's maximize button does.
        vcx.dispatch_action(super::MaximizePanel);
        vcx.run_until_parked();
        if logs_focused {
            let search = shell.read_with(&vcx, |this, cx| this.bottom_panel.read(cx).search_field());
            vcx.update(|window, cx| search.update(cx, |s, cx| s.focus(window, cx)));
            vcx.run_until_parked();
        }
        (window.into(), shell)
    }

    #[gpui_kit::test]
    fn esc_while_maximized_restores_the_panel(cx: &mut gpui_kit::TestAppContext) {
        for logs_focused in [false, true] {
            let (window, shell) = open_maximized(cx, logs_focused);
            let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
            let sizes = |shell: &gpui_kit::Entity<super::Shell>, cx: &gpui_kit::VisualTestContext| {
                shell.read_with(cx, |this, _| (this.layout.explorer.size, this.layout.bottom.size))
            };
            let before = sizes(&shell, &cx);
            assert!(shell.read_with(&cx, |this, _| this.layout.panel_maximized), "maximized (logs_focused={logs_focused})");
            cx.simulate_keystrokes("escape");
            assert!(!shell.read_with(&cx, |this, _| this.layout.panel_maximized), "Esc restores (logs_focused={logs_focused})");
            assert_eq!(sizes(&shell, &cx), before, "stored sizes unchanged (logs_focused={logs_focused})");
            let root = shell.read_with(&cx, |this, _| this.focus_handle.clone());
            assert!(!cx.update(|window, _| root.is_focused(window)), "content refocused after restore (logs_focused={logs_focused})");
        }
    }

    #[gpui_kit::test]
    fn tab_change_while_maximized_keeps_esc_restoring_the_panel(cx: &mut gpui_kit::TestAppContext) {
        for logs_focused in [false, true] {
            let (window, shell) = open_maximized(cx, logs_focused);
            let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
            cx.dispatch_action(super::NextTab);
            cx.run_until_parked();
            cx.simulate_keystrokes("escape");
            assert!(!shell.read_with(&cx, |this, _| this.layout.panel_maximized), "Esc restores after a tab change (logs_focused={logs_focused})");
        }
    }

    #[gpui_kit::test]
    fn every_way_out_of_panel_maximize_refocuses_the_content(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::Focusable as _;
        // The bottom splitter isn't drawn while maximized, so a drag can't hide the panel from there.
        let exits: [(&str, fn(&mut gpui_kit::VisualTestContext)); 4] = [
            ("Esc", |cx| cx.simulate_keystrokes("escape")),
            ("maximize button again", |cx| cx.dispatch_action(super::MaximizePanel)),
            ("Ctrl+J / hide panel button", |cx| cx.dispatch_action(super::ToggleBottomPanel)),
            ("Ctrl+Shift+F", |cx| cx.dispatch_action(super::MaximizeEditor)),
        ];
        for (exit, run) in exits {
            for logs_focused in [false, true] {
                let (window, shell) = open_maximized(cx, logs_focused);
                let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
                run(&mut cx);
                cx.run_until_parked();
                let (maximized, root, content) = shell.read_with(&cx, |this, cx| {
                    let content = match this.tabs.active().map(|t| &t.content) {
                        Some(super::TabContent::Grid(grid)) => grid.focus_handle(cx),
                        Some(super::TabContent::Console) => this.editor.focus_handle(cx),
                        None => panic!("launch opens a tab"),
                    };
                    (this.layout.panel_maximized, this.focus_handle.clone(), content)
                });
                assert!(!maximized, "{exit} ends maximize (logs_focused={logs_focused})");
                assert!(!cx.update(|window, _| root.is_focused(window)), "{exit} leaves the root (logs_focused={logs_focused})");
                assert!(cx.update(|window, _| content.is_focused(window)), "{exit} focuses the content (logs_focused={logs_focused})");
            }
        }
    }

    #[gpui_kit::test]
    fn clicking_chrome_keeps_focus_when_not_maximized(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Focusable as _};
        cx.update(|cx| gpui_kit::init(cx));
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let editor = shell.read_with(&cx, |this, cx| this.editor.focus_handle(cx));
        cx.update(|window, cx| window.focus(&editor, cx));
        let bottom_middle = cx.update(|window, _| {
            let v = window.viewport_size();
            gpui_kit::point(v.width / 2., v.height - gpui_kit::px(4.))
        });
        cx.simulate_click(bottom_middle, Default::default());
        assert!(cx.update(|window, _| editor.is_focused(window)), "a click on non-focusable chrome must not steal focus");
    }

    #[gpui_kit::test]
    fn zoom_keys_change_only_the_focused_grid_or_editor(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Focusable as _, px};
        cx.update(|cx| {
            gpui_kit::init(cx);
            super::bind_zoom_keys(cx);
        });
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let grid = shell.read_with(&cx, |this, _| match this.tabs.active().map(|t| &t.content) {
            Some(super::TabContent::Grid(grid)) => grid.clone(),
            _ => panic!("launch opens on a grid"),
        });
        let sizes = |cx: &gpui_kit::VisualTestContext| shell.read_with(cx, |this, cx| (grid.read(cx).mono(), this.console_mono));
        let steps: [(&str, &str, (f32, f32)); 7] = [
            ("grid", "ctrl-=", (13.5, 12.5)),
            ("grid", "ctrl-0", (12.5, 12.5)),
            ("grid", "ctrl--", (11.5, 12.5)),
            ("editor", "ctrl-=", (11.5, 13.5)),
            ("editor", "ctrl-0", (11.5, 12.5)),
            ("explorer", "ctrl-=", (11.5, 12.5)),
            ("nothing", "ctrl-=", (11.5, 12.5)),
        ];
        for (focus, keys, (grid_mono, console_mono)) in steps {
            if focus == "editor" {
                // The console is only mounted while its tab is active.
                shell.update(&mut cx, |this, cx| {
                    let console = this.tabs.tabs.iter().position(|t| matches!(t.content, super::TabContent::Console)).expect("launch opens a console");
                    this.tabs.activate(console);
                    this.tab_changed(cx);
                });
                cx.run_until_parked();
            }
            let handle = shell.read_with(&cx, |this, cx| match focus {
                "grid" => Some(grid.focus_handle(cx)),
                "editor" => Some(this.editor.focus_handle(cx)),
                "explorer" => Some(this.explorer.focus_handle(cx)),
                _ => None,
            });
            cx.update(|window, cx| match &handle {
                Some(handle) => window.focus(handle, cx),
                None => window.blur(cx),
            });
            cx.simulate_keystrokes(keys);
            assert_eq!(sizes(&cx), (px(grid_mono), px(console_mono)), "{keys} with {focus} focused");
        }
    }

    #[gpui_kit::test]
    fn console_editor_follows_ui_scale_like_the_grid(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Focusable as _};
        cx.update(|cx| {
            gpui_kit::init(cx);
            super::bind_zoom_keys(cx);
            crate::theme::set_ui_scale(130, cx);
        });
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        shell.update(&mut cx, |this, cx| {
            let console = this.tabs.tabs.iter().position(|t| matches!(t.content, super::TabContent::Console)).expect("launch opens a console");
            this.tabs.activate(console);
            this.tab_changed(cx);
        });
        cx.run_until_parked();
        let editor = shell.read_with(&cx, |this, cx| this.editor.focus_handle(cx));
        cx.update(|window, cx| window.focus(&editor, cx));
        for (keys, want) in [("", 16.25), ("ctrl-=", 17.55)] {
            if !keys.is_empty() {
                cx.simulate_keystrokes(keys);
            }
            let got: f32 = shell.read_with(&cx, |this, _| this.console_text_size()).into();
            assert!((got - want).abs() < 0.001, "after {keys:?} at 130%: console {got}px, want {want}px");
        }
    }

    /// Edits to the active user theme's file apply without a restart; the theme dir is only read.
    #[gpui_kit::test]
    fn editing_the_active_user_theme_applies_without_restart(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, component::ActiveTheme as _};
        let theme_json = |bg: &str| format!(r##"{{"name": "Nord", "themes": [{{"name": "Nord", "mode": "dark", "colors": {{"background": "{bg}"}}}}]}}"##);
        let dir = crate::theme::tests::test_dir("watch");
        let missing = crate::theme::tests::test_dir("watch-missing").join("themes");
        std::fs::write(dir.join("nord.json"), theme_json("#2E3440")).unwrap();
        cx.update(|cx| gpui_kit::init(cx));
        let open = |dir: std::path::PathBuf, cx: &mut gpui_kit::TestAppContext| {
            let (window, shell) = cx.update(|cx| {
                gpui_kit::open_window(Default::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        let mut shell = super::Shell::new(window, cx);
                        shell.watch_user_themes(dir, window, cx);
                        shell
                    })
                })
                .expect("open test window")
            });
            cx.run_until_parked();
            (window, shell)
        };
        let (window, shell) = open(dir.clone(), cx);
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| {
            shell.update(cx, |this, cx| {
                this.theme = crate::theme::ThemeChoice::User("Nord".into());
                this.theme.apply(&this.user_themes, window, cx);
            })
        });
        let background = |vcx: &mut gpui_kit::VisualTestContext| vcx.update(|_, cx| cx.theme().background);
        assert_eq!(background(&mut vcx), gpui_kit::rgb(0x2E3440).into(), "Nord applied");

        std::fs::write(dir.join("nord.json"), format!("{} ", theme_json("#3B4252"))).unwrap(); // longer too, so the change doesn't hinge on mtime resolution
        vcx.executor().advance_clock(std::time::Duration::from_secs(5));
        vcx.run_until_parked();
        assert_eq!(background(&mut vcx), gpui_kit::rgb(0x3B4252).into(), "edited Nord applied without restart");
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files, ["nord.json"], "the theme dir is only read");

        open(missing.clone(), cx);
        cx.executor().advance_clock(std::time::Duration::from_secs(5));
        cx.run_until_parked();
        assert!(!missing.exists(), "a missing theme dir is not created");
    }

    /// An unrelated edit doesn't repeat a known problem; deleting the active theme's file falls back to Dark.
    #[gpui_kit::test]
    fn reloading_user_themes_repeats_no_notification_and_falls_back_when_the_file_is_gone(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, component::WindowExt as _};
        let nord = |bg: &str| format!(r##"{{"name": "Nord", "themes": [{{"name": "Nord", "mode": "dark", "colors": {{"background": "{bg}"}}}}]}}"##);
        let dir = crate::theme::tests::test_dir("reload");
        std::fs::write(dir.join("bad.json"), "not json").unwrap();
        std::fs::write(dir.join("nord.json"), nord("#2E3440")).unwrap();
        cx.update(|cx| gpui_kit::init(cx));
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        // After the window exists: notifications need its root.
        vcx.update(|window, cx| shell.update(cx, |this, cx| this.watch_user_themes(dir.clone(), window, cx)));
        vcx.run_until_parked();
        let count = |vcx: &mut gpui_kit::VisualTestContext| vcx.update(|window, cx| window.notifications(cx).len());
        assert_eq!(count(&mut vcx), 1, "bad.json reported once on load");

        // (a) an unrelated edit reloads, but bad.json is not reported again.
        vcx.update(|window, cx| window.clear_notifications(cx));
        std::fs::write(dir.join("nord.json"), format!("{} ", nord("#3B4252"))).unwrap();
        vcx.executor().advance_clock(std::time::Duration::from_secs(2));
        vcx.run_until_parked();
        assert_eq!(count(&mut vcx), 0, "no repeat notification for bad.json");

        // (b) the active user theme's file is deleted: back to Dark.
        shell.update(&mut vcx, |this, _| this.theme = crate::theme::ThemeChoice::User("Nord".into()));
        std::fs::remove_file(dir.join("nord.json")).unwrap();
        vcx.executor().advance_clock(std::time::Duration::from_secs(2));
        vcx.run_until_parked();
        assert_eq!(shell.read_with(&vcx, |this, _| this.theme.clone()), crate::theme::ThemeChoice::Dark);
    }

    /// A broken theme file present at launch is reported once, not a crash.
    #[gpui_kit::test]
    fn broken_theme_file_at_startup_is_reported_not_a_crash(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::component::WindowExt as _;
        let dir = crate::theme::tests::test_dir("startup-bad");
        std::fs::write(dir.join("bad.json"), "not json").unwrap();
        cx.update(|cx| gpui_kit::init(cx));
        let (window, shell) = cx.update(|cx| super::open_shell(Default::default(), Some(dir), cx)).expect("open test window");
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.run_until_parked();
        assert_eq!(vcx.update(|window, cx| window.notifications(cx).len()), 1, "one notification");
        let errors = shell.read_with(&vcx, |this, _| this.user_theme_errors.clone());
        assert!(matches!(&errors[..], [e] if e.starts_with("Couldn't load theme bad.json: ")), "{errors:?}");
    }

    /// A Shell with 11 tabs and the overflow list open, its search field focused.
    fn open_overflow(cx: &mut gpui_kit::TestAppContext) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<super::Shell>) {
        use gpui_kit::AppContext as _;
        cx.update(|cx| {
            gpui_kit::init(cx);
            super::bind_overflow_keys(cx);
        });
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let _ = cx.update_window(window.into(), |_, _, cx| {
            shell.update(cx, |this, cx| {
                while !crate::tabs::show_search(this.tabs.tabs.len()) {
                    let ix = this.tabs.tabs.len();
                    this.tabs.open(super::grid_tab(format!("t{ix}"), std::rc::Rc::new(crate::mock::edge_cases()), cx));
                }
                this.overflow_open = true;
                cx.notify();
            });
        });
        // Let the popover mount and take focus, then focus the field as `on_open_change` does.
        let mut vcx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let search = shell.read_with(&vcx, |this, _| this.overflow_search.clone());
        vcx.update(|window, cx| search.update(cx, |s, cx| s.focus(window, cx)));
        vcx.run_until_parked();
        (window.into(), shell)
    }

    #[gpui_kit::test]
    fn typing_a_space_in_the_overflow_search_types_it_and_keeps_the_list_open(cx: &mut gpui_kit::TestAppContext) {
        let (window, shell) = open_overflow(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        cx.run_until_parked();
        cx.simulate_keystrokes("a space b");
        shell.read_with(&cx, |this, cx| {
            assert_eq!(this.overflow_search.read(cx).value().to_string(), "a b", "search text");
            assert!(this.overflow_open, "list stays open");
        });
    }

    #[gpui_kit::test]
    fn overflow_list_closes_when_closing_tabs_drops_the_search_field(cx: &mut gpui_kit::TestAppContext) {
        let (window, shell) = open_overflow(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        shell.update(&mut cx, |this, cx| {
            while crate::tabs::show_search(this.tabs.tabs.len()) {
                this.tabs.close(0);
                this.tab_changed(cx);
            }
            assert!(!this.overflow_open, "open flag cleared once the list has no search field");
        });
    }

    #[gpui_kit::test]
    fn focus_ring_follows_the_focused_pane(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, Focusable as _};
        cx.update(gpui_kit::init);
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let grid = shell.read_with(&cx, |this, _| match this.tabs.active().map(|t| &t.content) {
            Some(super::TabContent::Grid(grid)) => grid.clone(),
            _ => panic!("launch opens on a grid"),
        });
        // Tree, then Tab over to the grid, the console and the panel list; then nothing.
        for (focus, pane) in [("explorer", Some("pane-explorer")), ("grid", Some("pane-content")), ("console", Some("pane-content")), ("panel", Some("pane-panel")), ("nothing", None)] {
            if focus == "console" {
                shell.update(&mut cx, |this, cx| {
                    let console = this.tabs.tabs.iter().position(|t| matches!(t.content, super::TabContent::Console)).expect("launch opens a console");
                    this.tabs.activate(console);
                    this.tab_changed(cx);
                });
                cx.run_until_parked();
            }
            let handle = shell.read_with(&cx, |this, cx| match focus {
                "explorer" => Some(this.explorer.focus_handle(cx)),
                "grid" => Some(grid.focus_handle(cx)),
                "console" => Some(this.editor.focus_handle(cx)),
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
                        assert!(!ring.intersects(&line), "{focus} focused: {splitter} {line:?} covers an edge of the ring {ring:?}");
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
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let mut cx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let ints = ResultSet {
            columns: vec![ColumnMeta { name: "n".into(), data_type: "int".into(), is_pk: false }],
            rows: vec![[Value::Int(3)].into(), [Value::Int(4)].into()],
        };
        let grid = shell.update(&mut cx, |this, cx| {
            this.tabs.open(super::grid_tab("ints", std::rc::Rc::new(ints), cx));
            this.tab_changed(cx);
            match this.tabs.active().map(|t| &t.content) {
                Some(super::TabContent::Grid(grid)) => grid.clone(),
                _ => panic!("opened tab is a grid"),
            }
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("shift-down");
        let shown = Some("2 rows × 1 col · count 2 · sum 7 · avg 3.5".to_string());
        // Grid first, then each other pane and nothing, then back to the grid; the console editor shares the content pane.
        for (focus, expected) in [("grid", &shown), ("explorer", &None), ("console", &None), ("panel", &None), ("nothing", &None), ("grid", &shown)] {
            let handle = shell.read_with(&cx, |this, cx| match focus {
                "explorer" => Some(this.explorer.focus_handle(cx)),
                "grid" => Some(grid.focus_handle(cx)),
                "console" => Some(this.editor.focus_handle(cx)),
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

    /// The Shell with a table tab from prod-pg opened from the tree, the console on prod-pg it
    /// starts with, and a console on local.db.
    fn open_with_connection_tabs(cx: &mut gpui_kit::TestAppContext) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<super::Shell>) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let explorer = shell.read_with(&vcx, |this, _| this.explorer.clone());
        explorer.update_in(&mut vcx, |e, window, cx| e.reveal("0/0/0/table/0", window, cx));
        vcx.run_until_parked();
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        shell.update(&mut vcx, |this, cx| {
            let local = this.workspace.read(cx).connections[2].clone();
            this.tabs.open(super::tabs::EditorTab {
                key: "console-local".into(),
                title: "console-local".into(),
                origin: Some(super::tabs::Origin::of(&local)),
                tooltip: None,
                preview: false,
                pinned: false,
                content: super::TabContent::Console,
            });
            this.tab_changed(cx);
        });
        vcx.run_until_parked();
        let prod_tabs = shell.read_with(&vcx, |this, _| this.tabs.tabs.iter().filter(|t| t.origin.as_ref().is_some_and(|o| o.connection == crate::model::ConnectionId(1))).count());
        assert_eq!(prod_tabs, 2, "a table tab and the console on prod-pg");
        (window.into(), shell)
    }

    fn tab_origins(shell: &gpui_kit::Entity<super::Shell>, cx: &gpui_kit::VisualTestContext) -> Vec<(gpui_kit::SharedString, super::tabs::Origin)> {
        shell.read_with(cx, |this, _| this.tabs.tabs.iter().filter_map(|t| Some((t.key.clone(), t.origin.clone()?))).collect())
    }

    /// The Shell with the mock workspace, parked. Motion is reduced so a dialog lands in place
    /// at once; mid-slide, a click's mouse-up would hit a different spot than its mouse-down.
    fn open_plain(cx: &mut gpui_kit::TestAppContext) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<super::Shell>) {
        use gpui_kit::AppContext as _;
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (window, shell) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| super::Shell::new(window, cx))).expect("open test window")
        });
        gpui_kit::VisualTestContext::from_window(window.into(), cx).run_until_parked();
        (window.into(), shell)
    }

    #[gpui_kit::test]
    fn plus_button_or_add_connection_action_opens_the_dialog(cx: &mut gpui_kit::TestAppContext) {
        for via in ["action", "plus button"] {
            let (window, _shell) = open_plain(cx);
            let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
            assert!(cx.debug_bounds("dialog-layer").is_none(), "{via}: no dialog before");
            if via == "action" {
                cx.dispatch_action(super::AddConnection);
            } else {
                let plus = cx.debug_bounds("add-connection");
                assert!(plus.is_some(), "explorer header has a + button");
                cx.simulate_click(plus.unwrap().center(), Default::default());
            }
            cx.run_until_parked();
            assert!(cx.debug_bounds("dialog-layer").is_some(), "{via}: the dialog layer renders");
        }
    }

    /// Dispatches `AddConnection` and returns the dialog it opened.
    fn open_add_dialog(cx: &mut gpui_kit::VisualTestContext) -> gpui_kit::Entity<crate::connection_dialog::ConnectionDialog> {
        use std::{cell::RefCell, rc::Rc};
        let opened = Rc::new(RefCell::new(None));
        let sink = opened.clone();
        let _watch = cx.update(|_, cx| cx.observe_new(move |_: &mut crate::connection_dialog::ConnectionDialog, _, cx| *sink.borrow_mut() = Some(cx.entity())));
        cx.dispatch_action(super::AddConnection);
        cx.run_until_parked();
        opened.take().expect("AddConnection opened a dialog")
    }

    #[gpui_kit::test]
    fn add_dialog_opens_with_the_spec_defaults(cx: &mut gpui_kit::TestAppContext) {
        use crate::connection_dialog::Form;
        use crate::model::{DbKind, SslMode};
        let (window, _shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let dialog = open_add_dialog(&mut cx);
        // The mock workspace uses colors 0, 2 and 4.
        let want = Form {
            name: "".into(),
            color: 1,
            is_production: false,
            kind: DbKind::Postgres,
            host: "".into(),
            port: "5432".into(),
            user: "".into(),
            database: "postgres".into(),
            ssl: SslMode::Prefer,
            path: "".into(),
        };
        assert_eq!(dialog.read_with(&cx, |d, cx| d.form(cx)), want);
    }

    fn click(cx: &mut gpui_kit::VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} drawn"));
        cx.simulate_click(bounds.center(), Default::default());
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn switching_to_sqlite_shows_only_an_empty_path_field(cx: &mut gpui_kit::TestAppContext) {
        use crate::model::DbKind;
        let (window, _shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let dialog = open_add_dialog(&mut cx);
        let postgres = ["field-host", "field-port", "field-user", "field-database", "field-ssl"];
        // (click, kind, fields shown, fields hidden)
        let cases: [(&str, DbKind, &[&str], &[&str]); 2] = [
            ("kind-sqlite", DbKind::Sqlite, &["field-path"], &postgres),
            ("kind-postgres", DbKind::Postgres, &postgres, &["field-path"]),
        ];
        for (button, kind, shown, hidden) in cases {
            click(&mut cx, button);
            let form = dialog.read_with(&cx, |d, cx| d.form(cx));
            assert_eq!((form.kind, form.path.as_str()), (kind, ""), "{button}");
            for field in shown {
                assert!(cx.debug_bounds(field).is_some(), "{button}: {field} shown");
            }
            for field in hidden {
                assert!(cx.debug_bounds(field).is_none(), "{button}: {field} hidden");
            }
        }
    }

    /// Replaces a dialog field's text by typing into it.
    fn type_into(cx: &mut gpui_kit::VisualTestContext, input: gpui_kit::Entity<gpui_kit::component::input::InputState>, text: &str) {
        cx.update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
        cx.simulate_keystrokes("ctrl-a backspace");
        if !text.is_empty() {
            cx.simulate_input(text);
        }
        cx.run_until_parked();
    }

    /// Types `edits` (field, text) into the open dialog; field "kind-sqlite" clicks that button instead.
    fn fill(cx: &mut gpui_kit::VisualTestContext, dialog: &gpui_kit::Entity<crate::connection_dialog::ConnectionDialog>, edits: &[(&str, &str)]) {
        for (field, text) in edits {
            let input = dialog.read_with(cx, |d, _| match *field {
                "name" => Some(d.name.clone()),
                "host" => Some(d.host.clone()),
                "port" => Some(d.port.clone()),
                "user" => Some(d.user.clone()),
                "database" => Some(d.database.clone()),
                "path" => Some(d.path.clone()),
                _ => None,
            });
            match input {
                Some(input) => type_into(cx, input, text),
                None => click(cx, "kind-sqlite"),
            }
        }
    }

    #[gpui_kit::test]
    fn invalid_fields_show_their_error_and_keep_save_disabled(cx: &mut gpui_kit::TestAppContext) {
        let valid = [("name", "new"), ("host", "h"), ("user", "u")];
        let port = "Port must be a number from 1 to 65535";
        // (edits after `valid`, errors shown, Save enabled)
        let cases: Vec<(Vec<(&str, &str)>, Vec<(&str, &str)>, bool)> = vec![
            (vec![], vec![], true),
            (vec![("name", "prod-pg")], vec![("name", "Another connection is already named prod-pg")], false),
            (vec![("name", " prod-pg ")], vec![("name", "Another connection is already named prod-pg")], false),
            (vec![("name", "Prod-pg")], vec![], true),
            (vec![("name", "   ")], vec![("name", "Name can't be empty")], false),
            (vec![("name", "")], vec![("name", "Name can't be empty")], false),
            (vec![("host", "  ")], vec![("host", "Host can't be empty")], false),
            (vec![("user", "")], vec![("user", "User can't be empty")], false),
            (vec![("database", "")], vec![("database", "Database can't be empty")], false),
            (vec![("port", "0")], vec![("port", port)], false),
            (vec![("port", "65536")], vec![("port", port)], false),
            (vec![("port", "54a")], vec![("port", port)], false),
            (vec![("port", "")], vec![("port", port)], false),
            (vec![("port", " 65535 ")], vec![], true),
            (vec![("name", "prod-pg"), ("port", "x")], vec![("name", "Another connection is already named prod-pg"), ("port", port)], false),
            (vec![("kind-sqlite", ""), ("path", " ")], vec![("path", "Path can't be empty")], false),
            (vec![("kind-sqlite", ""), ("path", "/nope/x.db")], vec![], true),
            // Junk in a Postgres field doesn't block a SQLite save.
            (vec![("port", "x"), ("kind-sqlite", ""), ("path", "/nope/x.db")], vec![], true),
        ];
        let (window, _shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        for (edits, want_errors, want_save) in cases {
            cx.update(|window, cx| gpui_kit::component::WindowExt::close_all_dialogs(window, cx));
            let dialog = open_add_dialog(&mut cx);
            fill(&mut cx, &dialog, &valid);
            fill(&mut cx, &dialog, &edits);
            let errors = dialog.read_with(&cx, |d, cx| d.errors(cx));
            let want: Vec<(&str, String)> = want_errors.iter().map(|(f, m)| (*f, m.to_string())).collect();
            assert_eq!(errors, want, "{edits:?}");
            for (field, _) in &want_errors {
                assert!(cx.debug_bounds(format!("error-{field}").leak()).is_some(), "{edits:?}: {field} error drawn");
            }
            assert_eq!(dialog.read_with(&cx, |d, cx| d.save_enabled(cx)), want_save, "{edits:?}: Save enabled");
        }
    }

    #[gpui_kit::test]
    fn untouched_fields_show_no_error_yet(cx: &mut gpui_kit::TestAppContext) {
        let (window, _shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let dialog = open_add_dialog(&mut cx);
        fill(&mut cx, &dialog, &[("host", "h"), ("host", "")]);
        let errors = dialog.read_with(&cx, |d, cx| d.errors(cx));
        assert_eq!(errors, vec![("host", "Host can't be empty".to_string())], "only the field the user touched");
        assert!(!dialog.read_with(&cx, |d, cx| d.save_enabled(cx)), "Save disabled: name, host and user are empty");
    }

    #[gpui_kit::test]
    fn saving_a_valid_form_appends_the_connection_and_selects_its_row(cx: &mut gpui_kit::TestAppContext) {
        use crate::model::{Connection, ConnectionParams, ConnectionStatus, SslMode};
        use std::path::PathBuf;
        let postgres = ConnectionParams::Postgres { host: "db.local".into(), port: 5433, user: "u".into(), database: "d".into(), ssl: SslMode::Prefer };
        // (empty the workspace first, edits, clicks, want name, color, production, params)
        let cases: Vec<(bool, Vec<(&str, &str)>, Vec<&'static str>, &str, u8, bool, ConnectionParams)> = vec![
            (false, vec![("name", " c "), ("host", " db.local "), ("port", " 5433 "), ("user", " u "), ("database", " d ")], vec![], "c", 1, false, postgres.clone()),
            (false, vec![("name", "c"), ("host", "db.local"), ("port", "5433"), ("user", "u"), ("database", "d")], vec!["swatch-6", "production"], "c", 6, true, postgres.clone()),
            (false, vec![("kind-sqlite", ""), ("name", "notes"), ("path", " /tmp/notes.db ")], vec![], "notes", 1, false, ConnectionParams::Sqlite { path: PathBuf::from("/tmp/notes.db") }),
            (true, vec![("name", "c"), ("host", "db.local"), ("port", "5433"), ("user", "u"), ("database", "d")], vec![], "c", 0, false, postgres.clone()),
        ];
        for (empty, edits, clicks, name, color, production, params) in cases {
            let (window, shell) = open_plain(cx);
            let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
            let (workspace, explorer, panel) = shell.read_with(&cx, |this, _| (this.workspace.clone(), this.explorer.clone(), this.bottom_panel.clone()));
            if empty {
                workspace.update(&mut cx, |ws, cx| {
                    ws.connections.clear();
                    cx.notify();
                });
            }
            let before = workspace.read_with(&cx, |ws, _| ws.connections.clone());
            let dialog = open_add_dialog(&mut cx);
            fill(&mut cx, &dialog, &edits);
            for selector in clicks {
                click(&mut cx, selector);
            }
            click(&mut cx, "dialog-save");

            let after = workspace.read_with(&cx, |ws, _| ws.connections.clone());
            assert_eq!(after.len(), before.len() + 1, "{name}: one connection added");
            assert_eq!(after[..before.len()], before[..], "{name}: the others unchanged, in order");
            let added = after.last().unwrap();
            assert!(!before.iter().any(|c| c.id == added.id), "{name}: fresh id {:?}", added.id);
            let want = Connection { id: added.id, name: name.into(), color, is_production: production, status: ConnectionStatus::Disconnected, params, databases: vec![] };
            assert_eq!(added, &want, "{name}: trimmed values, Disconnected, no databases");
            assert_eq!(explorer.read_with(&cx, |e, _| e.row_labels().last().cloned()), Some(name.into()), "{name}: last tree row");
            assert_eq!(explorer.read_with(&cx, |e, _| e.selected_label()), Some(name.into()), "{name}: new row selected");
            assert_eq!(panel.read_with(&cx, |p, cx| p.connection(added.id, cx).0), name, "{name}: bottom panel knows it");
            assert!(cx.debug_bounds("dialog-layer").is_none(), "{name}: dialog closed");
        }
    }

    #[gpui_kit::test]
    fn enter_saves_a_valid_form_and_esc_or_cancel_close_without_changes(cx: &mut gpui_kit::TestAppContext) {
        let valid = [("host", "db.local"), ("user", "u"), ("name", "c")];
        let invalid = [("host", "db.local"), ("user", "u"), ("name", "prod-pg")];
        // (edits, then press, dialog still open, connections added)
        let cases: [(&[(&str, &str)], &str, bool, usize); 5] = [
            (&valid, "enter", false, 1),
            (&invalid, "enter", true, 0),
            (&valid, "escape", false, 0),
            (&invalid, "escape", false, 0),
            (&valid, "click dialog-cancel", false, 0),
        ];
        for (edits, press, open, added) in cases {
            let (window, shell) = open_plain(cx);
            let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
            let workspace = shell.read_with(&cx, |this, _| this.workspace.clone());
            let before = workspace.read_with(&cx, |ws, _| ws.clone());
            let dialog = open_add_dialog(&mut cx);
            fill(&mut cx, &dialog, edits);
            match press.strip_prefix("click ") {
                Some(selector) => click(&mut cx, selector),
                None => cx.simulate_keystrokes(press),
            }
            cx.run_until_parked();
            let label = format!("{:?} {press}", edits.last().unwrap());
            assert_eq!(cx.debug_bounds("dialog-layer").is_some(), open, "{label}: dialog open");
            let after = workspace.read_with(&cx, |ws, _| ws.clone());
            assert_eq!(after.connections.len(), before.connections.len() + added, "{label}: connections added");
            if added == 0 {
                assert_eq!(after, before, "{label}: workspace unchanged");
            }
        }
    }

    #[gpui_kit::test]
    fn enter_on_an_invalid_form_shows_every_error(cx: &mut gpui_kit::TestAppContext) {
        let (window, _shell) = open_plain(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let dialog = open_add_dialog(&mut cx);
        cx.simulate_keystrokes("enter");
        let fields: Vec<&str> = dialog.read_with(&cx, |d, cx| d.errors(cx)).into_iter().map(|(f, _)| f).collect();
        assert_eq!(fields, ["name", "host", "user"], "untouched fields show why Enter did nothing");
    }

    /// Right-clicks the tree row labeled `name`, picks Edit… from its menu and returns the dialog.
    fn open_edit_dialog(cx: &mut gpui_kit::VisualTestContext, shell: &gpui_kit::Entity<super::Shell>, name: &str) -> gpui_kit::Entity<crate::connection_dialog::ConnectionDialog> {
        use std::{cell::RefCell, rc::Rc};
        let explorer = shell.read_with(cx, |this, _| this.explorer.clone());
        let row = explorer.read_with(cx, |e, _| e.row_labels().iter().position(|l| l == name)).unwrap_or_else(|| panic!("{name} row"));
        let at = cx.debug_bounds(format!("tree-label-{row}").leak()).unwrap_or_else(|| panic!("{name} row drawn")).center();
        let opened = Rc::new(RefCell::new(None));
        let sink = opened.clone();
        let _watch = cx.update(|_, cx| cx.observe_new(move |_: &mut crate::connection_dialog::ConnectionDialog, _, cx| *sink.borrow_mut() = Some(cx.entity())));
        cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
        cx.run_until_parked();
        cx.simulate_keystrokes("down enter");
        cx.run_until_parked();
        opened.take().unwrap_or_else(|| panic!("Edit… on {name} opened a dialog"))
    }

    #[gpui_kit::test]
    fn right_click_edit_opens_the_dialog_prefilled_with_the_type_locked(cx: &mut gpui_kit::TestAppContext) {
        use crate::connection_dialog::Form;
        use crate::model::{DbKind, SslMode};
        let blank = Form { name: "".into(), color: 0, is_production: false, kind: DbKind::Postgres, host: "".into(), port: "5432".into(), user: "".into(), database: "postgres".into(), ssl: SslMode::Prefer, path: "".into() };
        // (row, its form, the other type's button)
        let cases = [
            ("prod-pg", Form { name: "prod-pg".into(), color: 0, is_production: true, host: "db.acme.internal".into(), user: "app".into(), database: "app_db".into(), ..blank.clone() }, "kind-sqlite"),
            ("staging-pg", Form { name: "staging-pg".into(), color: 2, host: "staging.acme.internal".into(), user: "app".into(), database: "app_db".into(), ..blank.clone() }, "kind-sqlite"),
            ("local.db", Form { name: "local.db".into(), color: 4, kind: DbKind::Sqlite, path: "~/acme/local.db".into(), ..blank.clone() }, "kind-postgres"),
        ];
        for (row, want, other_kind) in cases {
            let (window, shell) = open_plain(cx);
            let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
            let dialog = open_edit_dialog(&mut cx, &shell, row);
            assert!(cx.debug_bounds("dialog-layer").is_some(), "{row}: dialog shown");
            assert_eq!(dialog.read_with(&cx, |d, cx| d.form(cx)), want, "{row}: prefilled");
            click(&mut cx, other_kind);
            assert_eq!(dialog.read_with(&cx, |d, cx| d.form(cx).kind), want.kind, "{row}: type locked");
        }
    }

    #[gpui_kit::test]
    fn edit_name_validation_excludes_the_connection_being_edited(cx: &mut gpui_kit::TestAppContext) {
        // (row, name typed or None to leave it, error shown, Save enabled)
        let cases: [(&str, Option<&str>, Option<&str>, bool); 8] = [
            ("prod-pg", None, None, true),
            ("prod-pg", Some("prod-pg"), None, true),
            ("prod-pg", Some(" prod-pg "), None, true),
            ("prod-pg", Some("staging-pg"), Some("Another connection is already named staging-pg"), false),
            ("prod-pg", Some(" local.db "), Some("Another connection is already named local.db"), false),
            ("prod-pg", Some("Staging-pg"), None, true),
            ("prod-pg", Some("  "), Some("Name can't be empty"), false),
            ("local.db", Some("local.db"), None, true),
        ];
        for (row, name, error, save) in cases {
            let (window, shell) = open_plain(cx);
            let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
            let dialog = open_edit_dialog(&mut cx, &shell, row);
            if let Some(name) = name {
                fill(&mut cx, &dialog, &[("name", name)]);
            }
            let errors = dialog.read_with(&cx, |d, cx| d.errors(cx));
            let want: Vec<(&str, String)> = error.map(|e| ("name", e.to_string())).into_iter().collect();
            assert_eq!(errors, want, "{row} {name:?}");
            assert_eq!(dialog.read_with(&cx, |d, cx| d.save_enabled(cx)), save, "{row} {name:?}: Save enabled");
        }
    }

    #[gpui_kit::test]
    fn f2_rename_shows_in_the_tree_the_panel_and_that_connections_tabs_only(cx: &mut gpui_kit::TestAppContext) {
        use crate::model::{ConnectionId, ConnectionStatus};
        let (window, shell) = open_with_connection_tabs(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let before = tab_origins(&shell, &cx);
        let explorer = shell.read_with(&cx, |this, _| this.explorer.clone());
        explorer.update_in(&mut cx, |e, window, cx| e.reveal("0", window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("f2");
        for _ in 0.."prod-pg".len() {
            cx.simulate_keystrokes("backspace");
        }
        cx.simulate_keystrokes("l i v e enter");
        cx.run_until_parked();

        assert_eq!(explorer.read_with(&cx, |e, _| e.row_labels()[0].clone()), "live", "tree row");
        let panel = shell.read_with(&cx, |this, cx| this.bottom_panel.read(cx).connection(ConnectionId(1), cx).0);
        assert_eq!(panel, "live", "bottom panel connection label");
        let conn = shell.read_with(&cx, |this, cx| this.workspace.read(cx).connections[0].clone());
        assert_eq!((conn.name.as_str(), conn.status), ("live", ConnectionStatus::Connected), "shared workspace: renamed, still connected");
        let after = tab_origins(&shell, &cx);
        assert_eq!(after.len(), before.len());
        for ((key, b), (_, a)) in before.iter().zip(&after) {
            let path = match b.path.strip_prefix("prod-pg") {
                Some(tail) if b.connection == ConnectionId(1) => format!("live{tail}"),
                _ => b.path.to_string(),
            };
            assert_eq!(a, &super::tabs::Origin { path: path.into(), ..b.clone() }, "tab {key}");
        }
    }

    #[gpui_kit::test]
    fn recolor_and_production_from_outside_the_explorer_rerender_the_tree_the_panel_and_tabs(cx: &mut gpui_kit::TestAppContext) {
        use crate::model::ConnectionId;
        use std::{cell::Cell, rc::Rc};
        let (window, shell) = open_with_connection_tabs(cx);
        let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
        let (workspace, explorer, panel) = shell.read_with(&cx, |this, _| (this.workspace.clone(), this.explorer.clone(), this.bottom_panel.clone()));
        let renders = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
        cx.update(|_, cx| {
            let hits = renders[0].clone();
            cx.observe(&explorer, move |_, _| hits.set(hits.get() + 1)).detach();
            let hits = renders[1].clone();
            cx.observe(&panel, move |_, _| hits.set(hits.get() + 1)).detach();
        });
        let before = tab_origins(&shell, &cx);
        // (change, new color, new production flag)
        let cases: [(&str, u8, bool); 3] = [("recolor", 5, true), ("production off", 5, false), ("production on", 5, true)];
        for (change, color, production) in cases {
            renders.iter().for_each(|r| r.set(0));
            workspace.update(&mut cx, |ws, cx| {
                let conn = &mut ws.connections[0];
                (conn.color, conn.is_production) = (color, production);
                cx.notify();
            });
            cx.run_until_parked();
            assert!(renders[0].get() > 0, "{change}: explorer re-rendered");
            assert!(renders[1].get() > 0, "{change}: bottom panel re-rendered");
            assert_eq!(panel.read_with(&cx, |p, cx| p.connection(ConnectionId(1), cx)), ("prod-pg".to_string(), color, production), "{change}: panel row");
            for ((key, b), (_, a)) in before.iter().zip(&tab_origins(&shell, &cx)) {
                let expected = if b.connection == ConnectionId(1) { super::tabs::Origin { color, production, ..b.clone() } } else { b.clone() };
                assert_eq!(a, &expected, "{change}: tab {key} stripe and tint");
            }
        }
    }
}
