//! A driver whose calls stay pending until the test resolves them by hand, so UI tests control
//! timing and never touch tokio.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::channel::oneshot;

use super::{Driver, Reply, Session};
use crate::model::{Column, ConnectionParams, DbObject};

type Answer = Result<Box<dyn Any + Send>, String>;

/// A fake session: the database it was opened on, and whether the server has dropped it.
pub struct FakeSession {
    pub database: String,
    closed: AtomicBool,
}

impl FakeSession {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

/// A session on `database`, to resolve a connect with.
pub fn session(database: &str) -> Session {
    Session::Fake(Arc::new(FakeSession { database: database.into(), closed: AtomicBool::new(false) }))
}

/// Closes `session` from the server's side, as a server restart would.
pub fn close(session: &Session) {
    if let Session::Fake(fake) = session {
        fake.closed.store(true, Ordering::Relaxed);
    }
}

/// Each call is labelled `"{call} {database}"`, plus `/{schema}` and `/{object}` where it takes them,
/// e.g. `"connect app"`, `"objects app/public"`, `"test local.db"`. A connect resolves with a
/// [`session`] naming its database.
#[derive(Default)]
pub struct Fake {
    pending: Mutex<Vec<(String, oneshot::Sender<Answer>)>>,
    tested: Mutex<Vec<ConnectionParams>>,
}

impl Fake {
    /// The labels of the calls still waiting, oldest first.
    pub fn pending(&self) -> Vec<String> {
        self.pending.lock().unwrap().iter().map(|(label, _)| label.clone()).collect()
    }

    /// The params of every `test` call so far, oldest first.
    pub fn tested(&self) -> Vec<ConnectionParams> {
        self.tested.lock().unwrap().clone()
    }

    /// Resolves the oldest pending call labelled `label` with `result`.
    #[track_caller]
    pub fn resolve<T: Send + 'static>(&self, label: &str, result: Result<T, String>) {
        let mut pending = self.pending.lock().unwrap();
        let Some(ix) = pending.iter().position(|(l, _)| l == label) else {
            let waiting: Vec<_> = pending.iter().map(|(l, _)| l.clone()).collect();
            drop(pending); // unlocked first, so the panic doesn't poison the fake
            panic!("no pending call {label:?}; waiting: {waiting:?}");
        };
        let (_, tx) = pending.remove(ix);
        tx.send(result.map(|value| Box::new(value) as Box<dyn Any + Send>)).ok();
    }

    fn call<T: Send + 'static>(&self, label: String) -> Reply<T> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().push((label.clone(), tx));
        rx.map(move |answer| match answer {
            Ok(answer) => answer.map(|value| *value.downcast::<T>().unwrap_or_else(|_| panic!("{label} resolved with the wrong type"))),
            Err(_) => Err("the database task stopped".into()),
        })
        .boxed()
    }
}

fn on(session: &Session) -> &str {
    match session {
        Session::Fake(fake) => &fake.database,
        _ => panic!("the fake driver got a real session"),
    }
}

impl Driver for Fake {
    fn connect(&self, params: &ConnectionParams) -> Reply<Session> {
        self.call(format!("connect {}", params.database()))
    }

    fn test(&self, params: &ConnectionParams) -> Reply<()> {
        self.tested.lock().unwrap().push(params.clone());
        self.call(format!("test {}", params.database()))
    }

    fn databases(&self, session: &Session) -> Reply<Vec<String>> {
        self.call(format!("databases {}", on(session)))
    }

    fn schemas(&self, session: &Session) -> Reply<Vec<String>> {
        self.call(format!("schemas {}", on(session)))
    }

    fn objects(&self, session: &Session, schema: &str) -> Reply<Vec<DbObject>> {
        self.call(format!("objects {}/{schema}", on(session)))
    }

    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>> {
        self.call(format!("columns {}/{schema}/{object}", on(session)))
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
        DbObject { kind: ObjectKind::Table, name: name.into(), columns: None, keys: Vec::new(), indexes: Vec::new(), row_estimate: None }
    }

    #[test]
    fn calls_stay_pending_until_the_test_resolves_them() {
        let fake = Fake::default();
        let mut connect = fake.connect(&pg("app"));
        let mut test = fake.test(&ConnectionParams::Sqlite { path: PathBuf::from("/x/local.db") });
        assert!((&mut connect).now_or_never().is_none(), "connect waits for the test");
        assert_eq!(fake.pending(), ["connect app", "test local.db"]);

        fake.resolve("connect app", Ok(session("app")));
        let session = (&mut connect).now_or_never().expect("resolved").expect("Ok");
        assert!(matches!(&session, Session::Fake(fake) if fake.database == "app"));
        fake.resolve::<()>("test local.db", Err("No file at /x/local.db".into()));
        assert_eq!((&mut test).now_or_never(), Some(Err("No file at /x/local.db".to_string())));

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
        assert_eq!(fake.pending(), ["databases app"], "and leaves the fake usable");
    }
}
