//! The toolbar above the tree acts on the selected row.

use super::*;

fn select_label(
    explorer: &gpui_kit::Entity<super::Explorer>,
    cx: &mut gpui_kit::VisualTestContext,
    label: &str,
) {
    let ix = labels(explorer, cx)
        .iter()
        .position(|l| l == label)
        .unwrap();
    let at = cx
        .debug_bounds(format!("tree-label-{ix}").leak())
        .unwrap()
        .center();
    cx.simulate_click(at, Default::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn toolbar_buttons_act_on_the_selected_row(cx: &mut gpui_kit::TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    let (mut cx, explorer, workspace, fake) = tables_open(cx);
    let consoles = Rc::new(RefCell::new(0));
    let opened = Rc::new(RefCell::new(0));
    let (c, o) = (consoles.clone(), opened.clone());
    let _subs = cx.update(|_, cx| {
        (
            cx.subscribe(&explorer, move |_, _: &super::super::OpenConsole, _| {
                *c.borrow_mut() += 1
            }),
            cx.subscribe(&explorer, move |_, _: &OpenObject, _| *o.borrow_mut() += 1),
        )
    });

    select_label(&explorer, &mut cx, "public");
    click(&mut cx, "tb-refresh");
    assert_eq!(fake.pending(), ["objects app/public"], "Refresh");
    answer(&fake, "objects app/public", Ok(()));
    cx.run_until_parked();

    click(&mut cx, "tb-console");
    assert_eq!(*consoles.borrow(), 1, "Open console");

    select_label(&explorer, &mut cx, "t");
    let before = *opened.borrow();
    click(&mut cx, "tb-open");
    assert!(*opened.borrow() > before, "Open");

    click(&mut cx, "tb-disconnect");
    assert_eq!(
        workspace.read_with(&cx, |ws, _| ws.connections[0].status.clone()),
        ConnectionStatus::Disconnected,
        "Disconnect"
    );
}

#[gpui_kit::test]
fn console_opens_from_a_connection_row(cx: &mut gpui_kit::TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    let (mut cx, explorer, _, _) = tables_open(cx);
    let got = Rc::new(RefCell::new(None));
    let g = got.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&explorer, move |_, e: &super::super::OpenConsole, _| {
            *g.borrow_mut() = Some((e.database.clone(), e.schema.clone()))
        })
    });
    select_label(&explorer, &mut cx, "pg");
    click(&mut cx, "tb-console");
    assert_eq!(*got.borrow(), Some(("app".to_string(), None)));
}
