//! The console completion cache, fed by the explorer's model and sessions.

use std::collections::HashSet;

use super::*;
use crate::complete::SchemaSnapshot;
use crate::model::ConnectionId;
use crate::ui::console::{Target, completion};

fn snapshot(
    workspace: &gpui_kit::Entity<crate::model::Workspace>,
    fake: &Arc<crate::db::fake::Fake>,
    cx: &mut gpui_kit::VisualTestContext,
    words: &[&str],
) -> SchemaSnapshot {
    let target = Target {
        workspace: workspace.clone(),
        driver: fake.clone(),
        connection: Some(ConnectionId(1)),
        database: "app".into(),
        schema: None,
    };
    let words: HashSet<String> = words.iter().map(|w| w.to_string()).collect();
    cx.update(|_, cx| completion::snapshot(&target, &words, cx))
}

fn columns_of(snapshot: &SchemaSnapshot, table: &str) -> Option<Vec<String>> {
    let objects = &snapshot.objects["public"];
    let object = objects.iter().find(|o| o.name == table)?;
    let columns = object.columns.as_ref()?;
    Some(columns.iter().map(|c| c.name.clone()).collect())
}

#[gpui_kit::test]
fn uncached_columns_are_read_once_in_the_background_and_show_next_time(
    cx: &mut gpui_kit::TestAppContext,
) {
    let (mut cx, _, workspace, fake) = tables_open(cx);
    let first = snapshot(&workspace, &fake, &mut cx, &["t"]);
    assert_eq!(columns_of(&first, "t"), None, "never waits for them");
    assert_eq!(fake.pending(), ["columns app/public/t"]);
    snapshot(&workspace, &fake, &mut cx, &["t"]);
    assert_eq!(fake.pending(), ["columns app/public/t"], "asked once");

    fake.resolve("columns app/public/t", Ok(vec![column("id")]));
    cx.run_until_parked();
    let next = snapshot(&workspace, &fake, &mut cx, &["t"]);
    assert_eq!(columns_of(&next, "t"), Some(vec!["id".to_string()]));
    assert!(fake.pending().is_empty());
}

#[gpui_kit::test]
fn refreshing_a_schema_clears_its_cache(cx: &mut gpui_kit::TestAppContext) {
    let (mut cx, explorer, workspace, fake) = tables_open(cx);
    let seeded = snapshot(&workspace, &fake, &mut cx, &[]);
    assert_eq!(seeded.objects["public"].len(), 2, "seeded from the model");
    assert!(fake.pending().is_empty());

    refresh(&explorer, &mut cx, "public");
    fake.resolve("objects app/public", Ok(vec![table("t")]));
    cx.run_until_parked();
    snapshot(&workspace, &fake, &mut cx, &[]);
    assert_eq!(fake.pending(), ["objects app/public"], "read again");
    fake.resolve("objects app/public", Ok(vec![table("t")]));
    cx.run_until_parked();
    let next = snapshot(&workspace, &fake, &mut cx, &[]);
    assert_eq!(next.objects["public"].len(), 1);
}
