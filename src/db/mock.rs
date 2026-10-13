//! The `--demo` driver: serves the trees of `mock::workspaces()` after `mock::latency()`.

use std::sync::Arc;

use futures::FutureExt;
use gpui_kit::BackgroundExecutor;

use super::{Driver, Reply, Session, foreign};
use crate::model::{Column, ConnectionParams, Database, DbObject};

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
}
