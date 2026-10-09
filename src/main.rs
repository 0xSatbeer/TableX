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

actions!(queryx, [Quit, ToggleExplorer, ToggleBottomPanel, MaximizeEditor, MaximizePanel, RestorePanel, ToggleGallery, FocusExplorer, CloseTab, ReopenTab, NextTab, PrevTab, CloseOtherTabs, CloseTabsToRight, CloseAllTabs, TogglePinTab, CopyTabName, RevealTab, CycleTheme]);

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
    /// Overflow list: open state and its search field (shown above `tabs::SEARCH_MIN_TABS` tabs).
    overflow_open: bool,
    overflow_search: Entity<InputState>,
    _appearance: Subscription,
}

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = &mock::workspaces()[0];
        let explorer = cx.new(|cx| explorer::Explorer::new(workspace.clone(), window, cx));
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
                panel_maximized: false,
                explorer: split::EXPLORER,
                bottom: split::BOTTOM,
                dragging: None,
            },
            status_bar: cx.new(|cx| statusbar::StatusBar::new(window, cx)),
            bottom_panel: cx.new(|cx| panel::BottomPanel::new(workspace, window, cx)),
            focus_handle: cx.focus_handle(),
            gallery_visible: false,
            focus_content: false,
            theme: theme::ThemeChoice::default(),
            user_themes: Vec::new(),
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

    /// ⚙ in the titlebar. Holds the theme until there is a settings view (QX-020);
    /// `Ctrl+K Ctrl+T` still cycles it from the keyboard.
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
                h_flex().flex_none().h_full().px(theme::SPACE_4).border_l_1().border_b_1().border_color(theme.border).child(
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
                    div().p(theme::SPACE_8).text_color(cx.theme().muted_foreground).child(label).into_any_element()
                } else {
                    v_flex()
                        .id("overflow-list")
                        .max_h(theme::SPACE_24 * 10.)
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
                v_flex().key_context(OVERFLOW_CONTEXT).w(theme::SPACE_24 * 10.).gap(theme::SPACE_4).child(Input::new(&search).small()).child(list)
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

/// Drag payload of a tab; also its drag ghost. Dropping on tab `j` means "before `j`".
#[derive(Clone)]
struct TabDrag {
    ix: usize,
    title: SharedString,
}

impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div().px(theme::SPACE_12).py(theme::SPACE_6).bg(theme.tab_active).border_1().border_color(theme.border).text_size(theme::UI_SM).child(self.title.clone())
    }
}

impl Shell {
    /// Our own handle over the panel's edge, drawn after the panels so its band sits on top.
    /// `size` is the panel's drawn size; the 1px line covers the panel's own edge pixel.
    fn splitter(&self, edge: Edge, size: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let dragging = self.layout.dragging.as_ref().is_some_and(|d| d.edge == edge);
        let offset = size - px(1.) - SPLITTER_PAD;
        let line = div().flex_none().bg(if dragging { theme.primary } else { theme.border });
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
            .relative()
            .size_full()
            // Only while maximized: a tracked root focuses itself on every click on plain chrome.
            .when(self.layout.panel_maximized, |d| d.track_focus(&self.focus_handle).key_context(PANEL_MAXIMIZED_CONTEXT))
            .bg(theme.background)
            .text_size(theme::UI)
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
                            d.child(div().flex_none().h_full().w(explorer_w).overflow_hidden().child(self.explorer.clone()))
                        })
                        .child(
                            v_flex()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .when(!maximized, |d| d.child(div().flex_1().min_h_0().overflow_hidden().child(v_flex().size_full().child(tabs).child(content))))
                                .when(self.layout.bottom_visible, |d| {
                                    let panel = div().w_full().overflow_hidden().child(self.bottom_panel.clone());
                                    if maximized {
                                        d.child(panel.flex_1().min_h_0())
                                    } else {
                                        d.child(panel.flex_none().h(bottom_h)).child(self.splitter(Edge::Bottom, bottom_h, cx))
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

fn bind_panel_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", RestorePanel, Some(PANEL_MAXIMIZED_CONTEXT))]);
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
}
