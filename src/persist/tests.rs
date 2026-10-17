use super::*;
use crate::model::{
    Connection, ConnectionId, ConnectionParams, ConnectionStatus, Secret, SslMode, Workspace,
    Workspaces,
};

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("queryx-persist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn conn(id: u32, name: &str, color: u8, production: bool, params: ConnectionParams) -> Connection {
    Connection {
        id: ConnectionId(id),
        name: name.into(),
        color,
        is_production: production,
        status: ConnectionStatus::Disconnected,
        params,
        save_password: false,
        databases: None,
    }
}

fn two_workspaces() -> Workspaces {
    let pg = Connection {
        save_password: true,
        ..conn(
            1,
            "prod-pg",
            3,
            true,
            ConnectionParams::Postgres {
                host: "db.acme.io".into(),
                port: 5432,
                user: "app".into(),
                password: Some(Secret::new("hunter2")),
                database: "app_db".into(),
                ssl: SslMode::Require,
            },
        )
    };
    let sqlite = conn(
        2,
        "local.db",
        5,
        false,
        ConnectionParams::Sqlite {
            path: "/home/me/local.db".into(),
        },
    );
    Workspaces {
        active: 1,
        list: vec![
            Workspace {
                id: 1,
                name: "Acme".into(),
                connections: vec![pg, sqlite],
                id_floor: 0,
            },
            Workspace {
                id: 2,
                name: "Empty".into(),
                connections: Vec::new(),
                id_floor: 0,
            },
        ],
    }
}

const C6: &str = r#"version = 1
active = 1

[[workspace]]
id = 1
name = "Acme"

[[workspace.connection]]
id = 1
name = "prod-pg"
color = 3
production = true
kind = "postgres"
host = "db.acme.io"
port = 5432
user = "app"
database = "app_db"
ssl = "require"
save_password = true

[[workspace.connection]]
id = 2
name = "local.db"
color = 5
production = false
kind = "sqlite"
path = "/home/me/local.db"

[[workspace]]
id = 2
name = "Empty"
"#;

#[test]
fn two_workspaces_round_trip_in_the_c6_shape_without_passwords() {
    let dir = test_dir("round-trip");
    let (mut file, notice) = WorkspacesFile::load(&dir);
    assert_eq!(notice, None);
    let all = two_workspaces();
    file.all = all.clone();
    file.save(&all.list[0]).unwrap();
    let text = std::fs::read_to_string(dir.join("workspaces.toml")).unwrap();
    assert_eq!(text, C6);
    assert!(!text.contains("hunter2"), "a password was written");
    let (back, notice) = WorkspacesFile::load(&dir);
    assert_eq!(notice, None);
    let mut want = all;
    // The password stays in memory only; the read back side has the ids shared.
    if let ConnectionParams::Postgres { password, .. } = &mut want.list[0].connections[0].params {
        *password = None;
    }
    want.share_ids();
    assert_eq!(back.all, want);
}

#[test]
fn a_corrupt_file_moves_to_bak_and_loads_empty_with_a_notice() {
    let dir = test_dir("corrupt");
    std::fs::write(dir.join("workspaces.toml"), "version = [").unwrap();
    let (mut file, notice) = WorkspacesFile::load(&dir);
    assert_eq!(
        notice.as_deref(),
        Some(
            "workspaces.toml could not be read; it was saved as workspaces.toml.bak and QueryX started empty."
        )
    );
    assert!(file.all.list.is_empty());
    assert_eq!(file.active().connections, Vec::new());
    assert!(!dir.join("workspaces.toml").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("workspaces.toml.bak")).unwrap(),
        "version = ["
    );
}

#[test]
fn writes_go_through_tmp_and_rename() {
    let dir = test_dir("atomic");
    let path = dir.join("sub/workspaces.toml");
    write_atomic(&path, b"one").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
    assert!(
        !dir.join("sub/workspaces.toml.tmp").exists(),
        "tmp left behind"
    );
    // The tmp write fails (a dir is in its way), so the old file stays whole.
    std::fs::create_dir(dir.join("sub/workspaces.toml.tmp")).unwrap();
    assert!(write_atomic(&path, b"two").is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
}

#[test]
fn saving_unchanged_workspaces_writes_nothing() {
    let dir = test_dir("unchanged");
    let (mut file, _) = WorkspacesFile::load(&dir);
    let active = file.active();
    file.save(&active).unwrap();
    std::fs::remove_file(dir.join("workspaces.toml")).unwrap();
    file.save(&active).unwrap();
    assert!(
        !dir.join("workspaces.toml").exists(),
        "rewrote an unchanged file"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn config_dir_follows_xdg_then_home() {
    // (XDG_CONFIG_HOME, HOME, expected dir)
    let cases: [(Option<&str>, Option<&str>, Option<&str>); 7] = [
        (Some("/x"), Some("/h"), Some("/x/queryx")),
        (None, Some("/h"), Some("/h/.config/queryx")),
        (Some(""), Some("/h"), Some("/h/.config/queryx")), // empty counts as unset (XDG spec)
        (Some("rel"), Some("/h"), Some("/h/.config/queryx")), // relative is ignored (XDG spec)
        (Some("/x"), None, Some("/x/queryx")),
        (None, None, None),
        (None, Some(""), None),
    ];
    for (xdg, home, want) in cases {
        let env = |k: &str| match k {
            "XDG_CONFIG_HOME" => xdg.map(Into::into),
            "HOME" => home.map(Into::into),
            _ => None,
        };
        let got = ConfigDir::resolve(env).map(|d| d.root);
        assert_eq!(
            got,
            want.map(PathBuf::from),
            "XDG_CONFIG_HOME={xdg:?} HOME={home:?}"
        );
    }
}

#[test]
fn the_mem_keychain_keeps_one_password_per_connection_account() {
    let keychain = MemKeychain::default();
    keychain.set(ConnectionId(3), &Secret::new("pw")).unwrap();
    keychain
        .set(ConnectionId(4), &Secret::new("other"))
        .unwrap();
    keychain.delete(ConnectionId(4)).unwrap();
    keychain.delete(ConnectionId(9)).unwrap();
    let accounts: Vec<String> = keychain.0.lock().unwrap().keys().cloned().collect();
    assert_eq!(accounts, ["connection-3"]);
    assert_eq!(keychain.get(ConnectionId(3)), Ok(Some(Secret::new("pw"))));
    assert_eq!(keychain.get(ConnectionId(4)), Ok(None));
}

#[test]
fn each_theme_choice_a_scale_and_a_font_size_round_trip_through_config_toml() {
    use crate::theme::ThemeChoice;
    let dir = test_dir("config");
    assert_eq!(load_config(&dir), (Config::default(), None), "absent file");
    for theme in [
        ThemeChoice::Dark,
        ThemeChoice::Light,
        ThemeChoice::System,
        ThemeChoice::User("Nord".into()),
    ] {
        let config = Config {
            theme,
            ui_scale: 120,
            console_font_px: 14.5,
        };
        save_config(&dir, &config).unwrap();
        assert_eq!(load_config(&dir), (config, None));
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("config.toml")).unwrap(),
        "version = 1\ntheme = \"user:Nord\"\nui_scale = 120\nconsole_font_px = 14.5\n"
    );
}

#[test]
fn invalid_config_values_fall_back_to_defaults_and_a_corrupt_file_moves_to_bak() {
    let dir = test_dir("config-bad");
    let path = dir.join("config.toml");
    let default = Config::default();
    // (file text, expected config)
    let cases = [
        (
            "version = 1\ntheme = \"neon\"\nui_scale = 100\nconsole_font_px = 13\n",
            Config {
                console_font_px: 13.,
                ..default.clone()
            },
        ),
        (
            "version = 1\ntheme = \"light\"\nui_scale = 105\n",
            Config {
                theme: crate::theme::ThemeChoice::Light,
                ..default.clone()
            },
        ),
        ("version = 1\nconsole_font_px = 99\n", default.clone()),
        (
            "version = 1\ntheme = \"user:\"\nconsole_font_px = 0\n",
            default.clone(),
        ),
        ("", default.clone()),
    ];
    for (text, want) in cases {
        std::fs::write(&path, text).unwrap();
        assert_eq!(load_config(&dir), (want, None), "{text:?}");
    }
    std::fs::write(&path, "theme = [").unwrap();
    let (config, notice) = load_config(&dir);
    assert_eq!(config, default);
    assert!(notice.unwrap().starts_with("config.toml could not be read"));
    assert_eq!(
        std::fs::read_to_string(dir.join("config.toml.bak")).unwrap(),
        "theme = ["
    );
}

#[test]
fn the_next_console_id_counts_sql_files_and_sessions() {
    let dir = test_dir("console-ids");
    let consoles = dir.join("consoles");
    std::fs::create_dir_all(&consoles).unwrap();
    for name in ["3.sql", "9.sql", "x.sql", "12.txt"] {
        std::fs::write(consoles.join(name), "").unwrap();
    }
    let session = SessionFile {
        tabs: vec![SessionTab::Console {
            connection: 1,
            database: "app_db".into(),
            schema: String::new(),
            console: 5,
            autocommit: true,
            pinned: false,
        }],
        ..SessionFile::default()
    };
    save_session(&dir, 1, &session).unwrap();
    assert_eq!(max_console_id(&dir), 9);
}

fn log_entry(sql: &str) -> crate::model::QueryLogEntry {
    crate::model::QueryLogEntry {
        at: std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_760_436_121_123),
        connection_id: ConnectionId(1),
        database: "app_db".into(),
        sql: sql.into(),
        duration: std::time::Duration::from_millis(12),
        rows: Some(1),
        status: crate::model::QueryStatus::Ok,
        source: crate::model::QuerySource::Console,
    }
}

#[test]
fn a_query_log_entry_is_one_c9_line() {
    use crate::model::{QuerySource, QueryStatus};
    assert_eq!(
        query_log_line(&log_entry("SELECT 1")),
        r#"{"at_ms":1760436121123,"connection":1,"database":"app_db","sql":"SELECT 1","ms":12,"rows":1,"status":"ok","error":null,"source":"console"}"#
    );
    let failed = crate::model::QueryLogEntry {
        rows: None,
        status: QueryStatus::Err("boom".into()),
        source: QuerySource::TableView,
        ..log_entry("x")
    };
    let line = query_log_line(&failed);
    assert!(line.contains(r#""rows":null,"status":"error","error":"boom","source":"table""#));
    let dir = test_dir("query-log-roundtrip");
    append_query_log(&dir, &log_entry("SELECT 1")).unwrap();
    append_query_log(&dir, &failed).unwrap();
    let cancelled = crate::model::QueryLogEntry {
        status: QueryStatus::Cancelled,
        source: QuerySource::Introspection,
        ..log_entry("y")
    };
    append_query_log(&dir, &cancelled).unwrap();
    assert_eq!(
        load_query_log(&dir),
        [log_entry("SELECT 1"), failed, cancelled]
    );
}

#[test]
fn the_query_log_keeps_the_newest_5000_lines() {
    let dir = test_dir("query-log-trim");
    let lines: Vec<String> = (0..5200)
        .map(|i| query_log_line(&log_entry(&format!("q{i}"))))
        .collect();
    write_atomic(&query_log_path(&dir), (lines.join("\n") + "\n").as_bytes()).unwrap();
    let log = load_query_log(&dir);
    assert_eq!(log.len(), 5000);
    assert_eq!(log[0].sql, "q200");
    assert_eq!(log[4999].sql, "q5199");
    let on_disk = std::fs::read_to_string(query_log_path(&dir)).unwrap();
    assert_eq!(on_disk.lines().count(), 5000, "the file is trimmed too");
    assert_eq!(load_query_log(&dir), log);
}

#[test]
fn malformed_query_log_lines_are_skipped() {
    let dir = test_dir("query-log-malformed");
    let text = [
        query_log_line(&log_entry("a")),
        "{not json".into(),
        r#"{"at_ms":1,"status":"weird"}"#.into(),
        String::new(),
        query_log_line(&log_entry("b")),
    ]
    .join("\n");
    write_atomic(&query_log_path(&dir), text.as_bytes()).unwrap();
    let sql: Vec<_> = load_query_log(&dir).into_iter().map(|e| e.sql).collect();
    assert_eq!(sql, ["a", "b"]);
    assert!(load_query_log(&test_dir("query-log-missing")).is_empty());
}
