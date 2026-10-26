//! Fixtures shared by the explorer tests: connections, objects and an explorer to drive.

use super::*;

/// A Postgres connection `id` named `name` on database `database`, with nothing loaded.
pub(super) fn pg(id: u32, name: &str, database: &str) -> crate::model::Connection {
    use crate::model::{Connection, ConnectionId, ConnectionParams, SslMode};
    let params = ConnectionParams::Postgres {
        host: "h".into(),
        port: 5432,
        user: "u".into(),
        // In memory, as after a prompt, so connecting doesn't ask.
        password: Some(crate::model::Secret::new("pw")),
        database: database.into(),
        ssl: SslMode::Prefer,
    };
    Connection {
        save_password: false,
        id: ConnectionId(id),
        name: name.into(),
        color: 0,
        is_production: false,
        status: ConnectionStatus::Disconnected,
        params,
        databases: None,
    }
}

/// A table `name` whose columns are not loaded yet.
pub(super) fn table(name: &str) -> crate::model::DbObject {
    crate::model::DbObject {
        kind: ObjectKind::Table,
        name: name.into(),
        columns: None,
        keys: vec![],
        indexes: vec![],
    }
}

/// A column `name int`.
pub(super) fn column(name: &str) -> crate::model::Column {
    crate::model::Column {
        name: name.into(),
        data_type: "int".into(),
        nullable: true,
        default: None,
        is_pk: false,
        enum_values: vec![],
        identity: false,
    }
}

/// A connected `conn` holding one database `db` with one schema `schema` of `objects`.
pub(super) fn loaded(
    mut conn: crate::model::Connection,
    db: &str,
    schema: &str,
    objects: Vec<crate::model::DbObject>,
) -> crate::model::Connection {
    use crate::model::{Database, Schema};
    let schemas = Some(vec![Schema {
        name: schema.into(),
        objects: Some(objects),
    }]);
    (conn.status, conn.databases) = (
        ConnectionStatus::Connected,
        Some(vec![Database {
            name: db.into(),
            schemas,
        }]),
    );
    conn
}

/// An explorer on a workspace of `connections`, reading through a fake driver the test answers.
pub(super) fn open_explorer(
    cx: &mut gpui_kit::TestAppContext,
    connections: Vec<crate::model::Connection>,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<super::Explorer>,
    gpui_kit::Entity<crate::model::Workspace>,
    Arc<crate::db::fake::Fake>,
) {
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let fake = Arc::new(crate::db::fake::Fake::default());
    let driver: Arc<dyn crate::db::Driver> = fake.clone();
    let workspace = cx.update(|cx| {
        cx.new(|_| crate::model::Workspace {
            id_floor: 0,
            id: 1,
            name: "w".into(),
            connections,
        })
    });
    let ws = workspace.clone();
    let (window, explorer) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| super::Explorer::new(ws, driver, window, cx))
        })
        .expect("open test window")
    });
    let cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    (cx, explorer, workspace, fake)
}

/// A SQLite connection `id` named `name` on the file `path`, with nothing loaded.
pub(super) fn sqlite(id: u32, name: &str, path: &str) -> crate::model::Connection {
    crate::model::Connection {
        params: crate::model::ConnectionParams::Sqlite { path: path.into() },
        ..pg(id, name, "")
    }
}

/// Connection `conn`'s params with `password` and `database`.
pub(super) fn pg_params(password: Option<&str>, database: &str) -> crate::model::ConnectionParams {
    use crate::model::{ConnectionParams, Secret, SslMode};
    ConnectionParams::Postgres {
        host: "h".into(),
        port: 5432,
        user: "u".into(),
        password: password.map(Secret::new),
        database: database.into(),
        ssl: SslMode::Prefer,
    }
}

pub(super) fn view(name: &str) -> crate::model::DbObject {
    crate::model::DbObject {
        kind: ObjectKind::View,
        ..table(name)
    }
}

pub(super) fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// Connected `pg` on database app, open down to schema public's tables t and u.
pub(super) fn tables_open(
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<super::Explorer>,
    gpui_kit::Entity<crate::model::Workspace>,
    Arc<crate::db::fake::Fake>,
) {
    let (mut cx, explorer, workspace, fake) = open_explorer(cx, vec![pg(1, "pg", "app")]);
    connect_answered(&explorer, &fake, &mut cx, &["app"]);
    for step in ["app", "public", "Tables"] {
        expand(&explorer, &mut cx, step);
        for pending in fake.pending() {
            answer(&fake, &pending, Ok(()));
        }
        cx.run_until_parked();
    }
    (cx, explorer, workspace, fake)
}

/// The rows of one database expanded by `*`: its schemas, their object groups and objects.
pub(super) const EXPANDED_DB: [&str; 8] =
    ["public", "Tables", "t", "u", "sibling", "Tables", "t", "u"];
