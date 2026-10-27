use super::{Structure, Sub, Table};
use crate::db::fake::{self, Fake};
use crate::model::{Column, ForeignKey, Index, TableDef, Trigger};
use gpui_kit::{AppContext as _, VisualTestContext};
use gpui_kit::{Entity, WeakEntity};
use std::sync::Arc;

/// A Structure view on `public.users` (a view when `view`) of the sample workspace's first
/// connection, reading through a Fake.
fn open(
    view: bool,
    cx: &mut gpui_kit::TestAppContext,
) -> (Arc<Fake>, VisualTestContext, Entity<Structure>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let fake = Arc::new(Fake::default());
    let driver: Arc<dyn crate::db::Driver> = fake.clone();
    let workspace = crate::mock::workspaces().remove(0);
    let connection = workspace.connections[0].id;
    let (window, structure) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            let workspace = cx.new(|_| workspace);
            let handle = window.window_handle();
            let sessions =
                cx.new(|cx| crate::sessions::Sessions::new(workspace, driver.clone(), handle, cx));
            let table = Table {
                driver,
                connection,
                database: "app_db".into(),
                sessions,
                schema: "public".into(),
                name: "users".into(),
                sqlite_table: false,
                view,
                dialect: crate::model::DbKind::Postgres,
            };
            cx.new(|cx| {
                let grid = cx.new(|cx| crate::ui::grid::Grid::table(table.clone(), cx));
                Structure::new(grid, table, "key".into(), WeakEntity::new_invalid(), cx)
            })
        })
        .expect("open test window")
    });
    let vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    (fake, vcx, structure)
}

/// What picking `sub` in the tab's context menu does.
fn show(cx: &mut VisualTestContext, structure: &Entity<Structure>, sub: Sub) {
    cx.update(|window, cx| structure.update(cx, |s, cx| s.show(sub, window, cx)));
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} drawn"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

fn column(name: &str, is_pk: bool) -> Column {
    Column {
        name: name.into(),
        data_type: "int4".into(),
        nullable: !is_pk,
        default: None,
        is_pk,
        enum_values: Vec::new(),
        identity: is_pk,
    }
}

fn users() -> TableDef {
    TableDef {
        name: "users".into(),
        columns: vec![column("id", true), column("team_id", false)],
        indexes: vec![Index {
            name: "users_team_ix".into(),
            columns: vec!["team_id".into()],
            unique: false,
        }],
        foreign_keys: vec![ForeignKey {
            name: "users_team_fk".into(),
            columns: vec!["team_id".into()],
            ref_schema: "public".into(),
            ref_table: "teams".into(),
            ref_columns: vec!["id".into()],
            on_delete: "CASCADE".into(),
            on_update: "NO ACTION".into(),
        }],
        primary_key: Some("users_pkey".into()),
        unique_keys: Vec::new(),
    }
}

fn audit() -> Trigger {
    Trigger {
        name: "users_audit".into(),
        timing: "AFTER".into(),
        events: "INSERT OR UPDATE".into(),
        definition: "CREATE TRIGGER users_audit AFTER INSERT OR UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION audit()".into(),
    }
}

#[gpui_kit::test]
fn structure_loads_on_first_show_and_lists_everything(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, structure) = open(false, cx);
    assert!(
        fake.pending().is_empty(),
        "nothing read before the sub-tab is shown"
    );
    show(&mut cx, &structure, Sub::Structure);
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    fake.resolve("table_def app_db/public/users", Ok(users()));
    fake.resolve("triggers app_db/public/users", Ok(vec![audit()]));
    cx.run_until_parked();
    for selector in [
        "structure-column-id",
        "structure-column-team_id",
        "structure-index-users_team_ix",
        "structure-fk-users_team_fk",
        "structure-trigger-users_audit",
        "structure-modify",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector} drawn");
    }
}

#[gpui_kit::test]
fn empty_sections_say_so_and_ddl_reuses_the_load(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, structure) = open(false, cx);
    show(&mut cx, &structure, Sub::Structure);
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    let bare = TableDef {
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        ..users()
    };
    fake.resolve("table_def app_db/public/users", Ok(bare.clone()));
    fake.resolve("triggers app_db/public/users", Ok(Vec::<Trigger>::new()));
    cx.run_until_parked();
    for selector in [
        "structure-no-indexes",
        "structure-no-fks",
        "structure-no-triggers",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector} drawn");
    }
    show(&mut cx, &structure, Sub::Ddl);
    assert!(
        fake.pending().is_empty(),
        "the DDL reuses the loaded definition"
    );
    let text = structure.read_with(&cx, |s, cx| s.text(cx));
    assert_eq!(text, crate::ddl::table_ddl("public", &bare, &[]));
}

#[gpui_kit::test]
fn f5_on_the_sub_tab_reads_again(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, structure) = open(false, cx);
    show(&mut cx, &structure, Sub::Ddl);
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    fake.resolve("table_def app_db/public/users", Ok(users()));
    fake.resolve("triggers app_db/public/users", Ok(vec![audit()]));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    cx.run_until_parked();
    assert!(
        fake.pending()
            .contains(&"table_def app_db/public/users".to_string())
    );
}

#[gpui_kit::test]
fn a_view_shows_its_definition(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, structure) = open(true, cx);
    show(&mut cx, &structure, Sub::Ddl);
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    fake.resolve(
        "view_def app_db/public/users",
        Ok(" SELECT id FROM users;".to_string()),
    );
    cx.run_until_parked();
    assert_eq!(
        structure.read_with(&cx, |s, cx| s.text(cx)),
        " SELECT id FROM users;"
    );
    assert!(cx.debug_bounds("structure-copy").is_some());
}

#[gpui_kit::test]
fn ddl_says_loading_then_the_error_and_retry_reads_it_again(cx: &mut gpui_kit::TestAppContext) {
    let (fake, mut cx, structure) = open(false, cx);
    show(&mut cx, &structure, Sub::Ddl);
    assert!(cx.debug_bounds("structure-state:Loading…").is_some());
    fake.resolve("connect app_db", Ok(fake::session("app_db")));
    cx.run_until_parked();
    fake.resolve::<TableDef>(
        "table_def app_db/public/users",
        Err("permission denied".into()),
    );
    fake.resolve("triggers app_db/public/users", Ok(Vec::<Trigger>::new()));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("structure-state:Couldn't read the definition: permission denied")
            .is_some()
    );
    click(&mut cx, "structure-retry");
    assert!(cx.debug_bounds("structure-state:Loading…").is_some());
    fake.resolve("table_def app_db/public/users", Ok(users()));
    fake.resolve("triggers app_db/public/users", Ok(vec![audit()]));
    cx.run_until_parked();
    assert!(cx.debug_bounds("structure-retry").is_none());
}
