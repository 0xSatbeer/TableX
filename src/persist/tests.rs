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
