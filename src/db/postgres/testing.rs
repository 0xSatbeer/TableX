use super::super::runtime;
use crate::model::{ConnectionParams, Secret, SslMode};

/// The test server, or `None` after printing why the test is skipped.
pub fn test_pg() -> Option<ConnectionParams> {
    params_from(std::env::var("TABLEX_TEST_PG").ok().as_deref())
}

/// Params for the `postgres://user:pass@host:port/db` URL, or `None` when it is unset or empty.
pub fn params_from(url: Option<&str>) -> Option<ConnectionParams> {
    let Some(url) = url.map(str::trim).filter(|url| !url.is_empty()) else {
        println!("skipped: TABLEX_TEST_PG not set");
        return None;
    };
    let config: tokio_postgres::Config = url.parse().expect("TABLEX_TEST_PG is a postgres:// URL");
    let tokio_postgres::config::Host::Tcp(host) = &config.get_hosts()[0] else {
        panic!("TABLEX_TEST_PG needs a TCP host")
    };
    Some(ConnectionParams::Postgres {
        host: host.clone(),
        port: config.get_ports().first().copied().unwrap_or(5432),
        user: config.get_user().unwrap_or_default().to_string(),
        password: config
            .get_password()
            .map(|p| Secret::new(String::from_utf8_lossy(p))),
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
    let ConnectionParams::Postgres {
        host,
        port,
        user,
        password,
        database,
        ..
    } = pg
    else {
        unreachable!()
    };
    let mut config = tokio_postgres::Config::new();
    config.host(host).port(*port).user(user).dbname(database);
    if let Some(password) = password {
        config.password(password.expose());
    }
    runtime().block_on(async {
        let (client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .expect("connect to TABLEX_TEST_PG");
        tokio::spawn(connection);
        let messages = client
            .simple_query(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        messages
            .into_iter()
            .filter_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(row) => {
                    Some(row.get(0).unwrap_or_default().to_string())
                }
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
        Self {
            name,
            drop,
            pg: pg.clone(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.drop.starts_with("DROP DATABASE") {
            admin(
                &self.pg,
                &format!("ALTER DATABASE {} IS_TEMPLATE false", self.name),
            );
        }
        admin(&self.pg, &self.drop);
    }
}

#[test]
fn test_pg_skips_when_unset_and_each_test_gets_unique_names() {
    let pg = |host: &str, port, user: &str, password: Option<&str>, database: &str| {
        ConnectionParams::Postgres {
            host: host.into(),
            port,
            user: user.into(),
            password: password.map(Secret::new),
            database: database.into(),
            ssl: SslMode::Prefer,
        }
    };
    // (TABLEX_TEST_PG, params)
    let cases = [
        (None, None),
        (Some(""), None),
        (Some("  "), None),
        (
            Some("postgres://tablex:tablex@localhost:55432/tablex"),
            Some(pg("localhost", 55432, "tablex", Some("tablex"), "tablex")),
        ),
        (
            Some("postgres://bob@db.local/app"),
            Some(pg("db.local", 5432, "bob", None, "app")),
        ),
    ];
    for (url, want) in cases {
        assert_eq!(params_from(url), want, "{url:?}");
    }
    assert_ne!(unique("t"), unique("t"));

    let Some(pg) = test_pg() else { return };
    let exists = |name: &str| {
        admin(
            &pg,
            &format!("SELECT count(*) FROM pg_namespace WHERE nspname = '{name}'"),
        ) == ["1"]
    };
    let (a, b) = (
        Scratch::new(&pg, "SCHEMA", ""),
        Scratch::new(&pg, "SCHEMA", ""),
    );
    assert_ne!(a.name, b.name);
    assert!(exists(&a.name) && exists(&b.name));
    let name = a.name.clone();
    drop(a);
    assert!(!exists(&name), "the guard drops its schema");
}
