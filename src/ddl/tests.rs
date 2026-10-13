use super::*;
use crate::model::{Column, ForeignKey, Index, TableDef};

fn col(name: &str, data_type: &str, nullable: bool, default: Option<&str>) -> Column {
    Column {
        name: name.into(),
        data_type: data_type.into(),
        nullable,
        default: default.map(Into::into),
        is_pk: false,
        enum_values: vec![],
        identity: false,
    }
}

fn index(name: &str, columns: &[&str], unique: bool) -> Index {
    Index {
        name: name.into(),
        columns: columns.iter().map(|c| c.to_string()).collect(),
        unique,
    }
}

fn fk(name: &str, column: &str, on_delete: &str) -> ForeignKey {
    ForeignKey {
        name: name.into(),
        columns: vec![column.into()],
        ref_schema: "public".into(),
        ref_table: "users".into(),
        ref_columns: vec!["id".into()],
        on_delete: on_delete.into(),
        on_update: "NO ACTION".into(),
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn quoting_cases() {
    let cases = [
        ("plain", "\"plain\"", "'plain'"),
        ("a\"b", "\"a\"\"b\"", "'a\"b'"),
        ("it's", "\"it's\"", "'it''s'"),
        ("", "\"\"", "''"),
    ];
    for (raw, ident_q, lit_q) in cases {
        assert_eq!(ident(raw), ident_q, "{raw}");
        assert_eq!(literal(raw), lit_q, "{raw}");
    }
    assert_eq!(
        create_role("o\"k", Some("p'w"), &[]),
        s(&["CREATE ROLE \"o\"\"k\" LOGIN PASSWORD 'p''w'"])
    );
}

#[test]
fn database_and_schema_cases() {
    // (before name, after name, before owner, after owner, database statements)
    let cases = [
        ("a", "a", "r", "r", vec![]),
        (
            "a",
            "b",
            "r",
            "r",
            vec!["ALTER DATABASE \"a\" RENAME TO \"b\""],
        ),
        (
            "a",
            "a",
            "r",
            "q",
            vec!["ALTER DATABASE \"a\" OWNER TO \"q\""],
        ),
        (
            "a",
            "b",
            "r",
            "q",
            vec![
                "ALTER DATABASE \"a\" RENAME TO \"b\"",
                "ALTER DATABASE \"b\" OWNER TO \"q\"",
            ],
        ),
    ];
    for (bn, an, bo, ao, want) in cases {
        assert_eq!(
            alter_database(bn, an, bo, ao),
            s(&want),
            "{bn}->{an} {bo}->{ao}"
        );
        let schema: Vec<String> = want
            .iter()
            .map(|w| w.replace("DATABASE", "SCHEMA"))
            .collect();
        assert_eq!(
            alter_schema(bn, an, bo, ao),
            schema,
            "{bn}->{an} {bo}->{ao}"
        );
    }
}

#[test]
fn create_database_and_role_cases() {
    assert_eq!(
        create_database("x", "r"),
        s(&["CREATE DATABASE \"x\" OWNER \"r\""])
    );
    // (name, password, memberships, statements)
    let cases = [
        (
            "u",
            Some("p"),
            vec![],
            vec!["CREATE ROLE \"u\" LOGIN PASSWORD 'p'"],
        ),
        ("g", None, vec![], vec!["CREATE ROLE \"g\" NOLOGIN"]),
        (
            "u",
            Some("p"),
            vec!["b", "a"],
            vec![
                "CREATE ROLE \"u\" LOGIN PASSWORD 'p'",
                "GRANT \"b\" TO \"u\"",
                "GRANT \"a\" TO \"u\"",
            ],
        ),
        (
            "g",
            None,
            vec!["a"],
            vec!["CREATE ROLE \"g\" NOLOGIN", "GRANT \"a\" TO \"g\""],
        ),
    ];
    for (name, password, member_of, want) in cases {
        assert_eq!(
            create_role(name, password, &s(&member_of)),
            s(&want),
            "{name}"
        );
    }
}

fn orders() -> TableDef {
    TableDef {
        name: "orders".into(),
        columns: vec![
            col("id", "int", false, None),
            col("gone", "text", true, None),
            col("old", "text", true, None),
            col("qty", "int", true, None),
        ],
        indexes: vec![
            index("keep_ix", &["id"], false),
            index("edit_ix", &["qty"], false),
            index("drop_ix", &["old"], false),
        ],
        foreign_keys: vec![
            fk("keep_fk", "id", "NO ACTION"),
            fk("edit_fk", "qty", "NO ACTION"),
            fk("drop_fk", "old", "NO ACTION"),
        ],
        primary_key: None,
        unique_keys: vec![],
    }
}

fn unchanged(def: &TableDef) -> TableEdit {
    TableEdit {
        origins: def.columns.iter().map(|c| Some(c.name.clone())).collect(),
        def: def.clone(),
    }
}

#[test]
fn alter_table_without_changes_is_empty() {
    assert_eq!(
        alter_table("public", &orders(), &unchanged(&orders())),
        Vec::<String>::new()
    );
}

#[test]
fn alter_table_emits_every_kind_in_order() {
    let after = TableEdit {
        def: TableDef {
            name: "purchases".into(),
            columns: vec![
                col("id", "int", false, None),
                col("new", "text", true, None),
                col("qty", "bigint", false, Some("0")),
                col("note", "text", false, Some("'x'")),
            ],
            indexes: vec![
                index("keep_ix", &["id"], false),
                index("edit_ix", &["qty"], true),
                index("add_ix", &["new", "qty"], false),
            ],
            foreign_keys: vec![
                fk("keep_fk", "id", "NO ACTION"),
                fk("edit_fk", "qty", "CASCADE"),
                fk("add_fk", "new", "SET NULL"),
            ],
            primary_key: None,
            unique_keys: vec![],
        },
        origins: vec![
            Some("id".into()),
            Some("old".into()),
            Some("qty".into()),
            None,
        ],
    };
    let t = "ALTER TABLE \"public\".\"purchases\"";
    let want = vec![
        "ALTER TABLE \"public\".\"orders\" RENAME TO \"purchases\"".to_string(),
        format!("{t} DROP CONSTRAINT \"edit_fk\""),
        format!("{t} DROP CONSTRAINT \"drop_fk\""),
        "DROP INDEX \"public\".\"edit_ix\"".into(),
        "DROP INDEX \"public\".\"drop_ix\"".into(),
        format!("{t} DROP COLUMN \"gone\""),
        format!("{t} RENAME COLUMN \"old\" TO \"new\""),
        format!("{t} ALTER COLUMN \"qty\" TYPE bigint"),
        format!("{t} ALTER COLUMN \"qty\" SET NOT NULL"),
        format!("{t} ALTER COLUMN \"qty\" SET DEFAULT 0"),
        format!("{t} ADD COLUMN \"note\" text NOT NULL DEFAULT 'x'"),
        "CREATE UNIQUE INDEX \"edit_ix\" ON \"public\".\"purchases\" (\"qty\")".into(),
        "CREATE INDEX \"add_ix\" ON \"public\".\"purchases\" (\"new\", \"qty\")".into(),
        format!(
            "{t} ADD CONSTRAINT \"edit_fk\" FOREIGN KEY (\"qty\") REFERENCES \"public\".\"users\" (\"id\") ON DELETE CASCADE ON UPDATE NO ACTION"
        ),
        format!(
            "{t} ADD CONSTRAINT \"add_fk\" FOREIGN KEY (\"new\") REFERENCES \"public\".\"users\" (\"id\") ON DELETE SET NULL ON UPDATE NO ACTION"
        ),
    ];
    assert_eq!(alter_table("public", &orders(), &after), want);
}

#[test]
fn alter_table_drops_null_and_default() {
    let mut before = orders();
    before.columns[3] = col("qty", "int", false, Some("1"));
    let mut after = unchanged(&before);
    after.def.columns[3] = col("qty", "int", true, None);
    let t = "ALTER TABLE \"public\".\"orders\"";
    assert_eq!(
        alter_table("public", &before, &after),
        vec![
            format!("{t} ALTER COLUMN \"qty\" DROP NOT NULL"),
            format!("{t} ALTER COLUMN \"qty\" DROP DEFAULT"),
        ]
    );
}

#[test]
fn create_table_with_index_and_fk() {
    let def = TableDef {
        name: "t".into(),
        columns: vec![
            col("id", "int", false, None),
            col("user_id", "int", true, Some("1")),
        ],
        indexes: vec![index("t_user_ix", &["user_id"], false)],
        foreign_keys: vec![fk("t_user_fk", "user_id", "CASCADE")],
        primary_key: None,
        unique_keys: vec![],
    };
    assert_eq!(
        create_table("s", &def),
        s(&[
            "CREATE TABLE \"s\".\"t\" (\"id\" int NOT NULL, \"user_id\" int DEFAULT 1)",
            "CREATE INDEX \"t_user_ix\" ON \"s\".\"t\" (\"user_id\")",
            "ALTER TABLE \"s\".\"t\" ADD CONSTRAINT \"t_user_fk\" FOREIGN KEY (\"user_id\") REFERENCES \"public\".\"users\" (\"id\") ON DELETE CASCADE ON UPDATE NO ACTION",
        ])
    );
}

fn pk(mut c: Column) -> Column {
    c.is_pk = true;
    c
}

fn identity(mut c: Column) -> Column {
    c.identity = true;
    c
}

fn keyed() -> TableDef {
    TableDef {
        name: "t".into(),
        columns: vec![
            pk(col("id", "int", false, None)),
            col("code", "text", false, None),
            identity(col("n", "int", false, None)),
        ],
        indexes: vec![index("drop_ix", &["code"], false)],
        foreign_keys: vec![fk("drop_fk", "n", "NO ACTION")],
        primary_key: Some("t_pkey".into()),
        unique_keys: vec![
            index("t_code_key", &["code"], true),
            index("t_n_key", &["n"], true),
        ],
    }
}

#[test]
fn alter_table_emits_key_and_identity_changes_in_order() {
    let after = TableEdit {
        def: TableDef {
            columns: vec![
                pk(identity(col("id", "int", false, None))),
                pk(col("code", "text", false, None)),
                col("n", "int", false, None),
                col("m", "int", true, None),
            ],
            indexes: vec![index("add_ix", &["m"], false)],
            foreign_keys: vec![fk("add_fk", "m", "NO ACTION")],
            unique_keys: vec![
                index("t_code_key", &["code", "n"], true),
                index("t_m_key", &["m"], true),
            ],
            ..keyed()
        },
        origins: vec![
            Some("id".into()),
            Some("code".into()),
            Some("n".into()),
            None,
        ],
    };
    let t = "ALTER TABLE \"public\".\"t\"";
    let want = vec![
        format!("{t} DROP CONSTRAINT \"drop_fk\""),
        format!("{t} DROP CONSTRAINT \"t_pkey\""),
        "DROP INDEX \"public\".\"drop_ix\"".into(),
        format!("{t} DROP CONSTRAINT \"t_code_key\""),
        format!("{t} DROP CONSTRAINT \"t_n_key\""),
        format!("{t} ALTER COLUMN \"id\" ADD GENERATED BY DEFAULT AS IDENTITY"),
        format!("{t} ALTER COLUMN \"n\" DROP IDENTITY"),
        format!("{t} ADD COLUMN \"m\" int"),
        format!("{t} ADD PRIMARY KEY (\"id\", \"code\")"),
        "CREATE INDEX \"add_ix\" ON \"public\".\"t\" (\"m\")".into(),
        format!("{t} ADD CONSTRAINT \"t_code_key\" UNIQUE (\"code\", \"n\")"),
        format!("{t} ADD CONSTRAINT \"t_m_key\" UNIQUE (\"m\")"),
        format!(
            "{t} ADD CONSTRAINT \"add_fk\" FOREIGN KEY (\"m\") REFERENCES \"public\".\"users\" (\"id\") ON DELETE NO ACTION ON UPDATE NO ACTION"
        ),
    ];
    assert_eq!(alter_table("public", &keyed(), &after), want);
}

#[test]
fn alter_table_pk_cases() {
    let t = "ALTER TABLE \"public\".\"t\"";
    let drop = format!("{t} DROP CONSTRAINT \"t_pkey\"");
    // (pk flags on id, code, n; statements)
    let cases = [
        ([true, false, false], vec![]),
        ([false, false, false], vec![drop.clone()]),
        (
            [false, true, false],
            vec![drop.clone(), format!("{t} ADD PRIMARY KEY (\"code\")")],
        ),
    ];
    for (flags, want) in cases {
        let mut after = unchanged(&keyed());
        for (c, flag) in after.def.columns.iter_mut().zip(flags) {
            c.is_pk = flag;
        }
        assert_eq!(alter_table("public", &keyed(), &after), want, "{flags:?}");
    }

    let mut none = keyed();
    none.primary_key = None;
    none.columns[0].is_pk = false;
    let mut after = unchanged(&none);
    after.def.columns[1].is_pk = true;
    assert_eq!(
        alter_table("public", &none, &after),
        vec![format!("{t} ADD PRIMARY KEY (\"code\")")],
        "no constraint to drop"
    );
}

#[test]
fn renaming_a_pk_column_leaves_the_pk_alone() {
    let mut after = unchanged(&keyed());
    after.def.columns[0].name = "key".into();
    assert_eq!(
        alter_table("public", &keyed(), &after),
        vec!["ALTER TABLE \"public\".\"t\" RENAME COLUMN \"id\" TO \"key\"".to_string()]
    );
}

#[test]
fn create_table_with_identity_pk_and_unique_key() {
    let def = TableDef {
        name: "t".into(),
        columns: vec![
            pk(identity(col("id", "bigint", false, None))),
            pk(col("code", "text", false, None)),
            col("email", "text", true, None),
        ],
        indexes: vec![index("t_code_ix", &["code"], false)],
        foreign_keys: vec![],
        primary_key: None,
        unique_keys: vec![index("t_email_key", &["email"], true)],
    };
    assert_eq!(
        create_table("s", &def),
        s(&[
            "CREATE TABLE \"s\".\"t\" (\"id\" bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL, \"code\" text NOT NULL, \"email\" text, PRIMARY KEY (\"id\", \"code\"))",
            "ALTER TABLE \"s\".\"t\" ADD CONSTRAINT \"t_email_key\" UNIQUE (\"email\")",
            "CREATE INDEX \"t_code_ix\" ON \"s\".\"t\" (\"code\")",
        ])
    );
}
