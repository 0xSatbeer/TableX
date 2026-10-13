//! The shell's window-wide action handlers: layout toggles, tab commands, connections, theme.

use gpui_kit::*;

use crate::{
    AddConnection, CloseAllTabs, CloseOtherTabs, CloseTab, CloseTabsToRight, CopyTabName,
    CycleTheme, DeleteConnection, EditConnection, FocusExplorer, MaximizeEditor, MaximizePanel,
    NextTab, PrevTab, ReopenTab, RestorePanel, RevealTab, ToggleBottomPanel, ToggleExplorer,
    ToggleGallery, TogglePinTab,
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
        cx.notify();
    }))
    .on_action(cx.listener(|this, _: &ToggleGallery, _, cx| {
        this.gallery_visible = !this.gallery_visible;
        cx.notify();
    }))
}
