//! The Postgres driver: tokio-postgres on the shared runtime.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::lock::Mutex;

use super::tls::Tls;
use super::{
    Driver, Opened, QueryReply, Reply, Session, foreign, foreign_query, reason, spawn, spawn_query,
    timeout_reason,
};
use crate::model::{
    Column, ConnectionParams, DbObject, ForeignKey, Index, ObjectKind, QueryError, ResultSet, Role,
    Row, SslMode, TableDef, Trigger,
};

mod query;

pub struct Postgres {
    /// Wraps the whole connect: TCP, TLS, auth and the first query.
    pub timeout: Duration,
}

impl Default for Postgres {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
        }
    }
}

pub struct PgSession {
    /// Locked per call: binding or reading a portal needs the client exclusively.
    conn: Mutex<Conn>,
    cancel: tokio_postgres::CancelToken,
    /// Set by `cancel`, so the query it stops fails as `Cancelled` rather than as a server error.
    cancelled: AtomicBool,
    /// Drives the socket; ends once the client is dropped and the server has been told goodbye.
    io: tokio::task::JoinHandle<()>,
    /// Statements run through [`PgSession::query`], so tests can count an introspection's round trips.
    statements: std::sync::atomic::AtomicUsize,
}

struct Conn {
    client: tokio_postgres::Client,
    open: Option<query::Open>,
}

impl PgSession {
    pub fn is_closed(&self) -> bool {
        self.io.is_finished()
    }

    async fn query(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, String> {
        self.statements
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let conn = self.conn.lock().await;
        conn.client.query(sql, params).await.map_err(|e| reason(&e))
    }

    /// Runs one statement without parameters, so DDL and transaction control work.
    async fn run(&self, sql: &str) -> Result<(), QueryError> {
        self.statements
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let conn = self.conn.lock().await;
        conn.client
            .batch_execute(sql)
            .await
            .map_err(|e| self.error(&e, sql))
    }

    async fn run_all(&self, statements: &[String]) -> Result<(), QueryError> {
        for (statement, sql) in statements.iter().enumerate() {
            self.run(sql)
                .await
                .map_err(|err| QueryError { statement, ..err })?;
        }
        Ok(())
    }

    async fn run_atomic(&self, statements: &[String]) -> Result<(), QueryError> {
        self.run("BEGIN").await?;
        if let Err(err) = self.run_all(statements).await {
            self.run("ROLLBACK").await.ok();
            return Err(err);
        }
        // A deferred constraint fails at COMMIT; blame the last statement.
        let last = statements.len().saturating_sub(1);
        self.run("COMMIT").await.map_err(|err| QueryError {
            statement: last,
            ..err
        })
    }

    /// The owner named by `sql`'s one row, or `missing` when there is none.
    async fn owner(&self, sql: &str, param: &str, missing: String) -> Result<String, String> {
        let rows = self.query(sql, &[&param]).await?;
        rows.first().map(|row| row.get(0)).ok_or(missing)
    }
}

/// `read` for query calls.
fn run<T: Send + 'static, F>(
    session: &Session,
    run: impl FnOnce(Arc<PgSession>) -> F,
) -> QueryReply<T>
where
    F: Future<Output = Result<T, crate::model::QueryError>> + Send + 'static,
{
    let Session::Postgres(session) = session else {
        return foreign_query();
    };
    spawn_query(run(session.clone()))
}

/// Runs `read` on the session's runtime with `session`, or fails for another driver's session.
fn read<T: Send + 'static, F>(session: &Session, read: impl FnOnce(Arc<PgSession>) -> F) -> Reply<T>
where
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    let Session::Postgres(session) = session else {
        return foreign();
    };
    spawn(read(session.clone()))
}

/// Connects with `params` and runs the first query, all within `timeout`.
async fn open(params: ConnectionParams, timeout: Duration) -> Result<PgSession, String> {
    let ConnectionParams::Postgres {
        host,
        port,
        user,
        password,
        database,
        ssl,
    } = params
    else {
        return Err("not a Postgres connection".into());
    };
    let mut config = tokio_postgres::Config::new();
    config
        .host(&host)
        .port(port)
        .user(&user)
        .dbname(&database)
        .ssl_mode(match ssl {
            SslMode::Disable => tokio_postgres::config::SslMode::Disable,
            SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
            SslMode::Require => tokio_postgres::config::SslMode::Require,
        });
    if let Some(password) = &password {
        config.password(password.expose());
    }
    let connect = async {
        let (client, connection) = config.connect(Tls::get()).await.map_err(|e| reason(&e))?;
        let io = tokio::spawn(async move {
            connection.await.ok();
        });
        client
            .simple_query("SELECT 1")
            .await
            .map_err(|e| reason(&e))?;
        Ok(PgSession {
            cancel: client.cancel_token(),
            conn: Mutex::new(Conn { client, open: None }),
            cancelled: AtomicBool::new(false),
            io,
            statements: Default::default(),
        })
    };
    tokio::time::timeout(timeout, connect)
        .await
        .unwrap_or_else(|_| Err(timeout_reason(timeout)))
}

/// The kinds the objects query numbers 0 to 6, in listing order.
const KINDS: [ObjectKind; 7] = [
    ObjectKind::Table,
    ObjectKind::View,
    ObjectKind::MaterializedView,
    ObjectKind::Sequence,
    ObjectKind::Function,
    ObjectKind::Procedure,
    ObjectKind::Type,
];

impl Driver for Postgres {
    fn connect(&self, params: &ConnectionParams) -> Reply<Session> {
        let open = open(params.clone(), self.timeout);
        spawn(async move { Ok(Session::Postgres(Arc::new(open.await?))) })
    }

    fn test(&self, params: &ConnectionParams) -> Reply<()> {
        let open = open(params.clone(), self.timeout);
        spawn(async move {
            let PgSession { conn, io, .. } = open.await?;
            drop(conn);
            io.await.ok();
            Ok(())
        })
    }

    fn databases(&self, session: &Session) -> Reply<Vec<String>> {
        read(session, |session| async move {
            let sql = "SELECT datname::text FROM pg_database WHERE NOT datistemplate AND datallowconn ORDER BY datname COLLATE \"C\"";
            Ok(session
                .query(sql, &[])
                .await?
                .iter()
                .map(|row| row.get(0))
                .collect())
        })
    }

    fn schemas(&self, session: &Session) -> Reply<Vec<String>> {
        read(session, |session| async move {
            // The pg_ prefix is reserved for system schemas, temp and toast ones included.
            let sql = "SELECT nspname::text FROM pg_namespace WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' \
                       ORDER BY nspname COLLATE \"C\"";
            Ok(session
                .query(sql, &[])
                .await?
                .iter()
                .map(|row| row.get(0))
                .collect())
        })
    }

    fn objects(&self, session: &Session, schema: &str) -> Reply<Vec<DbObject>> {
        let schema = schema.to_string();
        read(session, |session| async move {
            // The catalogs are readable without USAGE, so check it the way a query on the schema would.
            let usage = session
                .query("SELECT has_schema_privilege($1, 'USAGE')", &[&schema])
                .await?;
            if !usage[0].get::<_, bool>(0) {
                return Err(format!("permission denied for schema {schema}"));
            }
            // One statement for every kind, so a schema of any size costs one round trip.
            // Each branch names its catalog and oid, so objects an extension owns can be dropped.
            let sql = "SELECT k, name FROM ( \
                         SELECT CASE c.relkind WHEN 'v' THEN 1 WHEN 'm' THEN 2 WHEN 'S' THEN 3 ELSE 0 END, c.relname::text, \
                           'pg_class'::regclass, c.oid \
                         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'S') \
                       UNION ALL \
                         SELECT CASE p.prokind WHEN 'p' THEN 5 ELSE 4 END, format('%s(%s)', p.proname, oidvectortypes(p.proargtypes)), \
                           'pg_proc'::regclass, p.oid \
                         FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = $1 \
                       UNION ALL \
                         SELECT 6, t.typname::text, 'pg_type'::regclass, t.oid \
                         FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
                         WHERE n.nspname = $1 AND (t.typtype IN ('e', 'd', 'r', 'm') \
                           OR t.typtype = 'c' AND (SELECT relkind FROM pg_class WHERE oid = t.typrelid) = 'c') \
                       ) o(k, name, catalog, oid) \
                       WHERE NOT EXISTS (SELECT FROM pg_depend d WHERE d.classid = o.catalog AND d.objid = o.oid AND d.deptype = 'e') \
                       ORDER BY k, name COLLATE \"C\"";
            let rows = session.query(sql, &[&schema]).await?;
            Ok(rows
                .iter()
                .map(|row| DbObject {
                    kind: KINDS[row.get::<_, i32>(0) as usize],
                    name: row.get(1),
                    columns: None,
                    keys: Vec::new(),
                    indexes: Vec::new(),
                })
                .collect())
        })
    }

    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>> {
        let (schema, object) = (schema.to_string(), object.to_string());
        read(session, |session| async move {
            columns(&session, &schema, &object).await
        })
    }

    fn execute(&self, session: &Session, statements: Vec<String>, atomic: bool) -> QueryReply<()> {
        run(session, |session| async move {
            match atomic {
                true => session.run_atomic(&statements).await,
                false => session.run_all(&statements).await,
            }
        })
    }

    fn roles(&self, session: &Session) -> Reply<Vec<Role>> {
        read(session, |session| async move {
            let sql =
                "SELECT rolname::text, rolcanlogin FROM pg_roles ORDER BY rolname COLLATE \"C\"";
            Ok(session
                .query(sql, &[])
                .await?
                .iter()
                .map(|row| Role {
                    name: row.get(0),
                    can_login: row.get(1),
                })
                .collect())
        })
    }

    fn database_owner(&self, session: &Session) -> Reply<String> {
        read(session, |session| async move {
            let sql = "SELECT pg_get_userbyid(datdba)::text FROM pg_database WHERE datname = current_database()";
            let rows = session.query(sql, &[]).await?;
            Ok(rows[0].get(0))
        })
    }

    fn schema_owner(&self, session: &Session, schema: &str) -> Reply<String> {
        let schema = schema.to_string();
        read(session, |session| async move {
            let sql = "SELECT pg_get_userbyid(nspowner)::text FROM pg_namespace WHERE nspname = $1";
            let missing = format!("schema \"{schema}\" does not exist");
            session.owner(sql, &schema, missing).await
        })
    }

    fn table_def(&self, session: &Session, schema: &str, table: &str) -> Reply<TableDef> {
        let (schema, table) = (schema.to_string(), table.to_string());
        read(session, |session| async move {
            let (primary_key, unique_keys) = constraints(&session, &schema, &table).await?;
            Ok(TableDef {
                columns: columns(&session, &schema, &table).await?,
                indexes: indexes(&session, &schema, &table).await?,
                foreign_keys: foreign_keys(&session, &schema, &table).await?,
                primary_key,
                unique_keys,
                name: table,
            })
        })
    }

    fn triggers(&self, session: &Session, schema: &str, table: &str) -> Reply<Vec<Trigger>> {
        let (schema, table) = (schema.to_string(), table.to_string());
        read(session, |session| async move {
            let sql = "SELECT t.tgname::text, t.tgtype::int4, pg_get_triggerdef(t.oid, true) \
                       FROM pg_trigger t \
                       JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
                       WHERE n.nspname = $1 AND c.relname = $2 AND NOT t.tgisinternal \
                       ORDER BY t.tgname COLLATE \"C\"";
            let rows = session.query(sql, &[&schema, &table]).await?;
            Ok(rows
                .iter()
                .map(|row| {
                    let (timing, events) = trigger_type(row.get(1));
                    Trigger {
                        name: row.get(0),
                        timing,
                        events,
                        definition: row.get(2),
                    }
                })
                .collect())
        })
    }

    fn view_def(&self, session: &Session, schema: &str, view: &str) -> Reply<String> {
        let (schema, view) = (schema.to_string(), view.to_string());
        read(session, |session| async move {
            let sql = "SELECT pg_get_viewdef(format('%I.%I', $1::text, $2::text)::regclass, true)";
            let rows = session.query(sql, &[&schema, &view]).await?;
            Ok(rows[0].get(0))
        })
    }

    fn open(&self, session: &Session, sql: String, autocommit: bool) -> QueryReply<Opened> {
        run(session, |session| async move {
            session.open(&sql, autocommit).await
        })
    }

    fn fetch(&self, session: &Session, max: usize) -> QueryReply<Vec<Row>> {
        run(session, |session| async move { session.fetch(max).await })
    }

    fn close(&self, session: &Session) -> Reply<()> {
        read(session, |session| async move {
            session.close().await.map_err(|e| e.message)
        })
    }

    fn cancel(&self, session: &Session) -> Reply<()> {
        read(session, |session| async move {
            session.cancelled.store(true, Ordering::Relaxed);
            let cancel = session.cancel.cancel_query(Tls::get()).await;
            cancel.map_err(|e| reason(&e))
        })
    }

    fn query(&self, session: &Session, sql: String) -> QueryReply<ResultSet> {
        run(
            session,
            |session| async move { session.query_all(&sql).await },
        )
    }
}

async fn columns(session: &PgSession, schema: &str, object: &str) -> Result<Vec<Column>, String> {
    let sql = "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull, \
                 pg_get_expr(d.adbin, d.adrelid), coalesce(a.attnum = ANY (i.indkey), false), \
                 ARRAY(SELECT e.enumlabel::text FROM pg_enum e WHERE e.enumtypid = a.atttypid ORDER BY e.enumsortorder), \
                 a.attidentity <> '' \
               FROM pg_attribute a \
               JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
               LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
               LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary \
               WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
               ORDER BY a.attnum";
    let rows = session.query(sql, &[&schema, &object]).await?;
    Ok(rows
        .iter()
        .map(|row| Column {
            name: row.get(0),
            data_type: row.get(1),
            nullable: row.get(2),
            default: row.get(3),
            is_pk: row.get(4),
            enum_values: row.get(5),
            identity: row.get(6),
        })
        .collect())
}

/// The table's indexes with their key columns in order, minus those backing a PK, unique or
/// exclusion constraint (expression parts have no column and are left out).
async fn indexes(session: &PgSession, schema: &str, table: &str) -> Result<Vec<Index>, String> {
    let sql = "SELECT ic.relname::text, x.indisunique, \
                 ARRAY(SELECT a.attname::text FROM unnest(x.indkey::int2[]) WITH ORDINALITY k(num, ord) \
                   JOIN pg_attribute a ON a.attrelid = x.indrelid AND a.attnum = k.num \
                   WHERE k.ord <= x.indnkeyatts ORDER BY k.ord) \
               FROM pg_index x JOIN pg_class ic ON ic.oid = x.indexrelid \
               JOIN pg_class c ON c.oid = x.indrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
               WHERE n.nspname = $1 AND c.relname = $2 AND NOT EXISTS (SELECT FROM pg_constraint k \
                 WHERE k.conrelid = x.indrelid AND k.conindid = x.indexrelid AND k.contype IN ('p', 'u', 'x')) \
               ORDER BY ic.relname COLLATE \"C\"";
    let rows = session.query(sql, &[&schema, &table]).await?;
    Ok(rows
        .iter()
        .map(|row| Index {
            name: row.get(0),
            unique: row.get(1),
            columns: row.get(2),
        })
        .collect())
}

/// The primary key constraint's name and the UNIQUE constraints, their columns in key order.
async fn constraints(
    session: &PgSession,
    schema: &str,
    table: &str,
) -> Result<(Option<String>, Vec<Index>), String> {
    let sql = "SELECT k.conname::text, k.contype = 'p', \
                 ARRAY(SELECT a.attname::text FROM unnest(k.conkey) WITH ORDINALITY u(num, ord) \
                   JOIN pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = u.num ORDER BY u.ord) \
               FROM pg_constraint k \
               JOIN pg_class c ON c.oid = k.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
               WHERE k.contype IN ('p', 'u') AND n.nspname = $1 AND c.relname = $2 \
               ORDER BY k.conname COLLATE \"C\"";
    let rows = session.query(sql, &[&schema, &table]).await?;
    let primary_key = rows.iter().find(|r| r.get(1)).map(|r| r.get(0));
    let unique_keys = rows
        .iter()
        .filter(|r| !r.get::<_, bool>(1))
        .map(|row| Index {
            name: row.get(0),
            columns: row.get(2),
            unique: true,
        })
        .collect();
    Ok((primary_key, unique_keys))
}

async fn foreign_keys(
    session: &PgSession,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKey>, String> {
    let sql = "SELECT k.conname::text, \
                 ARRAY(SELECT a.attname::text FROM unnest(k.conkey) WITH ORDINALITY u(num, ord) \
                   JOIN pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = u.num ORDER BY u.ord), \
                 rn.nspname::text, rc.relname::text, \
                 ARRAY(SELECT a.attname::text FROM unnest(k.confkey) WITH ORDINALITY u(num, ord) \
                   JOIN pg_attribute a ON a.attrelid = k.confrelid AND a.attnum = u.num ORDER BY u.ord), \
                 k.confdeltype::text, k.confupdtype::text \
               FROM pg_constraint k \
               JOIN pg_class c ON c.oid = k.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
               JOIN pg_class rc ON rc.oid = k.confrelid JOIN pg_namespace rn ON rn.oid = rc.relnamespace \
               WHERE k.contype = 'f' AND n.nspname = $1 AND c.relname = $2 \
               ORDER BY k.conname COLLATE \"C\"";
    let rows = session.query(sql, &[&schema, &table]).await?;
    Ok(rows
        .iter()
        .map(|row| ForeignKey {
            name: row.get(0),
            columns: row.get(1),
            ref_schema: row.get(2),
            ref_table: row.get(3),
            ref_columns: row.get(4),
            on_delete: action(row.get(5)),
            on_update: action(row.get(6)),
        })
        .collect())
}

/// The SQL keyword for a `pg_constraint` referential action code.
fn action(code: &str) -> String {
    match code {
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => "NO ACTION",
    }
    .into()
}

/// The timing and events a `pg_trigger.tgtype` bitmask encodes (PG's `TRIGGER_TYPE_*`).
fn trigger_type(tgtype: i32) -> (String, String) {
    let timing = match tgtype {
        t if t & 64 != 0 => "INSTEAD OF",
        t if t & 2 != 0 => "BEFORE",
        _ => "AFTER",
    };
    let events = [
        (4, "INSERT"),
        (8, "DELETE"),
        (16, "UPDATE"),
        (32, "TRUNCATE"),
    ]
    .into_iter()
    .filter(|(bit, _)| tgtype & bit != 0)
    .map(|(_, event)| event)
    .collect::<Vec<_>>()
    .join(" OR ");
    (timing.into(), events)
}

/// Helpers for tests against the compose Postgres (`TABLEX_TEST_PG`); T02's listing tests use them too.
#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests;
