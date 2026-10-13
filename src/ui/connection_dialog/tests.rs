use super::{form::PORT_ERROR, *};

fn pg(host: &str, port: &str, user: &str, database: &str) -> Form {
    Form {
        host: host.into(),
        port: port.into(),
        user: user.into(),
        database: database.into(),
        ..Form::new(&[])
    }
}

fn ok_pg() -> Form {
    pg("h", "5432", "u", "d")
}

#[test]
fn field_validation_cases() {
    let port_err = Some(PORT_ERROR.to_string());
    let cases: Vec<(Form, FieldErrors)> = vec![
        (
            pg("", "5432", "u", "d"),
            FieldErrors {
                host: Some("Host can't be empty".into()),
                ..Default::default()
            },
        ),
        (
            pg("  ", "5432", "u", "d"),
            FieldErrors {
                host: Some("Host can't be empty".into()),
                ..Default::default()
            },
        ),
        (
            pg("h", "5432", "", "d"),
            FieldErrors {
                user: Some("User can't be empty".into()),
                ..Default::default()
            },
        ),
        (
            pg("h", "5432", "  ", "d"),
            FieldErrors {
                user: Some("User can't be empty".into()),
                ..Default::default()
            },
        ),
        (
            pg("h", "5432", "u", ""),
            FieldErrors {
                database: Some("Database can't be empty".into()),
                ..Default::default()
            },
        ),
        (
            pg("h", "5432", "u", " "),
            FieldErrors {
                database: Some("Database can't be empty".into()),
                ..Default::default()
            },
        ),
        (
            pg("h", "0", "u", "d"),
            FieldErrors {
                port: port_err.clone(),
                ..Default::default()
            },
        ),
        (
            pg("h", "65536", "u", "d"),
            FieldErrors {
                port: port_err.clone(),
                ..Default::default()
            },
        ),
        (
            pg("h", "-1", "u", "d"),
            FieldErrors {
                port: port_err.clone(),
                ..Default::default()
            },
        ),
        (
            pg("h", "", "u", "d"),
            FieldErrors {
                port: port_err.clone(),
                ..Default::default()
            },
        ),
        (
            pg("h", "54a", "u", "d"),
            FieldErrors {
                port: port_err.clone(),
                ..Default::default()
            },
        ),
        (
            pg("h", "5432.0", "u", "d"),
            FieldErrors {
                port: port_err.clone(),
                ..Default::default()
            },
        ),
        (
            pg("h", "+5432", "u", "d"),
            FieldErrors {
                port: port_err.clone(),
                ..Default::default()
            },
        ),
        (
            pg("", "x", "", ""),
            FieldErrors {
                host: Some("Host can't be empty".into()),
                port: port_err.clone(),
                user: Some("User can't be empty".into()),
                database: Some("Database can't be empty".into()),
                path: None,
            },
        ),
        (
            Form {
                kind: DbKind::Sqlite,
                ..Form::new(&[])
            },
            FieldErrors {
                path: Some("Path can't be empty".into()),
                ..Default::default()
            },
        ),
        (
            Form {
                kind: DbKind::Sqlite,
                path: "  ".into(),
                ..Form::new(&[])
            },
            FieldErrors {
                path: Some("Path can't be empty".into()),
                ..Default::default()
            },
        ),
    ];
    for (form, want) in cases {
        assert_eq!(form.params(), Err(want), "{form:?}");
    }
}

#[test]
fn field_validation_ok_cases() {
    let postgres = |host: &str, port: u16| ConnectionParams::Postgres {
        host: host.into(),
        port,
        user: "u".into(),
        password: None,
        database: "d".into(),
        ssl: SslMode::Prefer,
    };
    let cases = [
        (
            pg(" db.local ", "5432", " u ", " d "),
            postgres("db.local", 5432),
        ),
        (pg("h", " 5432 ", "u", "d"), postgres("h", 5432)),
        (pg("h", "1", "u", "d"), postgres("h", 1)),
        (pg("h", "65535", "u", "d"), postgres("h", 65535)),
        (
            Form {
                kind: DbKind::Sqlite,
                path: " /nope/x.db ".into(),
                host: "".into(),
                port: "bad".into(),
                ..Form::new(&[])
            },
            ConnectionParams::Sqlite {
                path: PathBuf::from("/nope/x.db"),
            },
        ),
    ];
    for (form, want) in cases {
        assert_eq!(form.params(), Ok(want), "{form:?}");
    }
    let ssl = Form {
        ssl: SslMode::Require,
        ..ok_pg()
    };
    assert!(matches!(
        ssl.params(),
        Ok(ConnectionParams::Postgres {
            ssl: SslMode::Require,
            ..
        })
    ));
}

#[test]
fn the_password_reaches_the_params_as_a_secret_untrimmed_and_none_when_empty() {
    let params = |password: &str, kind| {
        Form {
            password: password.into(),
            kind,
            path: "/x.db".into(),
            ..ok_pg()
        }
        .params()
    };
    let cases = [
        ("", None),
        ("hunter2", Some("hunter2")),
        (" pad ", Some(" pad ")),
        ("p@ss w0rd/\\", Some("p@ss w0rd/\\")),
    ];
    for (typed, want) in cases {
        let Ok(ConnectionParams::Postgres { password, .. }) = params(typed, DbKind::Postgres)
        else {
            panic!("{typed:?}: valid")
        };
        assert_eq!(password.as_ref().map(Secret::expose), want, "{typed:?}");
    }
    assert_eq!(
        params("hunter2", DbKind::Sqlite),
        Ok(ConnectionParams::Sqlite {
            path: PathBuf::from("/x.db")
        }),
        "SQLite ignores it"
    );
}

#[test]
fn button_state_cases() {
    let cases = [
        (false, true, (false, true)),
        (true, false, (false, false)),
        (true, true, (true, true)),
        (false, false, (false, false)),
    ];
    for (name_valid, fields_valid, want) in cases {
        assert_eq!(button_state(name_valid, fields_valid), want);
    }
}

#[test]
fn default_color_cases() {
    let cases: [(&[u8], u8); 8] = [
        (&[], 0),
        (&[1], 0),
        (&[0, 1], 2),
        (&[0, 2], 1),
        (&[0, 1, 2, 3, 4, 5, 6], 7),
        (&[0, 1, 2, 3, 4, 5, 6, 7], 0),
        (&[7, 6, 5, 4, 3, 2, 1], 0),
        (&[0, 0, 1, 1], 2),
    ];
    for (used, want) in cases {
        assert_eq!(default_color(used), want, "{used:?}");
    }
}

#[test]
fn new_form_defaults() {
    let form = Form::new(&[0, 1]);
    assert_eq!(
        form,
        Form {
            name: "".into(),
            color: 2,
            is_production: false,
            kind: DbKind::Postgres,
            host: "".into(),
            port: "5432".into(),
            user: "".into(),
            password: "".into(),
            database: "postgres".into(),
            ssl: SslMode::Prefer,
            path: "".into(),
        }
    );
}

#[test]
fn delete_enabled_cases() {
    let cases = [
        ("local.db", false, "", true),
        ("local.db", false, "anything", true),
        ("prod-pg", true, "", false),
        ("prod-pg", true, "prod-p", false),
        ("prod-pg", true, "Prod-pg", false),
        ("prod-pg", true, " prod-pg", false),
        ("prod-pg", true, "prod-pg ", false),
        ("prod-pg", true, "prod-pgx", false),
        ("prod-pg", true, "prod-pg", true),
    ];
    for (name, prod, typed, want) in cases {
        assert_eq!(
            delete_enabled(name, prod, typed),
            want,
            "{name} {prod} {typed:?}"
        );
    }
}
