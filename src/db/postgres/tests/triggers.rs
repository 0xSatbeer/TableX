use super::*;
use crate::model::Trigger;

#[test]
fn triggers_list_timing_and_events_sorted_by_name() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    admin(
        &pg,
        &format!(
            "CREATE TABLE {s}.t (a int); \
             CREATE FUNCTION {s}.noop() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END'; \
             CREATE TRIGGER b_audit AFTER INSERT OR UPDATE ON {s}.t FOR EACH ROW EXECUTE FUNCTION {s}.noop(); \
             CREATE TRIGGER a_guard BEFORE DELETE ON {s}.t FOR EACH ROW EXECUTE FUNCTION {s}.noop()"
        ),
    );
    let session = session(&pg);

    let triggers = block_on(Postgres::default().triggers(&session, s, "t")).expect("triggers");

    let def = |name: &str, timing: &str, events: &str| Trigger {
        name: name.into(),
        timing: timing.into(),
        events: events.into(),
        definition: format!(
            "CREATE TRIGGER {name} {timing} {events} ON {s}.t FOR EACH ROW EXECUTE FUNCTION {s}.noop()"
        ),
    };
    assert_eq!(
        triggers,
        [
            def("a_guard", "BEFORE", "DELETE"),
            def("b_audit", "AFTER", "INSERT OR UPDATE"),
        ]
    );
}

#[test]
fn view_def_is_the_views_select() {
    let Some(pg) = test_pg() else { return };
    let schema = Scratch::new(&pg, "SCHEMA", "");
    let s = &schema.name;
    admin(
        &pg,
        &format!(
            "CREATE TABLE {s}.t (a int); CREATE VIEW {s}.v AS SELECT a FROM {s}.t WHERE a > 1"
        ),
    );
    let session = session(&pg);

    let def = block_on(Postgres::default().view_def(&session, s, "v")).expect("view_def");

    assert_eq!(def, format!(" SELECT a\n   FROM {s}.t\n  WHERE a > 1;"));
}
