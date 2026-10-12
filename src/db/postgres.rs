//! The Postgres driver: tokio-postgres on the shared runtime.

use std::time::Duration;

use futures::FutureExt;

use super::{Driver, Reply, Session};
use crate::model::{Column, ConnectionParams, DbObject};

pub struct Postgres {
    /// Wraps the whole connect: TCP, TLS, auth and the first query.
    pub timeout: Duration,
}

impl Default for Postgres {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(10) }
    }
}

pub struct PgSession {
    client: tokio_postgres::Client,
}

fn not_yet<T: Send + 'static>() -> Reply<T> {
    futures::future::ready(Err("not implemented".to_string())).boxed()
}

impl Driver for Postgres {
    fn connect(&self, _params: &ConnectionParams) -> Reply<Session> {
        not_yet()
    }

    fn test(&self, _params: &ConnectionParams) -> Reply<()> {
        not_yet()
    }

    fn databases(&self, _session: &Session) -> Reply<Vec<String>> {
        not_yet()
    }

    fn schemas(&self, _session: &Session) -> Reply<Vec<String>> {
        not_yet()
    }

    fn objects(&self, _session: &Session, _schema: &str) -> Reply<Vec<DbObject>> {
        not_yet()
    }

    fn columns(&self, _session: &Session, _schema: &str, _object: &str) -> Reply<Vec<Column>> {
        not_yet()
    }
}
