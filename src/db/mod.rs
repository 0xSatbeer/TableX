//! All database IO. The UI talks to a [`Driver`] and awaits its [`Reply`]s inside `cx.spawn`;
//! real sessions live on one shared tokio runtime (docs/adr/0001).

use std::error::Error as StdError;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::BoxFuture;

use crate::model::{Column, ConnectionParams, DbObject};

#[cfg(test)]
pub mod fake;
pub mod postgres;
pub mod sqlite;

/// A driver call's result, runtime-agnostic: awaiting it never needs tokio. `Err` holds the reason.
pub type Reply<T> = BoxFuture<'static, Result<T, String>>;

/// One open session, opened by the driver that reads from it.
#[derive(Clone)]
pub enum Session {
    Postgres(Arc<postgres::PgSession>),
    Sqlite(Arc<sqlite::SqliteSession>),
    /// The fake driver's session: the database it was opened on.
    #[cfg(test)]
    Fake(String),
}

pub trait Driver: Send + Sync {
    /// Opens a session to the database `params` name (for SQLite, its file).
    fn connect(&self, params: &ConnectionParams) -> Reply<Session>;
    /// Opens a throwaway session exactly as `connect` does, then closes it.
    fn test(&self, params: &ConnectionParams) -> Reply<()>;
    fn databases(&self, session: &Session) -> Reply<Vec<String>>;
    /// The schemas of the database `session` is open on.
    fn schemas(&self, session: &Session) -> Reply<Vec<String>>;
    /// Every object in `schema`, sorted by name within each kind; columns are not loaded.
    fn objects(&self, session: &Session, schema: &str) -> Reply<Vec<DbObject>>;
    /// The columns of table or view `object`, in ordinal order.
    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>>;
}

/// The reason we show for a failure: a server's own message without severity or prefixes,
/// otherwise the innermost error's text.
pub fn reason(err: &(dyn StdError + 'static)) -> String {
    let mut inner = Some(err);
    while let Some(e) = inner {
        if let Some(server) = e.downcast_ref::<tokio_postgres::error::DbError>() {
            return server.message().to_string();
        }
        inner = e.source();
    }
    err.to_string()
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("queryx-db")
            .enable_all()
            .build()
            .expect("start the database runtime")
    })
}

/// Runs `work` on the database runtime.
fn spawn<T: Send + 'static>(work: impl Future<Output = Result<T, String>> + Send + 'static) -> Reply<T> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn(async move { tx.send(work.await).ok() });
    rx.map(|result| result.unwrap_or_else(|_| Err("the database task stopped".into()))).boxed()
}

/// Runs blocking `work` (SQLite) on the database runtime's blocking pool.
fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Reply<T> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn_blocking(move || tx.send(work()).ok());
    rx.map(|result| result.unwrap_or_else(|_| Err("the database task stopped".into()))).boxed()
}

/// A session of the wrong driver: a caller bug, reported instead of panicking.
fn foreign<T: Send + 'static>() -> Reply<T> {
    futures::future::ready(Err("session belongs to another driver".to_string())).boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A server on a local port that answers the first startup message with an ErrorResponse
    /// carrying `severity` and `message`, the way Postgres reports a failed login.
    fn server_rejecting(severity: &'static str, code: &'static str, message: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut len = [0; 4];
            stream.read_exact(&mut len).unwrap();
            let mut startup = vec![0; u32::from_be_bytes(len) as usize - 4];
            stream.read_exact(&mut startup).unwrap();
            let mut fields = Vec::new();
            for (tag, value) in [(b'S', severity), (b'V', severity), (b'C', code), (b'M', message)] {
                fields.push(tag);
                fields.extend_from_slice(value.as_bytes());
                fields.push(0);
            }
            fields.push(0);
            let mut reply = vec![b'E'];
            reply.extend_from_slice(&(fields.len() as u32 + 4).to_be_bytes());
            reply.extend_from_slice(&fields);
            stream.write_all(&reply).unwrap();
        });
        port
    }

    /// The tokio-postgres error a login to `port` as `user` fails with.
    fn login_error(port: u16, user: &str) -> tokio_postgres::Error {
        let mut config = tokio_postgres::Config::new();
        config.host("127.0.0.1").port(port).user(user).ssl_mode(tokio_postgres::config::SslMode::Disable);
        runtime().block_on(config.connect(tokio_postgres::NoTls)).err().expect("the login fails")
    }

    #[test]
    fn reason_of_a_server_error_is_its_message_only() {
        // (severity, SQLSTATE, server message, reason)
        let cases = [
            ("FATAL", "28P01", r#"password authentication failed for user "bob""#, r#"password authentication failed for user "bob""#),
            ("FATAL", "3D000", r#"database "nope" does not exist"#, r#"database "nope" does not exist"#),
            ("ERROR", "42501", "permission denied for schema app", "permission denied for schema app"),
        ];
        for (severity, code, message, want) in cases {
            let err = login_error(server_rejecting(severity, code, message), "bob");
            assert_eq!(reason(&err), want, "{severity} {message}");
        }
    }

    /// A driver error wrapping `source` under a prefix, the shape tokio-postgres builds.
    #[derive(Debug)]
    struct Wrapped(&'static str, Box<dyn StdError + Send + Sync>);

    impl std::fmt::Display for Wrapped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}: {}", self.0, self.1)
        }
    }

    impl StdError for Wrapped {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            Some(&*self.1)
        }
    }

    #[test]
    fn reason_of_a_nested_io_error_is_the_io_text_only() {
        let io = |text: &str| Box::new(std::io::Error::other(text.to_string()));
        // (error, reason)
        let cases: [(Box<dyn StdError + Send + Sync>, &str); 4] = [
            (Box::new(Wrapped("error connecting to server", io("Connection refused (os error 111)"))), "Connection refused (os error 111)"),
            (Box::new(Wrapped("db error", Box::new(Wrapped("error connecting to server", io("no route to host"))))), "no route to host"),
            (io("unexpected EOF"), "unexpected EOF"),
            (Box::new(Wrapped("error performing TLS handshake", "server does not support TLS".into())), "server does not support TLS"),
        ];
        for (err, want) in cases {
            assert_eq!(reason(&*err), want, "{err}");
        }
        // A real refused connect: tokio-postgres' "error connecting to server:" prefix is gone.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = login_error(port, "bob");
        let io = std::iter::successors(Some(&err as &(dyn StdError + 'static)), |e| (*e).source()).last().unwrap().to_string();
        assert_eq!(reason(&err), io, "{err}");
        assert!(!reason(&err).contains("error connecting to server"), "{err}");
    }
}
