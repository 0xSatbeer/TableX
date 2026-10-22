//! The shell's window-wide action handlers: layout toggles, tab commands, connections, theme.

use gpui_kit::component::IndexPath;
use gpui_kit::component::list::{ListEvent, ListState};
use gpui_kit::*;

use crate::ui::{cheat_sheet, palette, quick_open};

use crate::{
    AddConnection, CheatSheet, CloseAllTabs, CloseOtherTabs, CloseTab, CloseTabsToRight,
    CommandPalette, CopyTabName, CycleTheme, DeleteConnection, EditConnection, FocusExplorer,
    MaximizeEditor, MaximizePanel, NextTab, PrevTab, QuickOpen, ReopenTab, RestorePanel, RevealTab,
    ToggleBottomPanel, ToggleExplorer, ToggleGallery, TogglePinTab,
};

use super::Shell;

/// Registers every shell action on the window's root element.
pub(super) fn on_actions(root: Div, cx: &Context<Shell>) -> Div {
    root.on_action(cx.listener(|this, _: &ToggleExplorer, _, cx| {
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
    .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
        this.close_tabs(vec![this.tabs.active], window, cx);
    }))
    .on_action(cx.listener(|this, _: &CloseOtherTabs, window, cx| {
        let active = this.tabs.active;
        this.close_tabs(this.tabs.closable(|ix| ix != active), window, cx);
    }))
    .on_action(cx.listener(|this, _: &CloseTabsToRight, window, cx| {
        let active = this.tabs.active;
        this.close_tabs(this.tabs.closable(|ix| ix > active), window, cx);
    }))
    .on_action(cx.listener(|this, _: &CloseAllTabs, window, cx| {
        this.close_tabs(this.tabs.closable(|_| true), window, cx);
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
        let Some(key) = this
            .tabs
            .active()
            .filter(|t| t.tooltip.is_some())
            .map(|t| t.key.clone())
        else {
            return;
        };
        this.layout.show_explorer();
        this.explorer
            .update(cx, |explorer, cx| explorer.reveal(&key, window, cx));
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
        if let Some(n) = ev
            .keystroke
            .key
            .parse::<usize>()
            .ok()
            .filter(|n| m.control && !m.shift && (1..=9).contains(n))
        {
            this.tabs.activate(n - 1);
            this.tab_changed(cx);
        }
    }))
    .on_action(cx.listener(|this, _: &AddConnection, window, cx| {
        this.explorer
            .update(cx, |explorer, cx| explorer.add_connection(window, cx));
    }))
    .on_action(cx.listener(|this, _: &EditConnection, window, cx| {
        this.explorer
            .update(cx, |explorer, cx| explorer.edit_connection(window, cx));
    }))
    .on_action(cx.listener(|this, _: &DeleteConnection, window, cx| {
        this.explorer
            .update(cx, |explorer, cx| explorer.delete_connection(window, cx));
    }))
    .on_action(cx.listener(|this, _: &FocusExplorer, window, cx| {
        this.layout.show_explorer();
        this.explorer
            .update(cx, |explorer, cx| explorer.focus_field(window, cx));
        cx.notify();
    }))
    .on_action(cx.listener(|this, _: &CycleTheme, window, cx| {
        this.theme = this.theme.next();
        this.theme.apply(&this.user_themes, window, cx);
        this.save_config(window, cx);
        cx.notify();
    }))
    .on_action(cx.listener(|this, _: &ToggleGallery, _, cx| {
        this.gallery_visible = !this.gallery_visible;
        cx.notify();
    }))
    .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
        this.open_palette(window, cx);
    }))
    .on_action(cx.listener(|this, _: &CheatSheet, window, cx| {
        this.open_cheat_sheet(window, cx);
    }))
    .on_action(cx.listener(|this, _: &QuickOpen, window, cx| {
        this.open_quick_open(window, cx);
    }))
}

impl Shell {
    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            return;
        }
        let previous = window.focused(cx);
        let list =
            cx.new(|cx| ListState::new(palette::Commands::new(cx), window, cx).searchable(true));
        let events = cx.subscribe_in(
            &list,
            window,
            |this, list, event: &ListEvent, window, cx| {
                let action = match event {
                    ListEvent::Confirm(ix) => list.read(cx).delegate().action(ix.row),
                    ListEvent::Cancel => None,
                    _ => return,
                };
                this.close_palette(action, window, cx);
            },
        );
        list.update(cx, |list, cx| {
            list.set_selected_index(Some(IndexPath::default()), window, cx);
            list.focus(window, cx);
        });
        self.palette = Some(palette::Open {
            list,
            previous,
            _events: events,
        });
        cx.notify();
    }

    /// Closes the palette, refocuses what it covered and runs `action` there.
    pub(super) fn close_palette(
        &mut self,
        action: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.palette.take() else {
            return;
        };
        if let Some(previous) = open.previous {
            window.focus(&previous, cx);
        }
        cx.notify();
        let Some(action) = action else {
            return;
        };
        palette::remember(action, cx);
        if let Ok(action) = cx.build_action(action, None) {
            window.dispatch_action(action, cx);
        }
    }

    fn open_cheat_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cheat_sheet.is_some() {
            return;
        }
        let previous = window.focused(cx);
        let list = cx
            .new(|cx| ListState::new(cheat_sheet::Sheet::current(cx), window, cx).searchable(true));
        let events = cx.subscribe_in(&list, window, |this, _, event: &ListEvent, window, cx| {
            if let ListEvent::Cancel = event {
                this.close_cheat_sheet(window, cx);
            }
        });
        list.update(cx, |list, cx| list.focus(window, cx));
        self.cheat_sheet = Some(cheat_sheet::Open {
            list,
            previous,
            _events: events,
        });
        cx.notify();
    }

    /// Closes the cheat sheet and refocuses what it covered.
    pub(super) fn close_cheat_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.cheat_sheet.take() else {
            return;
        };
        if let Some(previous) = open.previous {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    fn open_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_open.is_some() {
            return;
        }
        let previous = window.focused(cx);
        let explorer = self.explorer.clone();
        explorer.update(cx, |e, cx| e.load_all_objects(cx));
        let objects =
            quick_open::Objects::new(self.workspace.read(cx), explorer.read(cx).schemas_pending());
        let list = cx.new(|cx| ListState::new(objects, window, cx).searchable(true));
        let events = cx.subscribe_in(
            &list,
            window,
            |this, list, event: &ListEvent, window, cx| {
                let key = match event {
                    ListEvent::Confirm(ix) => list.read(cx).delegate().key(ix.row),
                    ListEvent::Cancel => None,
                    _ => return,
                };
                this.close_quick_open(key, window, cx);
            },
        );
        // The explorer notifies whenever a load lands, after storing it in the workspace.
        let loads = cx.observe(&explorer, |this, explorer, cx| {
            let Some(open) = &this.quick_open else {
                return;
            };
            let pending = explorer.read(cx).schemas_pending();
            let workspace = this.workspace.clone();
            open.list.update(cx, |list, cx| {
                list.delegate_mut().update(workspace.read(cx), pending);
                cx.notify();
            });
        });
        list.update(cx, |list, cx| {
            list.set_selected_index(Some(IndexPath::default()), window, cx);
            list.focus(window, cx);
        });
        self.quick_open = Some(quick_open::Open {
            list,
            previous,
            _events: events,
            _loads: loads,
        });
        cx.notify();
    }

    /// Closes quick open; opens and reveals object `key`, else refocuses what it covered.
    pub(super) fn close_quick_open(
        &mut self,
        key: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.quick_open.take() else {
            return;
        };
        self.explorer.update(cx, |e, _| e.stop_loading_all());
        cx.notify();
        match key {
            Some(key) => self
                .explorer
                .update(cx, |e, cx| e.open_key(&key, window, cx)),
            None => {
                if let Some(previous) = open.previous {
                    window.focus(&previous, cx);
                }
            }
        }
    }
}
