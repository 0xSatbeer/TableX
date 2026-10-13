use gpui_kit::component::input::{EditorState, InputEvent, InputState};
use gpui_kit::*;

use crate::ui::{explorer, grid, panel, split, statusbar, tabs};
use crate::{db, mock, model, theme};
use std::rc::Rc;

use super::{Mode, Shell, TabContent, layout::LayoutState};

impl Shell {
    pub(super) fn new(mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (workspace, driver): (_, std::sync::Arc<dyn db::Driver>) = match mode {
            Mode::Normal => (
                model::Workspace {
                    id: 1,
                    name: "Workspace".into(),
                    connections: Vec::new(),
                },
                std::sync::Arc::new(db::Live::default()),
            ),
            Mode::Demo => {
                // The trees start unloaded, so the mock driver serves each level as it is expanded.
                let mut workspace = mock::workspaces().remove(0);
                workspace
                    .connections
                    .iter_mut()
                    .for_each(|c| c.databases = None);
                (
                    workspace,
                    std::sync::Arc::new(db::mock::Mock::new(cx.background_executor().clone())),
                )
            }
        };
        Self::with_driver(mode, workspace, driver, window, cx)
    }

    /// The Shell started in `mode` on `workspace`, whose explorer reads databases through `driver`.
    pub(super) fn with_driver(
        mode: Mode,
        workspace: model::Workspace,
        driver: std::sync::Arc<dyn db::Driver>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let demo = mode == Mode::Demo;
        let workspace = cx.new(|_| workspace);
        // The demo console runs against production, so the tint shows up.
        let console_origin = workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| demo && c.is_production)
            .map(tabs::Origin::of);
        let explorer = cx.new(|cx| explorer::Explorer::new(workspace.clone(), driver, window, cx));
        cx.observe(&workspace, |this, workspace, cx| {
            this.tabs.refresh_origins(workspace.read(cx));
            let (origin, has_tab) = (
                this.tabs.active().and_then(|t| t.origin.clone()),
                this.tabs.active().is_some(),
            );
            this.status_bar
                .update(cx, |bar, cx| bar.set_origin(origin, has_tab, cx));
            cx.notify();
        })
        .detach();
        cx.subscribe(
            &explorer,
            |this, _, ev: &explorer::ConnectionDeleted, cx| {
                this.tabs.drop_connection(ev.id);
                this.tab_changed(cx);
            },
        )
        .detach();
        cx.subscribe(&explorer, move |this, _, ev: &explorer::OpenObject, cx| {
            // Table data needs query execution (QX-033); until then only the demo shows rows.
            let rows = match demo.then(|| mock::run_query(&format!("SELECT * FROM {}", ev.name))) {
                None => model::ResultSet {
                    columns: Vec::new(),
                    rows: Vec::new(),
                },
                Some(model::QueryOutcome::Rows(rs)) => rs,
                Some(_) => mock::edge_cases(),
            };
            let mut tab = grid_tab(ev.key.clone(), Rc::new(rows), cx);
            (tab.title, tab.origin, tab.preview) =
                (ev.name.clone(), Some(ev.origin.clone()), ev.preview);
            tab.tooltip = Some(ev.path.clone());
            this.tabs.open(tab);
            this.tab_changed(cx);
            // A preview (single click, Space) leaves focus in the tree for the next arrow key.
            this.focus_content = !ev.preview;
        })
        .detach();
        cx.subscribe_in(
            &explorer,
            window,
            |this, _, ev: &explorer::OpenConsole, window, cx| {
                let key: SharedString = match &ev.schema {
                    Some(schema) => format!("console:{}/{}/{schema}", ev.conn.0, ev.database),
                    None => format!("console:{}/{}", ev.conn.0, ev.database),
                }
                .into();
                if let Some(ix) = this.tabs.tabs.iter().position(|t| t.key == key) {
                    this.tabs.activate(ix);
                } else {
                    let Some(conn) = this
                        .workspace
                        .read(cx)
                        .connections
                        .iter()
                        .find(|c| c.id == ev.conn)
                    else {
                        return;
                    };
                    let tail = tabs::join_path(&ev.database, ev.schema.as_deref().unwrap_or(""));
                    let origin = tabs::Origin {
                        connection: conn.id,
                        color: conn.color,
                        production: conn.is_production,
                        path: tabs::join_path(&conn.name, &tail),
                        tail,
                    };
                    let title = format!("{} — console", ev.database);
                    this.tabs
                        .open(console_tab(key, title, Some(origin), "", window, cx));
                }
                this.tab_changed(cx);
            },
        )
        .detach();
        let status_bar =
            cx.new(|cx| statusbar::StatusBar::new(workspace.clone(), demo, window, cx));
        cx.subscribe(&status_bar, |this, _, statusbar::Connect(id), cx| {
            this.explorer
                .update(cx, |e, cx| e.connect_connection(*id, cx))
        })
        .detach();
        let overflow_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search tabs…"));
        cx.subscribe(
            &overflow_search,
            |this, search, ev: &InputEvent, cx| match ev {
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
            },
        )
        .detach();
        let console = console_tab(
            "console-1",
            "console-1",
            console_origin,
            if demo {
                "SELECT id, email\nFROM users\nWHERE created_at > now() - interval '7 days';\n"
            } else {
                ""
            },
            window,
            cx,
        );
        let tabs = match mode {
            Mode::Normal => vec![console],
            Mode::Demo => vec![
                grid_tab("big_events", Rc::new(grid::BigEvents::new()), cx),
                grid_tab("wide_table", Rc::new(mock::wide_table(2_000)), cx),
                grid_tab("edge_cases", Rc::new(mock::edge_cases()), cx),
                console,
            ],
        };
        let log = if demo {
            mock::mock_query_log(panel::LOG_CAP)
        } else {
            Vec::new()
        };
        let mut this = Self {
            overflow_open: false,
            overflow_search,
            workspace: workspace.clone(),
            explorer,
            tab_scroll: ScrollHandle::new(),
            tabs: tabs::Tabs::new(tabs),
            layout: LayoutState {
                explorer_visible: true,
                bottom_visible: true,
                maximized_from: None,
                panel_maximized: false,
                explorer: split::EXPLORER,
                bottom: split::BOTTOM,
                dragging: None,
            },
            status_bar,
            bottom_panel: cx.new(|cx| panel::BottomPanel::new(workspace.clone(), log, window, cx)),
            focus_handle: cx.focus_handle(),
            gallery_visible: false,
            focus_content: false,
            theme: theme::ThemeChoice::default(),
            user_themes: Vec::new(),
            user_theme_errors: Vec::new(),
            console_mono: theme::MONO,
            _appearance: cx.observe_window_appearance(window, |this, window, cx| {
                this.theme.apply(&this.user_themes, window, cx)
            }),
        };
        this.tab_changed(cx);
        this
    }
}

pub(super) fn grid_tab(
    key: impl Into<SharedString>,
    source: Rc<dyn grid::GridSource>,
    cx: &mut Context<Shell>,
) -> tabs::EditorTab<TabContent> {
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

/// A console tab keyed `key` with its own SQL editor holding `text`.
pub(super) fn console_tab(
    key: impl Into<SharedString>,
    title: impl Into<SharedString>,
    origin: Option<tabs::Origin>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> tabs::EditorTab<TabContent> {
    let text = text.to_string();
    tabs::EditorTab {
        key: key.into(),
        title: title.into(),
        origin,
        tooltip: None,
        preview: false,
        pinned: false,
        content: TabContent::Console(cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .default_value(text)
        })),
    }
}
