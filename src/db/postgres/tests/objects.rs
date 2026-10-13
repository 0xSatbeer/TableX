use super::*;

/// The objects of `schema` in the database `db` opens, and how many statements listing them took.
fn objects_of(db: &ConnectionParams, schema: &str) -> (Result<Vec<DbObject>, String>, usize) {
    let session = session(db);
    let Session::Postgres(pg) = &session else {
        unreachable!()
    };
    let before = pg.statements.load(std::sync::atomic::Ordering::Relaxed);
    let objects = block_on(Postgres::default().objects(&session, schema));
    (
        objects,
        pg.statements.load(std::sync::atomic::Ordering::Relaxed) - before,
    )
}

#[test]
fn a_schema_with_5000_tables_lists_them_all_in_at_most_2_statements() {
    let Some(pg) = test_pg() else { return };
    // Its own database, so dropping 5,000 tables never needs 5,000 locks at once.
    let scratch = Scratch::new(&pg, "DATABASE", "");
    let db = on(&pg, &scratch.name);
    admin(
        &db,
        "DO $$ BEGIN FOR i IN 1..5000 LOOP EXECUTE format('CREATE TABLE t%s ()', i); \
                  IF i % 500 = 0 THEN COMMIT; END IF; END LOOP; END $$",
    );
    let (objects, statements) = objects_of(&db, "public");
    let objects = objects.expect("objects");
    assert_eq!(objects.len(), 5000);
    assert!(
        objects
            .iter()
            .all(|o| o.kind == crate::model::ObjectKind::Table && o.name.starts_with('t'))
    );
    assert!(statements <= 2, "{statements} statements");
}

/// The (kind, name) of each object, names without their signature.
fn kinds_and_names(objects: &[DbObject]) -> Vec<(ObjectKind, &str)> {
    objects
        .iter()
        .map(|o| (o.kind, o.name.split('(').next().unwrap()))
        .collect()
}

#[test]
fn objects_come_by_kind_each_sorted_by_name() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    // Created in reverse order, so a listing in creation order fails.
    admin(&pg, &format!(
        "SET search_path = {s};
         CREATE TYPE tb AS ENUM ('x'); CREATE TYPE ta AS ENUM ('x');
         CREATE PROCEDURE pb() LANGUAGE sql AS ''; CREATE PROCEDURE pa() LANGUAGE sql AS '';
         CREATE FUNCTION fb() RETURNS int LANGUAGE sql AS 'SELECT 1'; CREATE FUNCTION fa() RETURNS int LANGUAGE sql AS 'SELECT 1';
         CREATE SEQUENCE sb; CREATE SEQUENCE sa;
         CREATE MATERIALIZED VIEW mb AS SELECT 1 AS x; CREATE MATERIALIZED VIEW ma AS SELECT 1 AS x;
         CREATE VIEW vb AS SELECT 1 AS x; CREATE VIEW va AS SELECT 1 AS x;
         CREATE TABLE b (); CREATE TABLE a (); CREATE TABLE \"B\" ();"
    ));
    use ObjectKind::*;
    let want = [
        (Table, "B"),
        (Table, "a"),
        (Table, "b"),
        (View, "va"),
        (View, "vb"),
        (MaterializedView, "ma"),
        (MaterializedView, "mb"),
        (Sequence, "sa"),
        (Sequence, "sb"),
        (Function, "fa"),
        (Function, "fb"),
        (Procedure, "pa"),
        (Procedure, "pb"),
        (Type, "ta"),
        (Type, "tb"),
    ];
    let (objects, _) = objects_of(&pg, s);
    assert_eq!(kinds_and_names(&objects.expect("objects")), want);
}

#[test]
fn tables_carry_their_row_estimate_unless_never_analyzed() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    admin(
        &pg,
        &format!(
            "SET search_path = {};
         CREATE TABLE t (a int); INSERT INTO t SELECT generate_series(1, 1200); ANALYZE t;
         CREATE TABLE one (a int); INSERT INTO one VALUES (1); ANALYZE one;
         CREATE TABLE emptied (a int); ANALYZE emptied;
         CREATE TABLE fresh (a int); INSERT INTO fresh VALUES (1);
         CREATE VIEW v AS SELECT 1 AS x;",
            schema.name
        ),
    );
    // (object, row estimate)
    let cases = [
        ("t", Some(1200)),
        ("one", Some(1)),
        ("emptied", Some(0)),
        ("fresh", None),
        ("v", None),
    ];
    let (objects, _) = objects_of(&pg, &schema.name);
    let objects = objects.expect("objects");
    for (name, want) in cases {
        let object = objects
            .iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("{name} listed"));
        assert_eq!(object.row_estimate, want, "{name}");
    }
}

#[test]
fn a_partitioned_table_lists_as_a_table_with_its_estimate_once_analyzed() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    admin(
        &pg,
        &format!(
            "SET search_path = {};
         CREATE TABLE ev (d date) PARTITION BY RANGE (d);
         CREATE TABLE ev_2024 PARTITION OF ev FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');
         INSERT INTO ev SELECT '2024-01-01'::date + i % 300 FROM generate_series(1, 1200) i;",
            schema.name
        ),
    );
    let ev = || {
        let (objects, _) = objects_of(&pg, &schema.name);
        objects
            .expect("objects")
            .into_iter()
            .find(|o| o.name == "ev")
            .expect("ev listed")
    };
    let before = ev();
    assert_eq!(
        (before.kind, before.row_estimate),
        (ObjectKind::Table, None),
        "never analyzed"
    );
    admin(&pg, &format!("ANALYZE {}.ev", schema.name));
    assert_eq!(ev().row_estimate, Some(1200), "analyzed");
}

#[test]
fn functions_and_procedures_are_named_with_their_argument_types() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    admin(
        &pg,
        &format!(
            "SET search_path = {};
         CREATE FUNCTION add_user(name text) RETURNS int LANGUAGE sql AS 'SELECT 1';
         CREATE FUNCTION add_user(text, age integer) RETURNS int LANGUAGE sql AS 'SELECT 1';
         CREATE FUNCTION now_ish() RETURNS int LANGUAGE sql AS 'SELECT 1';
         CREATE PROCEDURE p(integer) LANGUAGE sql AS '';",
            schema.name
        ),
    );
    use ObjectKind::*;
    // Argument names are dropped; overloads are separate rows.
    let want = [
        (Function, "add_user(text)".to_string()),
        (Function, "add_user(text, integer)".to_string()),
        (Function, "now_ish()".to_string()),
        (Procedure, "p(integer)".to_string()),
    ];
    let (objects, _) = objects_of(&pg, &schema.name);
    let got: Vec<_> = objects
        .expect("objects")
        .into_iter()
        .map(|o| (o.kind, o.name))
        .collect();
    assert_eq!(got, want);
}

#[test]
fn types_are_enums_composites_domains_and_ranges_but_not_row_or_array_types() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    admin(&pg, &format!(
        "SET search_path = {};
         CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
         CREATE TYPE pair AS (a int, b int);
         CREATE DOMAIN posint AS int CHECK (VALUE > 0);
         CREATE TYPE span AS RANGE (subtype = int);
         CREATE TABLE t (a int); CREATE VIEW v AS SELECT 1 AS x; CREATE MATERIALIZED VIEW m AS SELECT 1 AS x;
         CREATE SEQUENCE s;",
        schema.name
    ));
    // Tables, views, materialized views and sequences each have an implicit row type, and
    // every type an array type (_mood, ...); none of those is listed.
    let want = ["mood", "pair", "posint", "span", "span_multirange"];
    let (objects, _) = objects_of(&pg, &schema.name);
    let objects = objects.expect("objects");
    let types: Vec<_> = objects
        .iter()
        .filter(|o| o.kind == ObjectKind::Type)
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(types, want);
}

#[test]
fn objects_owned_by_an_extension_are_hidden_and_an_empty_schema_lists_nothing() {
    let Some(pg) = test_pg() else { return };
    // Its own database: an extension can be created once per database.
    let scratch = Scratch::new(&pg, "DATABASE", "");
    let db = on(&pg, &scratch.name);
    // hstore brings a type, its array type and functions; ALTER EXTENSION ADD makes the
    // extension own a table, view and sequence too, as PostGIS owns spatial_ref_sys.
    admin(&db, "CREATE SCHEMA app; CREATE SCHEMA empty; CREATE EXTENSION hstore SCHEMA app;
                SET search_path = app;
                CREATE TABLE spatial_ref_sys (); CREATE VIEW ext_view AS SELECT 1 AS x; CREATE SEQUENCE ext_seq;
                ALTER EXTENSION hstore ADD TABLE spatial_ref_sys;
                ALTER EXTENSION hstore ADD VIEW ext_view;
                ALTER EXTENSION hstore ADD SEQUENCE ext_seq;
                CREATE TABLE mine (h hstore);");
    // (schema, objects)
    let cases = [
        ("app", vec![(ObjectKind::Table, "mine")]),
        ("empty", vec![]),
    ];
    for (schema, want) in cases {
        let (objects, _) = objects_of(&db, schema);
        assert_eq!(
            kinds_and_names(&objects.expect("objects")),
            want,
            "{schema}"
        );
    }
}

#[test]
fn objects_of_a_schema_without_usage_fail_with_the_reason() {
    let Some(pg) = test_pg() else { return };
    let role = Scratch::new(&pg, "ROLE", "LOGIN PASSWORD 'pw'");
    let (closed, open) = (
        Scratch::new(&pg, "SCHEMA", ""),
        Scratch::new(&pg, "SCHEMA", ""),
    );
    admin(
        &pg,
        &format!(
            "CREATE TABLE {0}.t (); CREATE TABLE {1}.t (); GRANT USAGE ON SCHEMA {1} TO {2}",
            closed.name, open.name, role.name
        ),
    );
    let missing = super::testing::unique("qx_missing");
    // (schema, objects or the reason)
    let cases = [
        (
            closed.name.as_str(),
            Err(format!("permission denied for schema {}", closed.name)),
        ),
        (
            open.name.as_str(),
            Ok(vec![(ObjectKind::Table, "t".to_string())]),
        ),
        (
            missing.as_str(),
            Err(format!("schema \"{missing}\" does not exist")),
        ),
    ];
    let db = as_user(&pg, &role.name, Some("pw"));
    for (schema, want) in cases {
        let (objects, _) = objects_of(&db, schema);
        assert_eq!(
            objects.map(|o| o.into_iter().map(|o| (o.kind, o.name)).collect()),
            want,
            "{schema}"
        );
    }
}
