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
    /// The labels of the calls still waiting, oldest first.
    pub fn pending(&self) -> Vec<String> {
        Vec::new()
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ObjectKind, SslMode};
    use std::path::PathBuf;

    fn pg(database: &str) -> ConnectionParams {
        ConnectionParams::Postgres { host: "h".into(), port: 5432, user: "u".into(), password: None, database: database.into(), ssl: SslMode::Prefer }
    }

    fn table(name: &str) -> DbObject {
        DbObject { kind: ObjectKind::Table, name: name.into(), columns: Vec::new(), keys: Vec::new(), indexes: Vec::new(), row_estimate: None }
    }

    #[test]
    fn calls_stay_pending_until_the_test_resolves_them() {
        let fake = Fake::default();
        let mut connect = fake.connect(&pg("app"));
        let mut test = fake.test(&ConnectionParams::Sqlite { path: PathBuf::from("/x/local.db") });
        assert!((&mut connect).now_or_never().is_none(), "connect waits for the test");
        assert_eq!(fake.pending(), ["connect app", "test local.db"]);

        fake.resolve("connect app", Ok(Session::Fake("app".into())));
        let session = (&mut connect).now_or_never().expect("resolved").expect("Ok");
        assert!(matches!(&session, Session::Fake(db) if db == "app"));
        fake.resolve::<()>("test local.db", Err("No file at /x/local.db".into()));
        assert_eq!((&mut test).now_or_never(), Some(Err("No file at /x/local.db".to_string())));

        // (call, label)
        let mut databases = fake.databases(&session);
        let mut first = fake.schemas(&session);
        let mut second = fake.schemas(&session);
        let mut objects = fake.objects(&session, "public");
        let mut columns = fake.columns(&session, "public", "t");
        assert_eq!(fake.pending(), ["databases app", "schemas app", "schemas app", "objects app/public", "columns app/public/t"]);

        fake.resolve("schemas app", Ok(vec!["public".to_string()]));
        assert_eq!((&mut first).now_or_never(), Some(Ok(vec!["public".to_string()])), "the oldest call with a label resolves first");
        assert!((&mut second).now_or_never().is_none());
        fake.resolve::<Vec<String>>("schemas app", Err("permission denied for database app".into()));
        assert_eq!((&mut second).now_or_never(), Some(Err("permission denied for database app".to_string())));
        fake.resolve("objects app/public", Ok(vec![table("t")]));
        assert_eq!((&mut objects).now_or_never(), Some(Ok(vec![table("t")])));
        fake.resolve::<Vec<Column>>("columns app/public/t", Ok(Vec::new()));
        assert_eq!((&mut columns).now_or_never(), Some(Ok(Vec::new())));
        assert!((&mut databases).now_or_never().is_none());
        assert_eq!(fake.pending(), ["databases app"]);

        let unknown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fake.resolve::<()>("connect nope", Ok(()))));
        assert!(unknown.is_err(), "resolving a call nobody made is a test bug");
    }
}
