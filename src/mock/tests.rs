use super::*;

#[test]
fn same_seed_same_output() {
    assert_eq!(workspaces(), workspaces());
    assert_eq!(mock_query_log(200), mock_query_log(200));
    assert_eq!(wide_table(5), wide_table(5));
    assert_eq!(run_query("select 1"), run_query("select 1"));
    // Pinned values catch changes that are deterministic within a run but not across versions.
    assert_eq!(big_events_row(0)[0], Value::Int(1));
    assert_eq!(big_events_row(999_999)[0], Value::Int(1_000_000));
    assert_eq!(big_events_row(999_999), big_events_row(999_999));
}

#[test]
fn last_big_events_row_is_o1() {
    let start = std::time::Instant::now();
    let row = big_events_row(BIG_EVENTS_ROWS - 1);
    assert!(start.elapsed() < Duration::from_millis(50));
    assert_eq!(row.len(), big_events_columns().len());
}

#[test]
fn mock_shape_matches_ticket() {
    let ws = workspaces();
    let conns: Vec<_> = ws.iter().flat_map(|w| &w.connections).collect();
    assert_eq!(ws.len(), 3);
    assert_eq!(conns.len(), 6);
    assert_eq!(conns.iter().filter(|c| c.is_production).count(), 1);
    let ids: std::collections::HashSet<_> = conns.iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), 6, "connection ids are unique");

    let prod = conns.iter().find(|c| c.is_production).unwrap();
    assert_eq!(prod.kind(), DbKind::Postgres);
    let databases = prod.databases.as_ref().unwrap();
    assert_eq!(databases.len(), 3);
    assert!(
        databases
            .iter()
            .all(|d| d.schemas.as_ref().unwrap().len() == 4)
    );
    let tables = |s: &Schema| {
        s.objects
            .iter()
            .flatten()
            .filter(|o| o.kind == ObjectKind::Table)
            .count()
    };
    assert_eq!(tables(&databases[0].schemas.as_ref().unwrap()[0]), 2000);
    assert!(
        conns
            .iter()
            .any(|c| c.kind() == DbKind::Sqlite && c.databases.as_ref().unwrap().len() == 1)
    );

    assert_eq!(wide_table(1).columns.len(), 300);
    assert!(edge_cases().rows.iter().any(|r| r[1] == Value::Null));
}

fn all_objects() -> Vec<DbObject> {
    workspaces()
        .into_iter()
        .flat_map(|w| w.connections)
        .flat_map(|c| c.databases.unwrap())
        .flat_map(|d| d.schemas.unwrap())
        .flat_map(|s| s.objects.unwrap())
        .collect()
}

#[test]
fn only_id_columns_are_primary_keys() {
    for o in all_objects() {
        for c in o.columns.iter().flatten() {
            let want = o.kind == ObjectKind::Table && c.name == "id";
            assert_eq!(c.is_pk, want, "{}.{}: is_pk", o.name, c.name);
        }
    }
    let result_sets = [
        big_events_columns(),
        wide_table(0).columns,
        edge_cases().columns,
    ];
    for c in result_sets.iter().flatten() {
        assert_eq!(c.is_pk, &*c.name == "id", "result column {}: is_pk", c.name);
    }
}

#[test]
fn run_query_picks_outcome_from_sql() {
    assert!(matches!(
        run_query("  SELECT * FROM users"),
        QueryOutcome::Rows(_)
    ));
    assert!(matches!(
        run_query("update t set a = 1"),
        QueryOutcome::Affected(_)
    ));
    assert_eq!(
        run_query("SELEC 1"),
        QueryOutcome::Error("syntax error at or near \"SELEC\"".into())
    );
}

#[test]
fn latency_is_clamped() {
    set_latency(5000);
    assert_eq!(latency(), Duration::from_secs(2));
    set_latency(0);
}
