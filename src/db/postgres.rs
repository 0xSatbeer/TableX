//! The Postgres driver: tokio-postgres on the shared runtime.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;

use super::{Driver, Reply, Session, reason, spawn, timeout_reason};
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
    /// Drives the socket; ends once the client is dropped and the server has been told goodbye.
    io: tokio::task::JoinHandle<()>,
}

/// Connects with `params` and runs the first query, all within `timeout`.
async fn open(params: ConnectionParams, timeout: Duration) -> Result<PgSession, String> {
    let ConnectionParams::Postgres { host, port, user, password, database, .. } = params else {
        return Err("not a Postgres connection".into());
    };
    let mut config = tokio_postgres::Config::new();
    config.host(&host).port(port).user(&user).dbname(&database);
    if let Some(password) = &password {
        config.password(password.expose());
    }
    let connect = async {
        let (client, connection) = config.connect(tokio_postgres::NoTls).await.map_err(|e| reason(&e))?;
        let io = tokio::spawn(async move {
            connection.await.ok();
        });
        client.simple_query("SELECT 1").await.map_err(|e| reason(&e))?;
        Ok(PgSession { client, io })
    };
    tokio::time::timeout(timeout, connect).await.unwrap_or_else(|_| Err(timeout_reason(timeout)))
}

fn not_yet<T: Send + 'static>() -> Reply<T> {
    futures::future::ready(Err("not implemented".to_string())).boxed()
}

impl Driver for Postgres {
    fn connect(&self, params: &ConnectionParams) -> Reply<Session> {
        let open = open(params.clone(), self.timeout);
        spawn(async move { Ok(Session::Postgres(Arc::new(open.await?))) })
    }

    fn test(&self, params: &ConnectionParams) -> Reply<()> {
        let open = open(params.clone(), self.timeout);
        spawn(async move {
            let PgSession { client, io } = open.await?;
            drop(client);
            io.await.ok();
            Ok(())
        })
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

/// Helpers for tests against the compose Postgres (`QUERYX_TEST_PG`); T02's listing tests use them too.
#[cfg(test)]
pub(crate) mod testing {
    use super::super::runtime;
    use crate::model::{ConnectionParams, Secret, SslMode};

    /// The test server, or `None` after printing why the test is skipped.
    pub fn test_pg() -> Option<ConnectionParams> {
        params_from(std::env::var("QUERYX_TEST_PG").ok().as_deref())
    }

    /// Params for the `postgres://user:pass@host:port/db` URL, or `None` when it is unset or empty.
    pub fn params_from(url: Option<&str>) -> Option<ConnectionParams> {
        let Some(url) = url.map(str::trim).filter(|url| !url.is_empty()) else {
            println!("skipped: QUERYX_TEST_PG not set");
            return None;
        };
        let config: tokio_postgres::Config = url.parse().expect("QUERYX_TEST_PG is a postgres:// URL");
        let tokio_postgres::config::Host::Tcp(host) = &config.get_hosts()[0] else { panic!("QUERYX_TEST_PG needs a TCP host") };
        Some(ConnectionParams::Postgres {
            host: host.clone(),
            port: config.get_ports().first().copied().unwrap_or(5432),
            user: config.get_user().unwrap_or_default().to_string(),
            password: config.get_password().map(|p| Secret::new(String::from_utf8_lossy(p))),
            database: config.get_dbname().unwrap_or_default().to_string(),
            ssl: SslMode::Prefer,
        })
    }

    /// A name no other test, in this run or a parallel one, uses.
    pub fn unique(prefix: &str) -> String {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{prefix}_{}_{n}", std::process::id())
    }

    /// Runs `sql` as the test URL's user and returns the first column of every row.
    pub fn admin(pg: &ConnectionParams, sql: &str) -> Vec<String> {
        let ConnectionParams::Postgres { host, port, user, password, database, .. } = pg else { unreachable!() };
        let mut config = tokio_postgres::Config::new();
        config.host(host).port(*port).user(user).dbname(database);
        if let Some(password) = password {
            config.password(password.expose());
        }
        runtime().block_on(async {
            let (client, connection) = config.connect(tokio_postgres::NoTls).await.expect("connect to QUERYX_TEST_PG");
            tokio::spawn(connection);
            let messages = client.simple_query(sql).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
            messages
                .into_iter()
                .filter_map(|m| match m {
                    tokio_postgres::SimpleQueryMessage::Row(row) => Some(row.get(0).unwrap_or_default().to_string()),
                    _ => None,
                })
                .collect()
        })
    }

    /// A uniquely named schema, role or database that is dropped with the guard.
    pub struct Scratch {
        pub name: String,
        drop: String,
        pg: ConnectionParams,
    }

    impl Scratch {
        /// Creates `CREATE {kind} {name} {options}`, e.g. `("ROLE", "LOGIN PASSWORD 'pw'")`.
        pub fn new(pg: &ConnectionParams, kind: &str, options: &str) -> Self {
            let name = unique(&format!("qx_{}", kind.to_lowercase()));
            admin(pg, &format!("CREATE {kind} {name} {options}"));
            let drop = match kind {
                "SCHEMA" => format!("DROP SCHEMA {name} CASCADE"),
                "DATABASE" => format!("DROP DATABASE {name} WITH (FORCE)"),
                _ => format!("DROP {kind} {name}"),
            };
            Self { name, drop, pg: pg.clone() }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            admin(&self.pg, &self.drop);
        }
    }

    #[test]
    fn test_pg_skips_when_unset_and_each_test_gets_unique_names() {
        let pg = |host: &str, port, user: &str, password: Option<&str>, database: &str| ConnectionParams::Postgres {
            host: host.into(),
            port,
            user: user.into(),
            password: password.map(Secret::new),
            database: database.into(),
            ssl: SslMode::Prefer,
        };
        // (QUERYX_TEST_PG, params)
        let cases = [
            (None, None),
            (Some(""), None),
            (Some("  "), None),
            (Some("postgres://queryx:queryx@localhost:55432/queryx"), Some(pg("localhost", 55432, "queryx", Some("queryx"), "queryx"))),
            (Some("postgres://bob@db.local/app"), Some(pg("db.local", 5432, "bob", None, "app"))),
        ];
        for (url, want) in cases {
            assert_eq!(params_from(url), want, "{url:?}");
        }
        assert_ne!(unique("t"), unique("t"));

        let Some(pg) = test_pg() else { return };
        let exists = |name: &str| admin(&pg, &format!("SELECT count(*) FROM pg_namespace WHERE nspname = '{name}'")) == ["1"];
        let (a, b) = (Scratch::new(&pg, "SCHEMA", ""), Scratch::new(&pg, "SCHEMA", ""));
        assert_ne!(a.name, b.name);
        assert!(exists(&a.name) && exists(&b.name));
        let name = a.name.clone();
        drop(a);
        assert!(!exists(&name), "the guard drops its schema");
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Scratch, admin, test_pg};
    use super::*;
    use crate::model::{Secret, SslMode};
    use futures::executor::block_on;

    /// `pg` logging in as `user` with `password`.
    fn as_user(pg: &ConnectionParams, user: &str, password: Option<&str>) -> ConnectionParams {
        let ConnectionParams::Postgres { host, port, database, ssl, .. } = pg.clone() else { unreachable!() };
        ConnectionParams::Postgres { host, port, user: user.into(), password: password.map(Secret::new), database, ssl }
    }

    /// The last query of each server session `user` has open.
    fn sessions_of(pg: &ConnectionParams, user: &str) -> Vec<String> {
        admin(pg, &format!("SELECT query FROM pg_stat_activity WHERE usename = '{user}'"))
    }

    #[test]
    fn connect_opens_a_session_and_test_runs_select_1_then_closes_it() {
        let Some(pg) = test_pg() else { return };
        let role = Scratch::new(&pg, "ROLE", "LOGIN PASSWORD 'pw'");
        let params = as_user(&pg, &role.name, Some("pw"));
        let driver = Postgres::default();

        let session = block_on(driver.connect(&params)).expect("connect with valid settings");
        assert!(matches!(session, Session::Postgres(_)));
        assert_eq!(sessions_of(&pg, &role.name), ["SELECT 1"], "connect's first query is SELECT 1");
        drop(session);
        let closed = |why: &str| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !sessions_of(&pg, &role.name).is_empty() {
                assert!(std::time::Instant::now() < deadline, "{why}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        closed("dropping the session closes it");

        assert_eq!(block_on(driver.test(&params)), Ok(()));
        closed("test closes its session");
    }

    #[test]
    fn connect_logs_in_with_the_password_and_reports_a_wrong_one() {
        let Some(pg) = test_pg() else { return };
        let role = Scratch::new(&pg, "ROLE", "LOGIN PASSWORD 'pw'");
        let failed = format!("password authentication failed for user \"{}\"", role.name);
        // (user, password, Ok or the reason; "" means any non-empty reason)
        let cases = [
            (role.name.as_str(), Some("pw"), Ok(())),
            (role.name.as_str(), Some("wrong"), Err(failed.as_str())),
            (role.name.as_str(), Some("PW"), Err(failed.as_str())),
            (role.name.as_str(), None, Err("")),
            (role.name.as_str(), Some(""), Err("")),
            // compose.yaml's init script makes the server trust this role.
            ("queryx_trust", None, Ok(())),
        ];
        for (user, password, want) in cases {
            let got = block_on(Postgres::default().connect(&as_user(&pg, user, password))).map(drop);
            match (got, want) {
                (Ok(()), Ok(())) => {}
                (Err(got), Err("")) => assert!(!got.is_empty(), "{user} {password:?}"),
                (got, want) => assert_eq!(got, want.map_err(String::from), "{user} {password:?}"),
            }
        }
    }

    #[test]
    fn connect_to_a_missing_database_names_it() {
        let Some(pg) = test_pg() else { return };
        let missing = super::testing::unique("qx_missing");
        let ConnectionParams::Postgres { host, port, user, password, ssl, .. } = pg else { unreachable!() };
        let params = ConnectionParams::Postgres { host, port, user, password, database: missing.clone(), ssl };
        let got = block_on(Postgres::default().connect(&params)).map(drop);
        assert_eq!(got, Err(format!("database \"{missing}\" does not exist")));
    }

    fn local(host: &str, port: u16) -> ConnectionParams {
        ConnectionParams::Postgres { host: host.into(), port, user: "u".into(), password: None, database: "d".into(), ssl: SslMode::Disable }
    }

    #[test]
    fn connect_to_nothing_listening_or_an_unresolvable_host_fails_with_a_reason() {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        // wording is the OS resolver's or socket's, so only a non-empty reason is asserted
        let cases = [local("127.0.0.1", free), local("qx-nowhere.invalid", 5432)];
        for params in cases {
            match block_on(Postgres::default().connect(&params)) {
                Err(reason) => assert!(!reason.is_empty(), "{params:?}"),
                Ok(_) => panic!("{params:?} connected"),
            }
        }
    }

    #[test]
    fn connect_to_a_server_that_never_answers_times_out() {
        let stall = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let params = local("127.0.0.1", stall.local_addr().unwrap().port());
        let started = std::time::Instant::now();
        let got = block_on(Postgres { timeout: Duration::from_millis(200) }.connect(&params)).map(drop);
        let took = started.elapsed();
        assert_eq!(got, Err("timeout expired after 0.2 s".to_string()));
        assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(2), "took {took:?}");
    }

    /// A server without TLS in front of `pg`: it declines the SSLRequest with 'N', the way a
    /// server with `ssl = off` does, then relays everything else to `pg`.
    fn without_tls(pg: &ConnectionParams) -> u16 {
        use std::io::{Read, Write};
        let ConnectionParams::Postgres { host, port, .. } = pg.clone() else { unreachable!() };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for client in listener.incoming() {
                let (mut client, host) = (client.unwrap(), host.clone());
                std::thread::spawn(move || {
                    let mut first = [0; 8];
                    client.read_exact(&mut first).unwrap();
                    let mut server = std::net::TcpStream::connect((host.as_str(), port)).unwrap();
                    if first[4..] == 80877103u32.to_be_bytes() {
                        client.write_all(b"N").unwrap();
                    } else {
                        server.write_all(&first).unwrap();
                    }
                    let (mut client_in, mut server_out) = (client.try_clone().unwrap(), server.try_clone().unwrap());
                    std::thread::spawn(move || std::io::copy(&mut client_in, &mut server_out));
                    std::io::copy(&mut server, &mut client).ok();
                });
            }
        });
        local
    }

    #[test]
    fn ssl_mode_follows_libpq() {
        let Some(pg) = test_pg() else { return };
        let plain = without_tls(&pg);
        let with_ssl = |ssl, port: Option<u16>| {
            let ConnectionParams::Postgres { host, port: pg_port, user, password, database, .. } = pg.clone() else { unreachable!() };
            let host = if port.is_some() { "127.0.0.1".to_string() } else { host };
            ConnectionParams::Postgres { host, port: port.unwrap_or(pg_port), user, password, database, ssl }
        };
        let (tls_server, plain_server) = (None, Some(plain));
        // (mode, server, Ok(uses TLS) or Err); the compose server's cert is self-signed
        let cases = [
            (SslMode::Disable, plain_server, Some(false)),
            (SslMode::Disable, tls_server, Some(false)),
            (SslMode::Prefer, tls_server, Some(true)),
            (SslMode::Prefer, plain_server, Some(false)),
            (SslMode::Require, tls_server, Some(true)),
            (SslMode::Require, plain_server, None),
        ];
        for (ssl, server, want) in cases {
            let got = block_on(Postgres::default().connect(&with_ssl(ssl, server))).map(|session| {
                let Session::Postgres(session) = session else { unreachable!() };
                let sql = "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()";
                let rows = super::super::runtime().block_on(session.client.query_one(sql, &[])).unwrap();
                rows.get::<_, bool>(0)
            });
            match want {
                Some(tls) => assert_eq!(got, Ok(tls), "{ssl:?} via {server:?}"),
                None => assert!(got.is_err_and(|reason| !reason.is_empty()), "{ssl:?} via {server:?}"),
            }
        }
    }
}
