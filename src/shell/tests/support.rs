use super::*;

/// Dispatches `AddConnection` and returns the dialog it opened.
pub(super) fn open_add_dialog(
    cx: &mut gpui_kit::VisualTestContext,
) -> gpui_kit::Entity<crate::ui::connection_dialog::ConnectionDialog> {
    use std::{cell::RefCell, rc::Rc};
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(
            move |_: &mut crate::ui::connection_dialog::ConnectionDialog, _, cx| {
                *sink.borrow_mut() = Some(cx.entity())
            },
        )
    });
    cx.dispatch_action(crate::AddConnection);
    cx.run_until_parked();
    opened.take().expect("AddConnection opened a dialog")
}

pub(super) fn click(cx: &mut gpui_kit::VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} drawn"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

/// Replaces a dialog field's text by typing into it.
pub(super) fn type_into(
    cx: &mut gpui_kit::VisualTestContext,
    input: gpui_kit::Entity<gpui_kit::component::input::InputState>,
    text: &str,
) {
    cx.update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
    cx.simulate_keystrokes("ctrl-a backspace");
    if !text.is_empty() {
        cx.simulate_input(text);
    }
    cx.run_until_parked();
}

/// Types `edits` (field, text) into the open dialog; field "kind-sqlite" clicks that button instead.
pub(super) fn fill(
    cx: &mut gpui_kit::VisualTestContext,
    dialog: &gpui_kit::Entity<crate::ui::connection_dialog::ConnectionDialog>,
    edits: &[(&str, &str)],
) {
    for (field, text) in edits {
        let input = dialog.read_with(cx, |d, _| match *field {
            "name" => Some(d.name.clone()),
            "host" => Some(d.host.clone()),
            "port" => Some(d.port.clone()),
            "user" => Some(d.user.clone()),
            "password" => Some(d.password.clone()),
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

/// Right-clicks the tree row labeled `name`, picks Edit… from its menu and returns the dialog.
pub(super) fn open_edit_dialog(
    cx: &mut gpui_kit::VisualTestContext,
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    name: &str,
) -> gpui_kit::Entity<crate::ui::connection_dialog::ConnectionDialog> {
    use std::{cell::RefCell, rc::Rc};
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    let _watch = cx.update(|_, cx| {
        cx.observe_new(
            move |_: &mut crate::ui::connection_dialog::ConnectionDialog, _, cx| {
                *sink.borrow_mut() = Some(cx.entity())
            },
        )
    });
    right_click_row(cx, shell, name);
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    opened
        .take()
        .unwrap_or_else(|| panic!("Edit… on {name} opened a dialog"))
}

/// Right-clicks the tree row labeled `name`, revealing it first if it is scrolled out of view.
pub(super) fn right_click_row(
    cx: &mut gpui_kit::VisualTestContext,
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    name: &str,
) {
    let (explorer, workspace) = shell.read_with(cx, |this, _| {
        (this.explorer.clone(), this.workspace.clone())
    });
    let row = |cx: &mut gpui_kit::VisualTestContext| {
        explorer
            .read_with(cx, |e, _| e.row_labels().iter().position(|l| l == name))
            .unwrap_or_else(|| panic!("{name} row"))
    };
    let selector = |row: usize| -> &'static str { format!("tree-label-{row}").leak() };
    let first = selector(row(cx));
    if cx.debug_bounds(first).is_none() {
        // Scrolled out of view: reveal it.
        let id = workspace.read_with(cx, |ws, _| {
            ws.connections.iter().find(|c| c.name == name).unwrap().id
        });
        explorer.update_in(cx, |e, window, cx| e.reveal(&id.0.to_string(), window, cx));
        cx.run_until_parked();
    }
    let shown = selector(row(cx));
    let at = cx
        .debug_bounds(shown)
        .unwrap_or_else(|| panic!("{name} row drawn"))
        .center();
    cx.simulate_mouse_down(at, gpui_kit::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(at, gpui_kit::MouseButton::Right, Default::default());
    cx.run_until_parked();
}

/// Types `edits` into the open dialog, then presses a key or clicks `click <selector>`.
pub(super) fn finish(
    cx: &mut gpui_kit::VisualTestContext,
    dialog: &gpui_kit::Entity<crate::ui::connection_dialog::ConnectionDialog>,
    edits: &[(&str, &str)],
    press: &'static str,
) {
    fill(cx, dialog, edits);
    match press.strip_prefix("click ") {
        Some(selector) => click(cx, selector),
        None => cx.simulate_keystrokes(press),
    }
    cx.run_until_parked();
}

/// A fake driver and an app whose explorer and dialogs read through it.
pub(super) fn open_with_fake(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    std::sync::Arc<crate::db::fake::Fake>,
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<crate::shell::Shell>,
) {
    let fake = std::sync::Arc::new(crate::db::fake::Fake::default());
    let (window, shell) = open_plain_with(cx, fake.clone());
    (fake, window, shell)
}

/// Answers every pending Test call in order: the last with `last`, the earlier ones with Ok.
pub(super) fn answer_tests(fake: &crate::db::fake::Fake, last: Result<(), String>) {
    let labels = fake.pending();
    assert!(
        !labels.is_empty() && labels.iter().all(|l| l.starts_with("test ")),
        "pending: {labels:?}"
    );
    for (ix, label) in labels.iter().enumerate() {
        fake.resolve(
            label,
            if ix + 1 == labels.len() {
                last.clone()
            } else {
                Ok(())
            },
        );
    }
}

/// A console tab on `conn`, pinned or not.
pub(super) fn console_on(
    conn: &crate::model::Connection,
    key: &str,
    pinned: bool,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<crate::shell::Shell>,
) -> crate::shell::tabs::EditorTab<crate::shell::TabContent> {
    let key = key.to_string();
    crate::shell::tabs::EditorTab {
        pinned,
        ..crate::shell::start::console_tab(
            key.clone(),
            key,
            Some(crate::shell::tabs::Origin::of(conn)),
            crate::ui::console::Target {
                workspace: gpui_kit::AppContext::new(cx, |_| crate::mock::workspaces().remove(0)),
                driver: std::sync::Arc::new(crate::db::fake::Fake::default()),
                connection: Some(conn.id),
                database: conn.params.database(),
            },
            "",
            window,
            cx,
        )
    }
}

/// Right-clicks the row labeled `name` and picks Delete from its menu.
pub(super) fn open_delete(
    cx: &mut gpui_kit::VisualTestContext,
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    name: &str,
) {
    right_click_row(cx, shell, name);
    click(cx, "menu-delete");
}

/// Deletes the row labeled `name` through its menu, typing the name for Production.
pub(super) fn delete_row(
    cx: &mut gpui_kit::VisualTestContext,
    shell: &gpui_kit::Entity<crate::shell::Shell>,
    name: &str,
) {
    open_delete(cx, shell, name);
    if cx.debug_bounds("delete-name").is_some() {
        cx.simulate_input(name);
    }
    click(cx, "delete-confirm");
}
