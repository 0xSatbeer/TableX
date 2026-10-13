//! Loading one tree level from the driver and storing it in the workspace.

use super::*;

/// `params` pointed at database `db`: a Postgres server's other database, or the SQLite file itself.
pub(super) fn on_database(params: &ConnectionParams, db: &str) -> ConnectionParams {
    match params {
        ConnectionParams::Postgres { .. } if params.database() != db => {
            let mut params = params.clone();
            if let ConnectionParams::Postgres { database, .. } = &mut params {
                *database = db.to_string();
            }
            params
        }
        _ => params.clone(),
    }
}

/// What a load of one tree level brings back.
pub(super) enum Children {
    Databases(Vec<String>),
    Schemas(Vec<String>),
    Objects(Vec<DbObject>),
    Columns(Vec<Column>),
}

/// Starts the load of the children of the node whose key has `segments`.
pub(super) fn fetch(
    driver: &dyn Driver,
    session: &Session,
    segments: &[String],
) -> futures::future::BoxFuture<'static, Result<Children, String>> {
    use futures::TryFutureExt as _;
    match segments {
        [_] => Box::pin(driver.databases(session).map_ok(Children::Databases)),
        [_, _] => Box::pin(driver.schemas(session).map_ok(Children::Schemas)),
        [_, _, schema] => Box::pin(driver.objects(session, schema).map_ok(Children::Objects)),
        [_, _, schema, _, object] => Box::pin(
            driver
                .columns(session, schema, object)
                .map_ok(Children::Columns),
        ),
        _ => Box::pin(futures::future::ready(Err("nothing to load".to_string()))),
    }
}

/// A node of the tree whose children load: connection, database, schema or table-like object.
pub(super) enum NodeMut<'a> {
    Connection(&'a mut Connection),
    Database(&'a mut Database),
    Schema(&'a mut Schema),
    Object(&'a mut DbObject),
}

impl NodeMut<'_> {
    /// Whether its children are loaded.
    pub(super) fn loaded(&self) -> bool {
        match self {
            NodeMut::Connection(conn) => conn.databases.is_some(),
            NodeMut::Database(db) => db.schemas.is_some(),
            NodeMut::Schema(schema) => schema.objects.is_some(),
            NodeMut::Object(object) => object.columns.is_some(),
        }
    }

    /// Drops its children, so the next expand or Retry loads them afresh.
    pub(super) fn unload(self) {
        match self {
            NodeMut::Connection(conn) => conn.databases = None,
            NodeMut::Database(db) => db.schemas = None,
            NodeMut::Schema(schema) => schema.objects = None,
            NodeMut::Object(object) => object.columns = None,
        }
    }
}

/// The node with `segments` in connection `id`'s tree.
pub(super) fn locate<'a>(
    ws: &'a mut Workspace,
    id: ConnectionId,
    segments: &[String],
) -> Option<NodeMut<'a>> {
    let conn = ws.connections.iter_mut().find(|c| c.id == id)?;
    let Some(name) = segments.get(1) else {
        return Some(NodeMut::Connection(conn));
    };
    let db = conn
        .databases
        .iter_mut()
        .flatten()
        .find(|d| &d.name == name)?;
    let Some(name) = segments.get(2) else {
        return Some(NodeMut::Database(db));
    };
    let schema = db.schemas.iter_mut().flatten().find(|s| &s.name == name)?;
    let (Some(kind), Some(name)) = (segments.get(3), segments.get(4)) else {
        return Some(NodeMut::Schema(schema));
    };
    schema
        .objects
        .iter_mut()
        .flatten()
        .find(|o| o.kind.as_str() == kind && &o.name == name)
        .map(NodeMut::Object)
}

/// Puts `children` in connection `id`'s tree, under the node with `segments`. Returns the key
/// tails (`name`, or `kind/name` for objects) of that node's children before and after, so the caller can follow what came and went.
/// A child that is still there and that `keep` names by its tail keeps its own loaded children until they are re-read.
pub(super) fn store(
    ws: &mut Workspace,
    id: ConnectionId,
    segments: &[String],
    children: Children,
    keep: impl Fn(&str) -> bool,
) -> (Vec<String>, Vec<String>) {
    let db_tail = |d: &Database| seg(&d.name);
    let schema_tail = |s: &Schema| seg(&s.name);
    let object_tail = |o: &DbObject| format!("{}/{}", o.kind.as_str(), seg(&o.name));
    match (locate(ws, id, segments), children) {
        (Some(NodeMut::Connection(conn)), Children::Databases(names)) => {
            let mut old = conn.databases.take().unwrap_or_default();
            let mut kept = |name: &str| {
                old.iter_mut()
                    .find(|d| d.name == name && keep(&db_tail(d)))
                    .and_then(|d| d.schemas.take())
            };
            let new = names
                .into_iter()
                .map(|name| Database {
                    schemas: kept(&name),
                    name,
                })
                .collect::<Vec<_>>();
            let old = old.iter().map(db_tail).collect();
            (
                old,
                conn.databases.insert(new).iter().map(db_tail).collect(),
            )
        }
        (Some(NodeMut::Database(db)), Children::Schemas(names)) => {
            let mut old = db.schemas.take().unwrap_or_default();
            let mut kept = |name: &str| {
                old.iter_mut()
                    .find(|s| s.name == name && keep(&schema_tail(s)))
                    .and_then(|s| s.objects.take())
            };
            let new = names
                .into_iter()
                .map(|name| Schema {
                    objects: kept(&name),
                    name,
                })
                .collect::<Vec<_>>();
            let old = old.iter().map(schema_tail).collect();
            (
                old,
                db.schemas.insert(new).iter().map(schema_tail).collect(),
            )
        }
        (Some(NodeMut::Schema(schema)), Children::Objects(mut objects)) => {
            let mut old = schema.objects.take().unwrap_or_default();
            for object in &mut objects {
                let tail = object_tail(object);
                object.columns = old
                    .iter_mut()
                    .find(|o| object_tail(o) == tail && keep(&tail))
                    .and_then(|o| o.columns.take());
            }
            let old = old.iter().map(object_tail).collect();
            (
                old,
                schema
                    .objects
                    .insert(objects)
                    .iter()
                    .map(object_tail)
                    .collect(),
            )
        }
        (Some(NodeMut::Object(object)), Children::Columns(columns)) => {
            object.columns = Some(columns);
            (Vec::new(), Vec::new())
        }
        _ => (Vec::new(), Vec::new()),
    }
}
