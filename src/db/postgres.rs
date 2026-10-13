//! The Postgres driver: tokio-postgres on the shared runtime.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use super::tls::Tls;
use super::{Driver, Reply, Session, foreign, reason, spawn, timeout_reason};
use crate::model::{Column, ConnectionParams, DbObject, ObjectKind, SslMode};

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
    client: tokio_postgres::Client,
    /// Drives the socket; ends once the client is dropped and the server has been told goodbye.
    io: tokio::task::JoinHandle<()>,
    /// Statements run through [`PgSession::query`], so tests can count an introspection's round trips.
    statements: std::sync::atomic::AtomicUsize,
}

impl PgSession {
    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    async fn query(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<tokio_postgres::Row>, String> {
        self.statements
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.client.query(sql, params).await.map_err(|e| reason(&e))
    }
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
            client,
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
            let PgSession { client, io, .. } = open.await?;
            drop(client);
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
            let sql = "SELECT k, name, estimate FROM ( \
                         SELECT CASE c.relkind WHEN 'v' THEN 1 WHEN 'm' THEN 2 WHEN 'S' THEN 3 ELSE 0 END, c.relname::text, \
                           CASE WHEN c.relkind IN ('r', 'p') AND c.reltuples >= 0 THEN c.reltuples::int8 END, \
                           'pg_class'::regclass, c.oid \
                         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'S') \
                       UNION ALL \
                         SELECT CASE p.prokind WHEN 'p' THEN 5 ELSE 4 END, format('%s(%s)', p.proname, oidvectortypes(p.proargtypes)), NULL, \
                           'pg_proc'::regclass, p.oid \
                         FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = $1 \
                       UNION ALL \
                         SELECT 6, t.typname::text, NULL, 'pg_type'::regclass, t.oid \
                         FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
                         WHERE n.nspname = $1 AND (t.typtype IN ('e', 'd', 'r', 'm') \
                           OR t.typtype = 'c' AND (SELECT relkind FROM pg_class WHERE oid = t.typrelid) = 'c') \
                       ) o(k, name, estimate, catalog, oid) \
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
                    // reltuples is -1 until the table is first analyzed or vacuumed.
                    row_estimate: row.get::<_, Option<i64>>(2).map(|n| n as u64),
                })
                .collect())
        })
    }

    fn columns(&self, session: &Session, schema: &str, object: &str) -> Reply<Vec<Column>> {
        let (schema, object) = (schema.to_string(), object.to_string());
        read(session, |session| async move {
            let sql = "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull, \
                         pg_get_expr(d.adbin, d.adrelid), coalesce(a.attnum = ANY (i.indkey), false), \
                         ARRAY(SELECT e.enumlabel::text FROM pg_enum e WHERE e.enumtypid = a.atttypid ORDER BY e.enumsortorder) \
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
                })
                .collect())
        })
    }
}

/// Helpers for tests against the compose Postgres (`QUERYX_TEST_PG`); T02's listing tests use them too.
#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests;
