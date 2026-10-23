use std::collections::HashSet;

use gpui_kit::{Entity, VisualTestContext};

use super::super::{Console, completion};
use super::run::{connect, console};
use crate::db::Opened;
use crate::model::ObjectKind;

fn labels(console: &Entity<Console>, cx: &VisualTestContext) -> Vec<(String, Option<String>)> {
    console.read_with(cx, |c, cx| {
        let menu = c.editor.read(cx).completion_menu_state();
        assert!(menu.open, "the completion menu is open");
        let items = menu.items.iter();
        items.map(|i| (i.label.clone(), i.detail.clone())).collect()
    })
}

/// The tables and views of app_db.public in the console's workspace model, sorted as completion ranks them.
fn model_tables(console: &Entity<Console>, cx: &VisualTestContext) -> Vec<String> {
    console.read_with(cx, |c, cx| {
        let ws = c.target().workspace.read(cx);
        let databases = ws.connections[0].databases.as_ref().expect("loaded");
        let db = databases
            .iter()
            .find(|d| d.name == "app_db")
            .expect("app_db");
        let schemas = db.schemas.as_ref().expect("schemas loaded");
        let public = schemas.iter().find(|s| s.name == "public").expect("public");
        let objects = public.objects.as_ref().expect("objects loaded");
        let mut names: Vec<String> = objects
            .iter()
            .filter(|o| {
                matches!(
                    o.kind,
                    ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
                )
            })
            .map(|o| o.name.clone())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    })
}

#[gpui_kit::test]
fn typing_select_from_offers_the_schemas_tables_from_the_explorer_model(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (fake, mut cx, console, _) = console(cx, "");
    cx.simulate_input("SELECT * FROM ");
    cx.run_until_parked();
    let items = labels(&console, &cx);
    let first = &model_tables(&console, &cx)[0];
    assert_eq!(items[0].0, *first);
    assert!(items[0].1.is_some(), "with its kind as detail");
    assert!(fake.pending().is_empty(), "no IO for a loaded schema");

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    let text = console.read_with(&cx, |c, cx| c.text(cx));
    assert_eq!(text, format!("SELECT * FROM {first}"), "Tab accepts");
}

#[gpui_kit::test]
fn a_ddl_run_clears_the_cache_for_the_consoles_schema(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "create table x (a int)");
    let public = |cx: &mut VisualTestContext| {
        let target = console.read_with(cx, |c, _| c.target().clone());
        cx.update(|_, cx| completion::snapshot(&target, &HashSet::new(), cx))
            .objects
            .get("public")
            .map_or(0, Vec::len)
    };
    assert!(public(&mut cx) > 0, "seeded from the explorer's model");
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    connect(&fake, &mut cx);
    fake.resolve_query("open app_db", Ok(Opened::Affected(0)));
    cx.run_until_parked();
    assert_eq!(
        public(&mut cx),
        0,
        "cleared, and not re-seeded from the stale model"
    );
}

#[gpui_kit::test]
fn tab_without_the_completion_menu_indents(cx: &mut gpui_kit::TestAppContext) {
    let (_, mut cx, console, _) = console(cx, "");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    let text = console.read_with(&cx, |c, cx| c.text(cx));
    assert!(
        !text.is_empty() && text.trim().is_empty(),
        "Tab falls through to the editor's indent, got {text:?}"
    );
}
