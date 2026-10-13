use super::*;

/// The columns of `schema.object`, and how many statements listing them took.
fn columns_of(
    db: &ConnectionParams,
    schema: &str,
    object: &str,
) -> (Result<Vec<Column>, String>, usize) {
    let session = session(db);
    let Session::Postgres(pg) = &session else {
        unreachable!()
    };
    let before = pg.statements.load(std::sync::atomic::Ordering::Relaxed);
    let columns = block_on(Postgres::default().columns(&session, schema, object));
    (
        columns,
        pg.statements.load(std::sync::atomic::Ordering::Relaxed) - before,
    )
}

fn column(
    name: &str,
    data_type: &str,
    nullable: bool,
    default: Option<&str>,
    is_pk: bool,
) -> Column {
    Column {
        name: name.into(),
        data_type: data_type.into(),
        nullable,
        default: default.map(String::from),
        is_pk,
        enum_values: Vec::new(),
        identity: false,
    }
}

#[test]
fn columns_come_in_ordinal_order_with_type_nullability_default_and_pk() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    admin(&pg, &format!(
        "SET search_path = {};
         CREATE TABLE users (id bigint PRIMARY KEY, gone int, email text NULL DEFAULT 'x', code varchar(20) NOT NULL UNIQUE,
                             twice bigint GENERATED ALWAYS AS (id * 2) STORED);
         ALTER TABLE users DROP COLUMN gone;
         CREATE TABLE pairs (a int, c text, b int, PRIMARY KEY (a, b));
         CREATE TABLE \"Odd Name\" (\"Mixed Col\" numeric(10, 2));
         CREATE VIEW v AS SELECT id, email FROM users;
         CREATE MATERIALIZED VIEW m AS SELECT id FROM users;",
        schema.name
    ));
    let s = schema.name.as_str();
    // (object, columns)
    let cases = [
        (
            "users",
            vec![
                column("id", "bigint", false, None, true),
                column("email", "text", true, Some("'x'::text"), false),
                column("code", "character varying(20)", false, None, false),
                column("twice", "bigint", true, Some("(id * 2)"), false),
            ],
        ),
        (
            "pairs",
            vec![
                column("a", "integer", false, None, true),
                column("c", "text", true, None, false),
                column("b", "integer", false, None, true),
            ],
        ),
        (
            "Odd Name",
            vec![column("Mixed Col", "numeric(10,2)", true, None, false)],
        ),
        (
            "v",
            vec![
                column("id", "bigint", true, None, false),
                column("email", "text", true, None, false),
            ],
        ),
        ("m", vec![column("id", "bigint", true, None, false)]),
        ("no_such_table", vec![]),
    ];
    for (object, want) in cases {
        assert_eq!(columns_of(&pg, s, object).0, Ok(want), "{object}");
    }
}

#[test]
fn enum_columns_carry_their_labels_in_declared_order_in_one_statement() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    admin(
        &pg,
        &format!(
            "SET search_path = {};
         CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
         CREATE TYPE grade AS ENUM ('b', 'a');
         ALTER TYPE grade ADD VALUE 'ab' BEFORE 'a';
         CREATE TABLE t (id int, m mood, g grade, ms mood[], label text);
         CREATE VIEW v AS SELECT m FROM t;",
            schema.name
        ),
    );
    let labels = |l: &[&str]| l.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    // (object, enum_values per column)
    let cases = [
        (
            "t",
            vec![
                vec![],
                labels(&["sad", "ok", "happy"]),
                labels(&["b", "ab", "a"]),
                vec![],
                vec![],
            ],
        ),
        ("v", vec![labels(&["sad", "ok", "happy"])]),
    ];
    for (object, want) in cases {
        let (columns, statements) = columns_of(&pg, &schema.name, object);
        let got: Vec<_> = columns
            .expect("columns")
            .into_iter()
            .map(|c| c.enum_values)
            .collect();
        assert_eq!(got, want, "{object}");
        assert_eq!(statements, 1, "{object}");
    }
}
