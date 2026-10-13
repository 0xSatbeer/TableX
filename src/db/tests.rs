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
        for (tag, value) in [
            (b'S', severity),
            (b'V', severity),
            (b'C', code),
            (b'M', message),
        ] {
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
    config
        .host("127.0.0.1")
        .port(port)
        .user(user)
        .ssl_mode(tokio_postgres::config::SslMode::Disable);
    runtime()
        .block_on(config.connect(tokio_postgres::NoTls))
        .err()
        .expect("the login fails")
}

#[test]
fn reason_of_a_server_error_is_its_message_only() {
    // (severity, SQLSTATE, server message, reason)
    let cases = [
        (
            "FATAL",
            "28P01",
            r#"password authentication failed for user "bob""#,
            r#"password authentication failed for user "bob""#,
        ),
        (
            "FATAL",
            "3D000",
            r#"database "nope" does not exist"#,
            r#"database "nope" does not exist"#,
        ),
        (
            "ERROR",
            "42501",
            "permission denied for schema app",
            "permission denied for schema app",
        ),
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
        (
            Box::new(Wrapped(
                "error connecting to server",
                io("Connection refused (os error 111)"),
            )),
            "Connection refused (os error 111)",
        ),
        (
            Box::new(Wrapped(
                "db error",
                Box::new(Wrapped(
                    "error connecting to server",
                    io("no route to host"),
                )),
            )),
            "no route to host",
        ),
        (io("unexpected EOF"), "unexpected EOF"),
        (
            Box::new(Wrapped(
                "error performing TLS handshake",
                "server does not support TLS".into(),
            )),
            "server does not support TLS",
        ),
    ];
    for (err, want) in cases {
        assert_eq!(reason(&*err), want, "{err}");
    }
    // A real refused connect: tokio-postgres' "error connecting to server:" prefix is gone.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let err = login_error(port, "bob");
    let io = std::iter::successors(Some(&err as &(dyn StdError + 'static)), |e| (*e).source())
        .last()
        .unwrap()
        .to_string();
    assert_eq!(reason(&err), io, "{err}");
    assert!(
        !reason(&err).contains("error connecting to server"),
        "{err}"
    );
}

#[test]
fn reason_of_our_connect_timeout_names_the_seconds() {
    use std::time::Duration;
    // (timeout, reason)
    let cases = [
        (Duration::from_secs(10), "timeout expired after 10 s"),
        (Duration::from_millis(200), "timeout expired after 0.2 s"),
        (Duration::from_millis(1500), "timeout expired after 1.5 s"),
        (Duration::from_millis(250), "timeout expired after 0.25 s"),
        (Duration::from_secs(1), "timeout expired after 1 s"),
    ];
    for (after, want) in cases {
        assert_eq!(timeout_reason(after), want, "{after:?}");
    }
}
