//! A driver whose calls stay pending until the test resolves them by hand, so UI tests control
//! timing and never touch tokio.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::channel::oneshot;

use super::{Driver, Opened, QueryReply, Reply, Session, server_error};
use crate::model::{
    Column, ConnectionParams, DbObject, QueryError, ResultSet, Role, Row, TableDef,
};

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
    Session::Fake(Arc::new(FakeSession {
        database: database.into(),
        closed: AtomicBool::new(false),
    }))
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
    executed: Mutex<Vec<(String, Vec<String>, bool)>>,
    opened: Mutex<Vec<(String, bool)>>,
}

impl Fake {
    /// The labels of the calls still waiting, oldest first.
    pub fn pending(&self) -> Vec<String> {
        self.pending
            .lock()
            .unwrap()
            .iter()
            .map(|(label, _)| label.clone())
            .collect()
    }

    /// The params of every `test` call so far, oldest first.
    pub fn tested(&self) -> Vec<ConnectionParams> {
        self.tested.lock().unwrap().clone()
    }

    /// The (database, statements, atomic) of every `execute` call so far, oldest first.
    pub fn executed(&self) -> Vec<(String, Vec<String>, bool)> {
        self.executed.lock().unwrap().clone()
    }

    /// The SQL of every `open` call so far, oldest first.
    pub fn opened(&self) -> Vec<String> {
        self.opened_with().into_iter().map(|(sql, _)| sql).collect()
    }

    /// The (SQL, autocommit) of every `open` call so far, oldest first.
    pub fn opened_with(&self) -> Vec<(String, bool)> {
        self.opened.lock().unwrap().clone()
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
        tx.send(result.map(|value| Box::new(value) as Box<dyn Any + Send>))
            .ok();
    }

    /// `resolve` for the query calls (`open`, `fetch`, `query`), which fail with a [`QueryError`].
    #[track_caller]
    pub fn resolve_query<T: Send + 'static>(&self, label: &str, result: Result<T, QueryError>) {
        self.resolve(label, Ok(result));
    }

    fn call_query<T: Send + 'static>(&self, label: String) -> QueryReply<T> {
        self.call::<Result<T, QueryError>>(label)
            .map(|answer| answer.unwrap_or_else(|reason| Err(server_error(reason))))
            .boxed()
    }

    fn call<T: Send + 'static>(&self, label: String) -> Reply<T> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().push((label.clone(), tx));
        rx.map(move |answer| match answer {
            Ok(answer) => answer.map(|value| {
                *value
                    .downcast::<T>()
                    .unwrap_or_else(|_| panic!("{label} resolved with the wrong type"))
            }),
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

    fn execute(&self, session: &Session, statements: Vec<String>, atomic: bool) -> Reply<()> {
        let database = on(session).to_string();
        let label = format!("execute {database}");
        self.executed
            .lock()
            .unwrap()
            .push((database, statements, atomic));
        self.call(label)
    }

    fn roles(&self, session: &Session) -> Reply<Vec<Role>> {
        self.call(format!("roles {}", on(session)))
    }

    fn database_owner(&self, session: &Session) -> Reply<String> {
        self.call(format!("database_owner {}", on(session)))
    }

    fn schema_owner(&self, session: &Session, schema: &str) -> Reply<String> {
        self.call(format!("schema_owner {}/{schema}", on(session)))
    }

    fn table_def(&self, session: &Session, schema: &str, table: &str) -> Reply<TableDef> {
        self.call(format!("table_def {}/{schema}/{table}", on(session)))
    }

    fn open(&self, session: &Session, sql: String, autocommit: bool) -> QueryReply<Opened> {
        self.opened.lock().unwrap().push((sql, autocommit));
        self.call_query(format!("open {}", on(session)))
    }

    fn fetch(&self, session: &Session, _: usize) -> QueryReply<Vec<Row>> {
        self.call_query(format!("fetch {}", on(session)))
    }

    fn close(&self, session: &Session) -> Reply<()> {
        self.call(format!("close {}", on(session)))
    }

    fn cancel(&self, session: &Session) -> Reply<()> {
        self.call(format!("cancel {}", on(session)))
    }

    fn query(&self, session: &Session, _: String) -> QueryReply<ResultSet> {
        self.call_query(format!("query {}", on(session)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ObjectKind, SslMode};
    use std::path::PathBuf;

    fn pg(database: &str) -> ConnectionParams {
        ConnectionParams::Postgres {
            host: "h".into(),
            port: 5432,
            user: "u".into(),
            password: None,
            database: database.into(),
            ssl: SslMode::Prefer,
        }
    }

    fn table(name: &str) -> DbObject {
        DbObject {
            kind: ObjectKind::Table,
            name: name.into(),
            columns: None,
            keys: Vec::new(),
            indexes: Vec::new(),
            row_estimate: None,
        }
    }

    #[test]
    fn calls_stay_pending_until_the_test_resolves_them() {
        let fake = Fake::default();
        let mut connect = fake.connect(&pg("app"));
        let mut test = fake.test(&ConnectionParams::Sqlite {
            path: PathBuf::from("/x/local.db"),
        });
        assert!(
            (&mut connect).now_or_never().is_none(),
            "connect waits for the test"
        );
        assert_eq!(fake.pending(), ["connect app", "test local.db"]);

        fake.resolve("connect app", Ok(session("app")));
        let session = (&mut connect)
            .now_or_never()
            .expect("resolved")
            .expect("Ok");
        assert!(matches!(&session, Session::Fake(fake) if fake.database == "app"));
        fake.resolve::<()>("test local.db", Err("No file at /x/local.db".into()));
        assert_eq!(
            (&mut test).now_or_never(),
            Some(Err("No file at /x/local.db".to_string()))
        );

        let mut databases = fake.databases(&session);
        let mut first = fake.schemas(&session);
        let mut second = fake.schemas(&session);
        let mut objects = fake.objects(&session, "public");
        let mut columns = fake.columns(&session, "public", "t");
        assert_eq!(
            fake.pending(),
            [
                "databases app",
                "schemas app",
                "schemas app",
                "objects app/public",
                "columns app/public/t"
            ]
        );

        fake.resolve("schemas app", Ok(vec!["public".to_string()]));
        assert_eq!(
            (&mut first).now_or_never(),
            Some(Ok(vec!["public".to_string()])),
            "the oldest call with a label resolves first"
        );
        assert!((&mut second).now_or_never().is_none());
        fake.resolve::<Vec<String>>(
            "schemas app",
            Err("permission denied for database app".into()),
        );
        assert_eq!(
            (&mut second).now_or_never(),
            Some(Err("permission denied for database app".to_string()))
        );
        fake.resolve("objects app/public", Ok(vec![table("t")]));
        assert_eq!((&mut objects).now_or_never(), Some(Ok(vec![table("t")])));
        fake.resolve::<Vec<Column>>("columns app/public/t", Ok(Vec::new()));
        assert_eq!((&mut columns).now_or_never(), Some(Ok(Vec::new())));
        assert!((&mut databases).now_or_never().is_none());
        assert_eq!(fake.pending(), ["databases app"]);

        let unknown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fake.resolve::<()>("connect nope", Ok(()))
        }));
        assert!(
            unknown.is_err(),
            "resolving a call nobody made is a test bug"
        );
        assert_eq!(
            fake.pending(),
            ["databases app"],
            "and leaves the fake usable"
        );
    }

    #[test]
    fn ddl_calls_are_labelled_and_executes_are_recorded() {
        let fake = Fake::default();
        let session = session("app");
        let mut execute = fake.execute(&session, vec!["DROP TABLE t".into()], true);
        let _roles = fake.roles(&session);
        let _db = fake.database_owner(&session);
        let _schema = fake.schema_owner(&session, "public");
        let _def = fake.table_def(&session, "public", "t");
        assert_eq!(
            fake.pending(),
            [
                "execute app",
                "roles app",
                "database_owner app",
                "schema_owner app/public",
                "table_def app/public/t"
            ]
        );
        assert_eq!(
            fake.executed(),
            [("app".to_string(), vec!["DROP TABLE t".to_string()], true)]
        );
        fake.resolve("execute app", Ok(()));
        assert_eq!((&mut execute).now_or_never(), Some(Ok(())));
    }

    #[test]
    fn query_calls_are_labelled_and_resolve_with_a_query_error() {
        let fake = Fake::default();
        let session = session("app");
        let mut open = fake.open(&session, "SELECT 1".into(), true);
        let mut fetch = fake.fetch(&session, 500);
        let _close = fake.close(&session);
        let _cancel = fake.cancel(&session);
        let _query = fake.query(&session, "SELECT 1".into());
        assert_eq!(
            fake.pending(),
            [
                "open app",
                "fetch app",
                "close app",
                "cancel app",
                "query app"
            ]
        );
        fake.resolve_query("open app", Ok(Opened::Affected(3)));
        assert_eq!((&mut open).now_or_never(), Some(Ok(Opened::Affected(3))));
        let error = server_error("boom");
        fake.resolve_query::<Vec<Row>>("fetch app", Err(error.clone()));
        assert_eq!((&mut fetch).now_or_never(), Some(Err(error)));
    }
}
