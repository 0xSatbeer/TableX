use super::*;

/// The last query of each server session `user` has open.
fn sessions_of(pg: &ConnectionParams, user: &str) -> Vec<String> {
    admin(
        pg,
        &format!("SELECT query FROM pg_stat_activity WHERE usename = '{user}'"),
    )
}

#[test]
fn connect_opens_a_session_and_test_runs_select_1_then_closes_it() {
    let Some(pg) = test_pg() else { return };
    let role = Scratch::new(&pg, "ROLE", "LOGIN PASSWORD 'pw'");
    let params = as_user(&pg, &role.name, Some("pw"));
    let driver = Postgres::default();

    let session = block_on(driver.connect(&params)).expect("connect with valid settings");
    assert!(matches!(session, Session::Postgres(_)));
    assert_eq!(
        sessions_of(&pg, &role.name),
        ["SELECT 1"],
        "connect's first query is SELECT 1"
    );
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
    let ConnectionParams::Postgres {
        host,
        port,
        user,
        password,
        ssl,
        ..
    } = pg
    else {
        unreachable!()
    };
    let params = ConnectionParams::Postgres {
        host,
        port,
        user,
        password,
        database: missing.clone(),
        ssl,
    };
    let got = block_on(Postgres::default().connect(&params)).map(drop);
    assert_eq!(got, Err(format!("database \"{missing}\" does not exist")));
}

fn local(host: &str, port: u16) -> ConnectionParams {
    ConnectionParams::Postgres {
        host: host.into(),
        port,
        user: "u".into(),
        password: None,
        database: "d".into(),
        ssl: SslMode::Disable,
    }
}

#[test]
fn connect_to_nothing_listening_or_an_unresolvable_host_fails_with_a_reason() {
    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
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
    let got = block_on(
        Postgres {
            timeout: Duration::from_millis(200),
        }
        .connect(&params),
    )
    .map(drop);
    let took = started.elapsed();
    assert_eq!(got, Err("timeout expired after 0.2 s".to_string()));
    assert!(
        took >= Duration::from_millis(200) && took < Duration::from_secs(2),
        "took {took:?}"
    );
}

/// A server without TLS in front of `pg`: it declines the SSLRequest with 'N', the way a
/// server with `ssl = off` does, then relays everything else to `pg`.
fn without_tls(pg: &ConnectionParams) -> u16 {
    use std::io::{Read, Write};
    let ConnectionParams::Postgres { host, port, .. } = pg.clone() else {
        unreachable!()
    };
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
                let (mut client_in, mut server_out) =
                    (client.try_clone().unwrap(), server.try_clone().unwrap());
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
        let ConnectionParams::Postgres {
            host,
            port: pg_port,
            user,
            password,
            database,
            ..
        } = pg.clone()
        else {
            unreachable!()
        };
        let host = if port.is_some() {
            "127.0.0.1".to_string()
        } else {
            host
        };
        ConnectionParams::Postgres {
            host,
            port: port.unwrap_or(pg_port),
            user,
            password,
            database,
            ssl,
        }
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
            let Session::Postgres(session) = session else {
                unreachable!()
            };
            let sql = "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()";
            let rows = crate::db::runtime()
                .block_on(session.query(sql, &[]))
                .unwrap();
            rows[0].get::<_, bool>(0)
        });
        match want {
            Some(tls) => assert_eq!(got, Ok(tls), "{ssl:?} via {server:?}"),
            None => assert!(
                got.is_err_and(|reason| !reason.is_empty()),
                "{ssl:?} via {server:?}"
            ),
        }
    }
}
