use super::*;
use crate::ddl::{TableEdit, alter_table};
use crate::model::{ForeignKey, Index, Role, TableDef};

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn an_atomic_execute_that_fails_on_its_second_statement_leaves_no_trace_of_the_first() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    let session = session(&pg);
    let run = Postgres::default().execute(
        &session,
        strings(&[&format!("CREATE TABLE {s}.kept (a int)"), "SELECT 1/0"]),
        true,
    );
    assert_eq!(block_on(run), Err("division by zero".to_string()));
    let left = format!("SELECT count(*) FROM pg_tables WHERE schemaname = '{s}'");
    assert_eq!(admin(&pg, &left), ["0"]);

    let run = Postgres::default().execute(
        &session,
        strings(&[
            &format!("CREATE TABLE {s}.kept (a int)"),
            &format!("CREATE TABLE {s}.more (a int)"),
        ]),
        true,
    );
    assert_eq!(block_on(run), Ok(()));
    assert_eq!(
        admin(&pg, &left),
        ["2"],
        "the session is usable after the rollback"
    );
}

#[test]
fn a_non_atomic_execute_creates_a_database_and_stops_at_the_first_error() {
    let Some(pg) = test_pg() else { return };
    let name = super::super::testing::unique("qx_database");
    let session = session(&pg);
    let exists = || {
        admin(
            &pg,
            &format!("SELECT count(*) FROM pg_database WHERE datname = '{name}'"),
        )
    };
    let run = Postgres::default().execute(
        &session,
        strings(&[&format!("CREATE DATABASE {name}")]),
        false,
    );
    assert_eq!(block_on(run), Ok(()));
    assert_eq!(exists(), ["1"]);

    let run = Postgres::default().execute(
        &session,
        strings(&["SELECT 1/0", &format!("DROP DATABASE {name}")]),
        false,
    );
    assert_eq!(block_on(run), Err("division by zero".to_string()));
    assert_eq!(exists(), ["1"], "nothing runs after the failure");

    let run = Postgres::default().execute(
        &session,
        strings(&[&format!("DROP DATABASE {name}")]),
        false,
    );
    assert_eq!(block_on(run), Ok(()));
    assert_eq!(exists(), ["0"]);
}

#[test]
fn roles_and_owners_name_what_the_scratch_setup_created() {
    let Some(pg) = test_pg() else { return };
    let owner = Scratch::new(&pg, "ROLE", "NOLOGIN");
    let login = Scratch::new(&pg, "ROLE", "LOGIN");
    let schema = Scratch::new(&pg, "SCHEMA", &format!("AUTHORIZATION {}", owner.name));
    let session = session(&pg);

    let roles = block_on(Postgres::default().roles(&session)).expect("roles");
    let mut sorted = roles.clone();
    sorted.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    assert_eq!(roles, sorted, "sorted by name, bytewise");
    assert!(roles.contains(&Role {
        name: owner.name.clone(),
        can_login: false
    }));
    assert!(roles.contains(&Role {
        name: login.name.clone(),
        can_login: true
    }));

    let want = admin(
        &pg,
        "SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = current_database()",
    );
    assert_eq!(
        block_on(Postgres::default().database_owner(&session)),
        Ok(want[0].clone())
    );
    assert_eq!(
        block_on(Postgres::default().schema_owner(&session, &schema.name)),
        Ok(owner.name.clone())
    );
    assert!(block_on(Postgres::default().schema_owner(&session, "qx_no_such_schema")).is_err());
}

#[test]
fn table_def_has_columns_plain_indexes_and_foreign_keys_in_key_order() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let other = Scratch::new(&pg, "SCHEMA", "");
    let (s, o) = (&schema.name, &other.name);
    admin(&pg, &format!(
        "CREATE TABLE {o}.parent (a int, b int, PRIMARY KEY (a, b));
         CREATE TABLE {s}.solo (id int PRIMARY KEY);
         CREATE TABLE {s}.t (id int PRIMARY KEY, code text UNIQUE, x int, y int, solo_id int,
             CONSTRAINT t_xy_fkey FOREIGN KEY (y, x) REFERENCES {o}.parent (b, a) ON DELETE CASCADE ON UPDATE SET NULL,
             CONSTRAINT t_solo_fkey FOREIGN KEY (solo_id) REFERENCES {s}.solo ON DELETE RESTRICT);
         CREATE INDEX t_y_x_idx ON {s}.t (y, x);
         CREATE UNIQUE INDEX t_solo_idx ON {s}.t (solo_id);"
    ));
    let session = session(&pg);
    let def = block_on(Postgres::default().table_def(&session, s, "t")).expect("table_def");
    let columns = block_on(Postgres::default().columns(&session, s, "t")).expect("columns");
    assert_eq!(
        def,
        TableDef {
            name: "t".into(),
            columns,
            indexes: vec![
                Index {
                    name: "t_solo_idx".into(),
                    columns: strings(&["solo_id"]),
                    unique: true
                },
                Index {
                    name: "t_y_x_idx".into(),
                    columns: strings(&["y", "x"]),
                    unique: false
                },
            ],
            foreign_keys: vec![
                ForeignKey {
                    name: "t_solo_fkey".into(),
                    columns: strings(&["solo_id"]),
                    ref_schema: s.clone(),
                    ref_table: "solo".into(),
                    ref_columns: strings(&["id"]),
                    on_delete: "RESTRICT".into(),
                    on_update: "NO ACTION".into(),
                },
                ForeignKey {
                    name: "t_xy_fkey".into(),
                    columns: strings(&["y", "x"]),
                    ref_schema: o.clone(),
                    ref_table: "parent".into(),
                    ref_columns: strings(&["b", "a"]),
                    on_delete: "CASCADE".into(),
                    on_update: "SET NULL".into(),
                },
            ],
            primary_key: Some("t_pkey".into()),
            unique_keys: vec![Index {
                name: "t_code_key".into(),
                columns: strings(&["code"]),
                unique: true
            }],
        }
    );
    assert_eq!(def.columns.len(), 5);
}

#[test]
fn table_def_has_the_pk_name_unique_keys_and_identity() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    admin(
        &pg,
        &format!(
            "CREATE TABLE {s}.t (id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, \
             email text UNIQUE, a int, b int, n serial, CONSTRAINT t_ab_key UNIQUE (b, a))"
        ),
    );
    let session = session(&pg);
    let def = block_on(Postgres::default().table_def(&session, s, "t")).expect("table_def");
    assert_eq!(def.primary_key, Some("t_pkey".into()));
    assert_eq!(
        def.unique_keys,
        vec![
            Index {
                name: "t_ab_key".into(),
                columns: strings(&["b", "a"]),
                unique: true
            },
            Index {
                name: "t_email_key".into(),
                columns: strings(&["email"]),
                unique: true
            },
        ]
    );
    assert_eq!(def.indexes, vec![]);
    let identity: Vec<(&str, bool)> = def
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.identity))
        .collect();
    assert_eq!(
        identity,
        [
            ("id", true),
            ("email", false),
            ("a", false),
            ("b", false),
            ("n", false)
        ]
    );
    let columns = block_on(Postgres::default().columns(&session, s, "t")).expect("columns");
    assert_eq!(columns, def.columns);
}

#[test]
fn an_alter_table_that_moves_the_pk_adds_a_unique_key_and_identity_runs() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    admin(
        &pg,
        &format!("CREATE TABLE {s}.t (a int PRIMARY KEY, b int NOT NULL, c text)"),
    );
    let session = session(&pg);
    let before = block_on(Postgres::default().table_def(&session, s, "t")).expect("table_def");
    let mut after = TableEdit {
        origins: before
            .columns
            .iter()
            .map(|c| Some(c.name.clone()))
            .collect(),
        def: before.clone(),
    };
    after.def.columns[0].is_pk = false;
    after.def.columns[1].is_pk = true;
    after.def.columns[1].identity = true;
    after.def.unique_keys = vec![Index {
        name: "t_c_key".into(),
        columns: strings(&["c"]),
        unique: true,
    }];
    let run = Postgres::default().execute(&session, alter_table(s, &before, &after), true);
    assert_eq!(block_on(run), Ok(()));

    let def = block_on(Postgres::default().table_def(&session, s, "t")).expect("table_def");
    assert_eq!(def.primary_key, Some("t_pkey".into()));
    assert_eq!(def, after.def);
}
