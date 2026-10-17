//! The titlebar's workspace switcher and moving connections between workspaces.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::*;

use crate::model::{ConnectionId, ConnectionStatus, Workspace};
use crate::persist::Keys;
use crate::ui::password_prompt::keychain_failed;
use crate::ui::workspace_dialog::{NameDialog, Named};

use super::{Shell, TabContent};

impl Shell {
    /// `◉ <workspace> ▾`: every workspace (the active one checked), then New, Rename, Delete.
    pub(super) fn workspace_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let active = self.workspace.read(cx);
        let (current, name) = (active.id, active.name.clone());
        let list: Vec<(u32, String)> = self.workspace_list(cx);
        let deletable = list.len() > 1;
        Button::new("workspace")
            .ghost()
            .small()
            .label(format!("◉ {name} ▾"))
            .tooltip("Switch workspace")
            .dropdown_menu(move |mut menu, _, _| {
                for (id, name) in list.iter().cloned() {
                    let shell = shell.clone();
                    menu = menu.item(PopupMenuItem::new(name).checked(id == current).on_click(
                        move |_, window, cx| {
                            _ = shell.update(cx, |this, cx| this.switch_workspace(id, window, cx));
                        },
                    ));
                }
                let (new, rename, delete) = (shell.clone(), shell.clone(), shell.clone());
                menu.separator()
                    .item(
                        PopupMenuItem::new("New workspace…").on_click(move |_, window, cx| {
                            _ = new.update(cx, |this, cx| this.ask_name(None, window, cx));
                        }),
                    )
                    .item(
                        PopupMenuItem::new("Rename workspace…").on_click(move |_, window, cx| {
                            _ = rename
                                .update(cx, |this, cx| this.ask_name(Some(current), window, cx));
                        }),
                    )
                    .item(
                        PopupMenuItem::new("Delete workspace…")
                            .disabled(!deletable)
                            .on_click(move |_, window, cx| {
                                _ = delete.update(cx, |this, cx| this.confirm_delete(window, cx));
                            }),
                    )
            })
    }

    /// Every workspace by id and name, in display order; demo mode has only the shown one.
    pub(super) fn workspace_list(&self, cx: &App) -> Vec<(u32, String)> {
        let active = self.workspace.read(cx);
        match &self.workspaces_file {
            Some(file) => file
                .all
                .list
                .iter()
                .map(|w| {
                    (
                        w.id,
                        if w.id == active.id {
                            &active.name
                        } else {
                            &w.name
                        }
                        .clone(),
                    )
                })
                .collect(),
            None => vec![(active.id, active.name.clone())],
        }
    }

    /// Opens New workspace (`renaming` is `None`) or Rename workspace.
    fn ask_name(&mut self, renaming: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        let (title, text) = match renaming {
            Some(_) => ("Rename workspace", self.workspace.read(cx).name.clone()),
            None => ("New workspace", String::new()),
        };
        let dialog = NameDialog::open(title, &text, window, cx);
        cx.subscribe_in(
            &dialog,
            window,
            move |this, _, Named(name), window, cx| match renaming {
                Some(_) => this.rename_workspace(name.clone(), window, cx),
                None => this.new_workspace(name.clone(), window, cx),
            },
        )
        .detach();
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = self.workspace.read(cx);
        let n = ws.connections.len();
        let title = format!("Delete workspace {} and its {n} connections?", ws.name);
        let shell = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let shell = shell.clone();
            dialog
                .title(title.clone())
                .confirm()
                .ok_text("Delete workspace")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    _ = shell.update(cx, |this, cx| this.delete_workspace(window, cx));
                    true
                })
        });
    }

    /// Adds an empty workspace named `name` after the others and switches to it.
    pub(super) fn new_workspace(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = self.workspaces_file.as_mut() else {
            return;
        };
        let id = file.all.list.iter().map(|w| w.id).max().unwrap_or(0) + 1;
        file.all.list.push(Workspace {
            id,
            name,
            connections: Vec::new(),
            id_floor: 0,
        });
        self.switch_workspace(id, window, cx);
    }

    pub(super) fn rename_workspace(
        &mut self,
        name: String,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |ws, cx| {
            ws.name = name;
            cx.notify();
        });
    }

    /// Closes the shown workspace's tabs and shows workspace `id` instead, disconnected.
    pub(super) fn switch_workspace(
        &mut self,
        id: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if id == self.workspace.read(cx).id {
            return;
        }
        if !self
            .workspaces_file
            .as_ref()
            .is_some_and(|f| f.all.list.iter().any(|w| w.id == id))
        {
            return;
        }
        self.save_session(cx);
        let Some(file) = self.workspaces_file.as_mut() else {
            return;
        };
        let Some(mut next) = file.all.list.iter().find(|w| w.id == id).cloned() else {
            return;
        };
        file.all.active = id;
        // The ids every workspace uses, so `add` in this one stays unique.
        file.all.share_ids();
        next.id_floor = file.all.list[0].id_floor;
        for conn in &mut next.connections {
            conn.status = ConnectionStatus::Disconnected;
            conn.databases = None;
        }
        let gone: Vec<ConnectionId> = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .map(|c| c.id)
            .collect();
        gone.into_iter()
            .for_each(|id| self.close_connection_tabs(id, cx));
        self.workspace.update(cx, |ws, cx| {
            *ws = next;
            cx.notify();
        });
        self.update_move_targets(cx);
        self.restore_session(window, cx);
    }

    /// Deletes the shown workspace, its connections and their saved passwords, then shows the
    /// first one left. The last workspace stays: returns false.
    pub(super) fn delete_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let ws = self.workspace.read(cx);
        let (id, ids): (u32, Vec<ConnectionId>) =
            (ws.id, ws.connections.iter().map(|c| c.id).collect());
        let Some(file) = self
            .workspaces_file
            .as_mut()
            .filter(|f| f.all.list.len() > 1)
        else {
            return false;
        };
        file.all.list.retain(|w| w.id != id);
        let next = file.all.list[0].id;
        for conn in ids {
            if let Err(error) = Keys::of(cx).delete(conn) {
                keychain_failed(&error, cx);
            }
        }
        self.switch_workspace(next, window, cx);
        if let Some(root) = &self.sessions_root {
            _ = std::fs::remove_file(crate::persist::session_path(root, id));
        }
        true
    }

    /// Moves connection `id` to workspace `to`, keeping its id and saved password; its tabs close.
    pub(super) fn move_connection(
        &mut self,
        id: ConnectionId,
        to: u32,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut conn) = self
            .workspace
            .read(cx)
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
        else {
            return;
        };
        let Some(target) = self
            .workspaces_file
            .as_mut()
            .and_then(|f| f.all.list.iter_mut().find(|w| w.id == to))
        else {
            return;
        };
        conn.status = ConnectionStatus::Disconnected;
        conn.databases = None;
        target.connections.push(conn);
        self.close_connection_tabs(id, cx);
        self.workspace.update(cx, |ws, cx| {
            ws.delete(id);
            // The moved id stays taken, so `add` never hands it out again here.
            ws.id_floor = ws.id_floor.max(id.0);
            cx.notify();
        });
    }

    /// Closes connection `id`'s tabs, giving back their consoles' sessions.
    pub(super) fn close_connection_tabs(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        for tab in self.tabs.drop_connection(id) {
            if let TabContent::Console(console) = tab.content {
                console.update(cx, |console, cx| console.close_session(cx));
            }
        }
        self.tab_changed(cx);
    }

    /// Gives the explorer's Move to submenu the workspaces other than the shown one.
    pub(super) fn update_move_targets(&mut self, cx: &mut Context<Self>) {
        let current = self.workspace.read(cx).id;
        let targets = self
            .workspace_list(cx)
            .into_iter()
            .filter(|(id, _)| *id != current)
            .map(|(id, name)| (id, name.into()))
            .collect();
        self.explorer
            .update(cx, |e, cx| e.set_move_targets(targets, cx));
    }
}
