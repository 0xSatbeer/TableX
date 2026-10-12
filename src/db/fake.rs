//! A driver whose calls stay pending until the test resolves them by hand, so UI tests control
//! timing and never touch tokio.

use futures::FutureExt;

use super::{Driver, Reply, Session};
use crate::model::{Column, ConnectionParams, DbObject};

#[derive(Default)]
pub struct Fake;

fn not_yet<T: Send + 'static>() -> Reply<T> {
    futures::future::ready(Err("not implemented".to_string())).boxed()
}

impl Fake {
    /// Resolves the oldest pending call labelled `label` with `result`.
    pub fn resolve<T: Send + 'static>(&self, _label: &str, _result: Result<T, String>) {}
}

impl Driver for Fake {
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
