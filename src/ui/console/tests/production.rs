use gpui_kit::{Entity, VisualTestContext};

use super::super::Console;
use super::run::{connect, console};
use crate::db::Opened;
use crate::db::fake::Fake;

fn make_production(console: &Entity<Console>, cx: &mut VisualTestContext) {
    let workspace = console.read_with(cx, |c, _| c.target.workspace.clone());
    workspace.update(cx, |ws, _| ws.connections[0].is_production = true);
}

fn run_script(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("ctrl-shift-enter");
    cx.run_until_parked();
}

fn asks(cx: &mut VisualTestContext) -> bool {
    cx.debug_bounds("prod-run-preview").is_some()
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).expect("button drawn");
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

fn ran_update(fake: &Fake, cx: &mut VisualTestContext) {
    connect(fake, cx);
    fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
    cx.run_until_parked();
    assert_eq!(
        fake.opened_with(),
        [("update t set a = 1".to_string(), true)]
    );
}

#[gpui_kit::test]
fn an_update_on_production_asks_and_runs_only_once_confirmed(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "select 1;\nupdate t set a = 1");
    make_production(&console, &mut cx);
    run_script(&mut cx);
    let name = console.read_with(&cx, |c, cx| c.production(cx).expect("production"));
    let title: &'static str = format!("prod-run-title Run 1 writing statement on {name}?").leak();
    assert!(cx.debug_bounds(title).is_some(), "asks first");
    assert!(fake.pending().is_empty(), "nothing runs until confirmed");
    click(&mut cx, "prod-run-cancel");
    assert!(!asks(&mut cx));
    assert!(fake.pending().is_empty(), "Cancel runs nothing");
    run_script(&mut cx);
    click(&mut cx, "prod-run-confirm");
    connect(&fake, &mut cx);
    fake.resolve_query("open app_db", Ok(Opened::Affected(0)));
    cx.run_until_parked();
    fake.resolve_query("open app_db", Ok(Opened::Affected(1)));
    cx.run_until_parked();
    let opened: Vec<String> = fake.opened_with().into_iter().map(|(s, _)| s).collect();
    assert_eq!(opened, ["select 1", "update t set a = 1"]);
}

#[gpui_kit::test]
fn a_select_on_production_runs_straight_away(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, console, _) = console(cx, "select 1");
    make_production(&console, &mut cx);
    run_script(&mut cx);
    assert!(!asks(&mut cx));
    connect(&fake, &mut cx);
    fake.resolve_query("open app_db", Ok(Opened::Affected(0)));
    cx.run_until_parked();
    assert_eq!(fake.opened_with(), [("select 1".to_string(), true)]);
}

#[gpui_kit::test]
fn an_update_off_production_runs_straight_away(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, _console, _) = console(cx, "update t set a = 1");
    run_script(&mut cx);
    assert!(!asks(&mut cx));
    ran_update(&fake, &mut cx);
}
