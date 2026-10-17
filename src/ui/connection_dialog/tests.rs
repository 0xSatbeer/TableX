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
            save_password: true,
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

/// A window holding the dialog for `editing` on `connections`, with `keychain` installed.
fn open_dialog(
    cx: &mut gpui_kit::TestAppContext,
    connections: Vec<Connection>,
    editing: Option<ConnectionId>,
    keychain: Arc<dyn crate::persist::Keychain>,
) -> (
    gpui_kit::VisualTestContext,
    Entity<ConnectionDialog>,
    Entity<Workspace>,
) {
    cx.update(gpui_kit::init);
    cx.update(|cx| cx.set_global(crate::persist::Keys::new(keychain)));
    let workspace = cx.update(|cx| {
        cx.new(|_| Workspace {
            id_floor: 0,
            id: 1,
            name: "w".into(),
            connections,
        })
    });
    let driver: Arc<dyn Driver> = Arc::new(crate::db::fake::Fake::default());
    let ws = workspace.clone();
    let (window, dialog) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            cx.new(|cx| ConnectionDialog::new(ws, driver, editing, window, cx))
        })
        .expect("open test window")
    });
    let cx = gpui_kit::VisualTestContext::from_window(window, cx);
    (cx, dialog, workspace)
}

#[gpui_kit::test]
fn save_keeps_the_password_in_the_keychain_when_checked_and_removes_it_when_not(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::persist::{Keychain as _, MemKeychain};
    let saved = Connection {
        id: ConnectionId(1),
        name: "pg".into(),
        color: 0,
        is_production: false,
        status: ConnectionStatus::Disconnected,
        params: ok_pg().params().unwrap(),
        save_password: true,
        databases: None,
    };
    // (editing, box checked, password typed or None to keep the prefill, entry after)
    let cases = [
        (true, true, None, Some("old")),
        (true, true, Some("new"), Some("new")),
        (true, false, None, None),
        (true, true, Some(""), None),
        (false, true, Some("pw"), Some("pw")),
        (false, false, Some("pw"), None),
    ];
    for (editing, checked, typed, want) in cases {
        let label = format!("editing {editing} checked {checked} typed {typed:?}");
        let (id, keychain) = (ConnectionId(1), Arc::new(MemKeychain::default()));
        let (connections, editing) = match editing {
            true => (vec![saved.clone()], Some(id)),
            false => (vec![], None),
        };
        if editing.is_some() {
            keychain.set(id, &Secret::new("old")).unwrap();
        }
        let (mut cx, dialog, workspace) =
            open_dialog(cx, connections, editing, keychain.clone() as _);
        if editing.is_some() {
            let prefill = dialog.read_with(&cx, |d, cx| d.form(cx).password);
            assert_eq!(prefill, "old", "{label}: prefilled from the keychain");
        }
        dialog.update_in(&mut cx, |d, window, cx| {
            let fields = [(&d.name, "pg"), (&d.host, "h"), (&d.user, "u")];
            for (input, text) in fields.into_iter().chain(typed.map(|t| (&d.password, t))) {
                input.update(cx, |i, cx| i.set_value(text, window, cx));
            }
            d.save_password = checked;
            assert!(d.save(cx), "{label}: saved");
        });
        let entry = keychain.get(id).unwrap();
        assert_eq!(entry.as_ref().map(Secret::expose), want, "{label}");
        let flag = workspace.read_with(&cx, |ws, _| ws.connections[0].save_password);
        assert_eq!(flag, checked, "{label}: save_password");
    }
}

/// Holds entries but can't read them, like a Secret Service whose unlock was dismissed.
#[derive(Default)]
struct Locked(crate::persist::MemKeychain);

impl crate::persist::Keychain for Locked {
    fn get(&self, _: ConnectionId) -> Result<Option<Secret>, String> {
        Err("locked".into())
    }
    fn set(&self, id: ConnectionId, password: &Secret) -> Result<(), String> {
        self.0.set(id, password)
    }
    fn delete(&self, id: ConnectionId) -> Result<(), String> {
        self.0.delete(id)
    }
}

#[gpui_kit::test]
fn a_saved_password_the_keychain_could_not_read_on_open_survives_save(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::persist::Keychain as _;
    let saved = Connection {
        id: ConnectionId(1),
        name: "pg".into(),
        color: 0,
        is_production: false,
        status: ConnectionStatus::Disconnected,
        params: ok_pg().params().unwrap(),
        save_password: true,
        databases: None,
    };
    // (box checked, password typed, entry after)
    let cases = [
        (true, None, Some("old")),
        (true, Some("new"), Some("new")),
        (false, None, None),
    ];
    for (checked, typed, want) in cases {
        let label = format!("checked {checked} typed {typed:?}");
        let keychain = Arc::new(Locked::default());
        keychain
            .0
            .set(ConnectionId(1), &Secret::new("old"))
            .unwrap();
        let editing = Some(ConnectionId(1));
        let (mut cx, dialog, _) = open_dialog(cx, vec![saved.clone()], editing, keychain.clone());
        dialog.update_in(&mut cx, |d, window, cx| {
            if let Some(text) = typed {
                d.password.update(cx, |i, cx| i.set_value(text, window, cx));
            }
            d.name
                .update(cx, |i, cx| i.set_value("renamed", window, cx));
            d.save_password = checked;
            assert!(d.save(cx), "{label}: saved");
        });
        let entry = keychain.0.get(ConnectionId(1)).unwrap();
        assert_eq!(entry.as_ref().map(Secret::expose), want, "{label}");
    }
}
