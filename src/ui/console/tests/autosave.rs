//! The console text saves to its file `SAVE_DELAY` after the last edit.

use super::super::SAVE_DELAY;
use super::run::console;

#[gpui_kit::test]
fn typing_then_waiting_writes_the_file_once(cx: &mut gpui_kit::TestAppContext) {
    let (_, mut cx, console, _) = console(cx, "");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("consoles").join("7.sql");
    console.update(&mut cx, |c, _| c.set_file(file.clone()));
    let half = SAVE_DELAY / 2;
    cx.simulate_input("select");
    cx.executor().advance_clock(half);
    cx.simulate_input(" 1");
    cx.executor().advance_clock(half);
    cx.run_until_parked();
    assert!(!file.exists(), "the second edit restarts the wait");
    cx.executor().advance_clock(half);
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "select 1");
    std::fs::remove_file(&file).unwrap();
    cx.executor().advance_clock(SAVE_DELAY * 4);
    cx.run_until_parked();
    assert!(!file.exists(), "written once");
}
