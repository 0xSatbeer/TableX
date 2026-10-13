//! The `--demo` driver: serves the trees of `mock::workspaces()` after `mock::latency()`.

use std::sync::Arc;

use futures::FutureExt;
use gpui_kit::BackgroundExecutor;

use super::{Driver, Reply, Session, foreign};
use crate::model::{
    Column, ConnectionParams, Database, DbObject, ForeignKey, Index, Key, Role, TableDef,
};

pub struct Mock {
    /// Times the simulated latency, so tests can advance it by hand.
    executor: BackgroundExecutor,
}

/// A session on one database of a sample connection's tree.
pub struct MockSession {
    database: String,
    tree: Vec<Database>,
}

impl Mock {
    pub fn new(executor: BackgroundExecutor) -> Self {
        Self { executor }
    }

    /// `result`, delivered after the latency set on the calling (UI) thread.
    fn after_latency<T: Send + 'static>(&self, result: Result<T, String>) -> Reply<T> {
        let timer = self.executor.timer(crate::mock::latency());
        async move {
            timer.await;
            result
        }
        .boxed()
    }

    fn on<T: Send + 'static>(
        &self,
        session: &Session,
        read: impl FnOnce(&MockSession) -> Result<T, String>,
    ) -> Reply<T> {
        match session {
            Session::Mock(session) => self.after_latency(read(session)),
            _ => foreign(),
        }
    }
}

/// The server or file `params` point at, ignoring the database, which a session picks itself.
fn server(params: &ConnectionParams) -> ConnectionParams {
    let mut params = params.clone();
    if let ConnectionParams::Postgres {
        database, password, ..
    } = &mut params
    {
        (*database, *password) = (String::new(), None);
    }
    params
}

/// The sample tree of the connection at `params`' server, or none for a server the samples lack.
fn tree(params: &ConnectionParams) -> Vec<Database> {
    let conn = crate::mock::workspaces()
        .into_iter()
        .flat_map(|ws| ws.connections)
        .find(|c| server(&c.params) == server(params));
    conn.and_then(|c| c.databases).unwrap_or_default()
}

/// Fails like an unreachable host for a Postgres host ending in `.invalid`.
fn reach(params: &ConnectionParams) -> Result<(), String> {
    match params {
        ConnectionParams::Postgres { host, .. } => crate::mock::test_connection(host),
        ConnectionParams::Sqlite { .. } => Ok(()),
    }
}

impl MockSession {
    /// The database the session is open on; a SQLite file's one database is its first.
    fn database(&self) -> Option<&Database> {
        self.tree
            .iter()
            .find(|d| d.name == self.database)
            .or(self.tree.first())
    }

    fn objects(&self, schema: &str) -> Vec<DbObject> {
        let schema = self
            .database()
            .and_then(|d| d.schemas.iter().flatten().find(|s| s.name == schema));
        schema.and_then(|s| s.objects.clone()).unwrap_or_default()
    }
}

impl Driver for Mock {
    fn connect(&self, params: &ConnectionParams) -> Reply<Session> {
        let session = reach(params).map(|()| {
            Session::Mock(Arc::new(MockSession {
                database: params.database(),
                tree: tree(params),
            }))
        });
        self.after_latency(session)
    }

    fn test(&self, params: &ConnectionParams) -> Reply<()> {
        self.after_latency(reach(params))
    }

    fn databases(&self, session: &Session) -> Reply<Vec<String>> {
        self.on(session, |s| {
            Ok(s.tree.iter().map(|d| d.name.clone()).collect())
        })
    }

    fn schemas(&self, session: &Session) -> Reply<Vec<String>> {
        self.on(session, |s| {
            Ok(s.database()
                .map(|d| d.schemas.iter().flatten().map(|s| s.name.clone()).collect())
                .unwrap_or_default())
        })
    }

    fn objects(&self, session: &Session, schema: &str) -> Reply<Vec<DbObject>> {
        self.on(session, |s| {
            Ok(s.objects(schema)
                .into_iter()
                .map(|o| DbObject { columns: None, ..o })
                .collect())
        })
    }

    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>> {
        self.on(session, |s| {
            Ok(s.objects(schema)
                .into_iter()
                .find(|o| o.name == object)
                .and_then(|o| o.columns)
                .unwrap_or_default())
        })
    }

    fn execute(&self, _: &Session, _: Vec<String>, _: bool) -> Reply<()> {
        self.after_latency(Ok(()))
    }

    fn roles(&self, _: &Session) -> Reply<Vec<Role>> {
        let role = |name: &str, can_login| Role {
            name: name.into(),
            can_login,
        };
        self.after_latency(Ok(vec![
            role("postgres", true),
            role("app_user", true),
            role("readonly", false),
        ]))
    }

    fn database_owner(&self, _: &Session) -> Reply<String> {
        self.after_latency(Ok("postgres".into()))
    }

    fn schema_owner(&self, _: &Session, _: &str) -> Reply<String> {
        self.after_latency(Ok("postgres".into()))
    }

    fn table_def(&self, session: &Session, schema: &str, table: &str) -> Reply<TableDef> {
        self.on(session, |s| {
            let object = s.objects(schema).into_iter().find(|o| o.name == table);
            object
                .map(|o| table_def(schema, o))
                .ok_or_else(|| format!("relation \"{schema}.{table}\" does not exist"))
        })
    }
}

/// `object`'s definition; its primary key's index backs a constraint, so it is left out.
fn table_def(schema: &str, object: DbObject) -> TableDef {
    let backing: Vec<&str> = object
        .keys
        .iter()
        .filter_map(|k| match k {
            Key::Primary { name, .. } | Key::Unique { name, .. } => Some(name.as_str()),
            Key::Foreign { .. } => None,
        })
        .collect();
    let foreign_keys = object
        .keys
        .iter()
        .filter_map(|k| match k {
            Key::Foreign {
                name,
                columns,
                ref_table,
                ref_columns,
            } => Some(ForeignKey {
                name: name.clone(),
                columns: columns.clone(),
                ref_schema: schema.into(),
                ref_table: ref_table.clone(),
                ref_columns: ref_columns.clone(),
                on_delete: "NO ACTION".into(),
                on_update: "NO ACTION".into(),
            }),
            _ => None,
        })
        .collect();
    let primary_key = object.keys.iter().find_map(|k| match k {
        Key::Primary { name, .. } => Some(name.clone()),
        _ => None,
    });
    let mut unique_keys: Vec<Index> = object
        .keys
        .iter()
        .filter_map(|k| match k {
            Key::Unique { name, columns } => Some(Index {
                name: name.clone(),
                columns: columns.clone(),
                unique: true,
            }),
            _ => None,
        })
        .collect();
    unique_keys.sort_by(|a, b| a.name.cmp(&b.name));
    TableDef {
        indexes: object
            .indexes
            .iter()
            .filter(|i| !backing.contains(&i.name.as_str()))
            .cloned()
            .collect(),
        name: object.name,
        columns: object.columns.unwrap_or_default(),
        foreign_keys,
        primary_key,
        unique_keys,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ForeignKey, Index, Key, Role};

    #[gpui_kit::test]
    fn roles_and_table_def_come_from_the_sample_tree(cx: &mut gpui_kit::TestAppContext) {
        let conn = crate::mock::workspaces()[0].connections[0].clone();
        let database = &conn.databases.as_ref().unwrap()[0];
        let schema = &database.schemas.as_ref().unwrap()[0];
        let table = schema
            .objects
            .iter()
            .flatten()
            .find(|o| o.keys.iter().any(|k| matches!(k, Key::Foreign { .. })))
            .expect("a sample table with a foreign key")
            .clone();
        let mock = Mock::new(cx.background_executor.clone());
        let mut params = conn.params.clone();
        if let ConnectionParams::Postgres { database: d, .. } = &mut params {
            *d = database.name.clone();
        }
        let mut connect = mock.connect(&params);
        cx.run_until_parked();
        let session = (&mut connect).now_or_never().unwrap().unwrap();

        let mut roles = mock.roles(&session);
        let mut owner = mock.schema_owner(&session, &schema.name);
        let mut def = mock.table_def(&session, &schema.name, &table.name);
        cx.run_until_parked();
        let role = |name: &str, can_login| Role {
            name: name.into(),
            can_login,
        };
        assert_eq!(
            (&mut roles).now_or_never(),
            Some(Ok(vec![
                role("postgres", true),
                role("app_user", true),
                role("readonly", false)
            ]))
        );
        assert_eq!(
            (&mut owner).now_or_never(),
            Some(Ok("postgres".to_string()))
        );
        let def = (&mut def).now_or_never().unwrap().unwrap();
        assert_eq!(def.name, table.name);
        assert_eq!(def.primary_key, Some(format!("{}_pkey", table.name)));
        assert_eq!(def.unique_keys, vec![]);
        assert_eq!(Some(def.columns), table.columns);
        assert_eq!(
            def.indexes,
            vec![Index {
                name: format!("{}_owner_id_idx", table.name),
                columns: vec!["owner_id".into()],
                unique: false
            }]
        );
        assert_eq!(
            def.foreign_keys,
            vec![ForeignKey {
                name: format!("{}_owner_id_fkey", table.name),
                columns: vec!["owner_id".into()],
                ref_schema: schema.name.clone(),
                ref_table: "users".into(),
                ref_columns: vec!["id".into()],
                on_delete: "NO ACTION".into(),
                on_update: "NO ACTION".into(),
            }]
        );
    }
}
