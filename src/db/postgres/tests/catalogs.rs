use super::*;

#[test]
fn databases_are_sorted_without_templates_or_ones_that_refuse_connections() {
    let Some(pg) = test_pg() else { return };
    let (app, closed, template) = (
        Scratch::new(&pg, "DATABASE", ""),
        Scratch::new(&pg, "DATABASE", ""),
        Scratch::new(&pg, "DATABASE", ""),
    );
    admin(
        &pg,
        &format!("ALTER DATABASE {} ALLOW_CONNECTIONS false", closed.name),
    );
    admin(
        &pg,
        &format!("ALTER DATABASE {} IS_TEMPLATE true", template.name),
    );
    let got = block_on(Postgres::default().databases(&session(&pg))).expect("databases");
    // Other tests create databases in parallel, so only these names are checked.
    // (database, listed)
    let cases = [
        ("postgres", true),
        (app.name.as_str(), true),
        ("template0", false),
        ("template1", false),
        (closed.name.as_str(), false),
        (template.name.as_str(), false),
    ];
    for (name, listed) in cases {
        assert_eq!(got.iter().any(|d| d == name), listed, "{name} in {got:?}");
    }
    let mut sorted = got.clone();
    sorted.sort();
    assert_eq!(got, sorted, "sorted by name");
}

#[test]
fn schemas_are_sorted_without_system_or_pg_prefixed_ones() {
    let Some(pg) = test_pg() else { return };
    let (full, empty) = (
        Scratch::new(&pg, "DATABASE", ""),
        Scratch::new(&pg, "DATABASE", ""),
    );
    let (full_db, empty_db) = (on(&pg, &full.name), on(&pg, &empty.name));
    admin(&full_db, "CREATE SCHEMA app");
    // The pg_ prefix is reserved, so only this setting lets a user create pg_custom.
    admin(
        &full_db,
        "SET allow_system_table_mods = on; CREATE SCHEMA pg_custom",
    );
    // Leaves pg_temp_N and pg_toast_temp_N behind.
    admin(&full_db, "CREATE TEMP TABLE scratch (a text)");
    admin(&empty_db, "DROP SCHEMA public");
    let all = admin(
        &full_db,
        "SELECT nspname FROM pg_namespace ORDER BY nspname",
    );
    for hidden in [
        "pg_catalog",
        "information_schema",
        "pg_toast",
        "pg_custom",
        "pg_temp_",
        "pg_toast_temp_",
    ] {
        assert!(
            all.iter().any(|s| s.starts_with(hidden)),
            "the server has {hidden}: {all:?}"
        );
    }
    // (database, schemas)
    let cases = [(&full_db, vec!["app", "public"]), (&empty_db, vec![])];
    for (db, want) in cases {
        assert_eq!(
            block_on(Postgres::default().schemas(&session(db))),
            Ok(want.iter().map(|s| s.to_string()).collect()),
            "{db:?}"
        );
    }
}

#[test]
fn schemas_of_a_database_without_connect_privilege_fail_with_the_reason() {
    let Some(pg) = test_pg() else { return };
    let (db, role) = (
        Scratch::new(&pg, "DATABASE", ""),
        Scratch::new(&pg, "ROLE", "LOGIN PASSWORD 'pw'"),
    );
    admin(
        &pg,
        &format!("REVOKE CONNECT ON DATABASE {} FROM PUBLIC", db.name),
    );
    let schemas = |database: &str| {
        let driver = Postgres::default();
        let session =
            block_on(driver.connect(&on(&as_user(&pg, &role.name, Some("pw")), database)))?;
        block_on(driver.schemas(&session))
    };
    // Expanding a database opens a session to it, so the missing privilege fails that connect.
    assert_eq!(
        schemas(&db.name),
        Err(format!("permission denied for database \"{}\"", db.name))
    );
    assert_eq!(
        schemas("postgres"),
        Ok(vec!["public".to_string()]),
        "a database the role may connect to still lists"
    );
}
