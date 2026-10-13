//! Statements the user runs: one open portal per session, its rows decoded into `Value`s.

use std::error::Error as StdError;
use std::sync::atomic::Ordering;

use bytes::BytesMut;
use tokio_postgres::error::{ErrorPosition, SqlState};
use tokio_postgres::types::{FromSql, IsNull, Kind, ToSql, Type, to_sql_checked};
use tokio_postgres::{Client, Error, Portal, Statement};

use super::{Conn, PgSession};
use crate::db::{Opened, reason};
use crate::model::{ColumnMeta, QueryError, QueryErrorKind, ResultSet, Row, Value};

/// The result `open` left on the server for `fetch`.
pub(super) struct Open {
    portal: Portal,
    statement: Statement,
    sql: String,
    autocommit: bool,
}

impl PgSession {
    pub(super) async fn open(&self, sql: &str, autocommit: bool) -> Result<Opened, QueryError> {
        let mut conn = self.conn.lock().await;
        let Conn { client, open } = &mut *conn;
        finish(client, open.take(), true)
            .await
            .map_err(|e| self.error(&e, ""))?;
        self.statements.fetch_add(1, Ordering::Relaxed);
        let statement = client.prepare(sql).await.map_err(|e| self.error(&e, sql))?;
        if statement.columns().is_empty() {
            let affected = client.execute(&statement, &[]).await;
            return affected
                .map(Opened::Affected)
                .map_err(|e| self.error(&e, sql));
        }
        let portal = match bind(client, &statement).await {
            Ok(portal) => portal,
            Err(err) => {
                let err = self.error(&err, sql);
                if autocommit {
                    client.batch_execute("ROLLBACK").await.ok();
                }
                return Err(err);
            }
        };
        let columns = statement.columns().iter().map(meta).collect();
        *open = Some(Open {
            portal,
            statement,
            sql: sql.into(),
            autocommit,
        });
        Ok(Opened::Rows(columns))
    }

    pub(super) async fn fetch(&self, max: usize) -> Result<Vec<Row>, QueryError> {
        let mut conn = self.conn.lock().await;
        let Conn { client, open } = &mut *conn;
        // tokio-postgres reads a 0-row page as "every row", which would leave the result open.
        let (Some(result), 1..) = (open.as_mut(), max) else {
            return Ok(Vec::new());
        };
        let (portal, statement) = (result.portal.clone(), result.statement.clone());
        let rows = async {
            let rows = portal_rows(client, &portal, i32::try_from(max).unwrap_or(i32::MAX)).await?;
            decode(client, &statement, &rows).await
        };
        match rows.await {
            Ok(rows) if rows.len() >= max => Ok(rows),
            Ok(rows) => {
                finish(client, open.take(), true)
                    .await
                    .map_err(|e| self.error(&e, ""))?;
                Ok(rows)
            }
            Err(err) => {
                let err = self.error(&err, &result.sql);
                finish(client, open.take(), false).await.ok();
                Err(err)
            }
        }
    }

    pub(super) async fn close(&self) -> Result<(), QueryError> {
        let mut conn = self.conn.lock().await;
        let Conn { client, open } = &mut *conn;
        finish(client, open.take(), true)
            .await
            .map_err(|e| self.error(&e, ""))
    }

    pub(super) async fn query_all(&self, sql: &str) -> Result<ResultSet, QueryError> {
        let conn = self.conn.lock().await;
        self.statements.fetch_add(1, Ordering::Relaxed);
        let result = async {
            let statement = conn.client.prepare(sql).await?;
            let rows = conn.client.query(&statement, &[]).await?;
            Ok(ResultSet {
                columns: statement.columns().iter().map(meta).collect(),
                rows: decode(&conn.client, &statement, &rows).await?,
            })
        };
        result.await.map_err(|e| self.error(&e, sql))
    }

    /// `err` as a query failure; `sql` is the statement it came from, for the error position.
    fn error(&self, err: &Error, sql: &str) -> QueryError {
        let db = err.as_db_error();
        let fatal = db.is_some_and(|db| matches!(db.severity(), "FATAL" | "PANIC"));
        let cancelled = db.is_some_and(|db| *db.code() == SqlState::QUERY_CANCELED);
        let kind = if err.is_closed() || fatal || self.is_closed() {
            QueryErrorKind::Disconnected
        } else if cancelled && self.cancelled.swap(false, Ordering::Relaxed) {
            QueryErrorKind::Cancelled
        } else {
            QueryErrorKind::Server
        };
        // The server counts characters from 1, one past the end for "at end of input"; callers index by bytes.
        let position = match db.and_then(|db| db.position()) {
            Some(ErrorPosition::Original(at)) => sql
                .char_indices()
                .chain([(sql.len(), ' ')])
                .nth((*at as usize).saturating_sub(1))
                .map(|(byte, _)| byte),
            _ => None,
        };
        QueryError {
            kind,
            message: reason(err),
            position,
            statement: 0,
        }
    }
}

// shortcut: tokio-postgres binds and reads portals only through a `Transaction`, and getting one
// sends a BEGIN (a harmless warning inside the transaction already open). It is forgotten so its
// drop doesn't roll back; upgrade if tokio-postgres grows `Client::bind`.
async fn bind(client: &mut Client, statement: &Statement) -> Result<Portal, Error> {
    let tx = client.transaction().await?;
    let portal = tx.bind(statement, &[]).await;
    std::mem::forget(tx);
    portal
}

async fn portal_rows(
    client: &mut Client,
    portal: &Portal,
    max: i32,
) -> Result<Vec<tokio_postgres::Row>, Error> {
    let tx = client.transaction().await?;
    let rows = tx.query_portal(portal, max).await;
    std::mem::forget(tx);
    rows
}

/// Drops `open`'s portal and, in autocommit, ends its transaction: COMMIT when `commit`.
async fn finish(client: &Client, open: Option<Open>, commit: bool) -> Result<(), Error> {
    let Some(open) = open else { return Ok(()) };
    let autocommit = open.autocommit;
    drop(open);
    match (autocommit, commit) {
        (false, _) => Ok(()),
        (true, true) => client.batch_execute("COMMIT").await,
        (true, false) => client.batch_execute("ROLLBACK").await,
    }
}

fn meta(column: &tokio_postgres::Column) -> ColumnMeta {
    let ty = column.type_();
    ColumnMeta {
        name: column.name().into(),
        data_type: ty.name().into(),
        is_pk: false,
        enum_values: match ty.kind() {
            Kind::Enum(labels) => labels.iter().map(|l| l.as_str().into()).collect(),
            _ => Default::default(),
        },
    }
}

/// A cell's wire bytes, taken as they are and sent back as they are.
struct Raw<'a>(&'a [u8]);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn StdError + Sync + Send>> {
        Ok(Raw(raw))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

impl ToSql for Raw<'_> {
    fn to_sql(
        &self,
        _: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn StdError + Sync + Send>> {
        out.extend_from_slice(self.0);
        Ok(IsNull::No)
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

impl std::fmt::Debug for Raw<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes", self.0.len())
    }
}

/// `rows` as `Value`s. Cells we don't decode ourselves get the server's text form: sent back
/// as they came, through `format`, which uses the type's output function (a `::text` cast may not).
async fn decode(
    client: &Client,
    statement: &Statement,
    rows: &[tokio_postgres::Row],
) -> Result<Vec<Row>, Error> {
    let types: Vec<&Type> = statement.columns().iter().map(|c| c.type_()).collect();
    let mut decoded: Vec<Vec<Option<Value>>> = rows
        .iter()
        .map(|row| {
            let cell = |i| match row.get::<_, Option<Raw>>(i) {
                None => Some(Value::Null),
                Some(Raw(raw)) => value(types[i], raw),
            };
            (0..types.len()).map(cell).collect()
        })
        .collect();
    let pending: Vec<usize> = (0..types.len())
        .filter(|&i| decoded.iter().any(|row| row[i].is_none()))
        .collect();
    if !pending.is_empty() {
        let casts: Vec<String> = (1..=pending.len())
            .map(|n| format!("format('%s', ${n})"))
            .collect();
        let param_types: Vec<Type> = pending.iter().map(|&i| types[i].clone()).collect();
        let cast = client
            .prepare_typed(&format!("SELECT {}", casts.join(", ")), &param_types)
            .await?;
        // shortcut: one pipelined query per row; batch a page into one statement if pages get slow.
        let texts = futures::future::try_join_all(rows.iter().map(|row| {
            let cells: Vec<Option<Raw>> = pending.iter().map(|&i| row.get(i)).collect();
            let cast = &cast;
            async move {
                let params: Vec<&(dyn ToSql + Sync)> = cells.iter().map(|c| c as _).collect();
                client.query_one(cast, &params).await
            }
        }))
        .await?;
        for (row, text) in decoded.iter_mut().zip(&texts) {
            for (n, &i) in pending.iter().enumerate() {
                row[i].get_or_insert_with(|| match text.get::<_, Option<&str>>(n) {
                    Some(s) => Value::Text(s.into()),
                    None => Value::Null,
                });
            }
        }
    }
    Ok(decoded
        .into_iter()
        .map(|row| row.into_iter().map(|v| v.unwrap_or(Value::Null)).collect())
        .collect())
}

/// A non-null cell of type `ty`, or `None` to ask the server for its text form.
fn value(ty: &Type, raw: &[u8]) -> Option<Value> {
    let text = |raw| std::str::from_utf8(raw).ok().map(Into::into);
    Some(match *ty {
        Type::BOOL => Value::Bool(bool::from_sql(ty, raw).ok()?),
        Type::INT2 => Value::Int(i16::from_sql(ty, raw).ok()?.into()),
        Type::INT4 => Value::Int(i32::from_sql(ty, raw).ok()?.into()),
        Type::INT8 => Value::Int(i64::from_sql(ty, raw).ok()?),
        Type::FLOAT4 => Value::Float(f32::from_sql(ty, raw).ok()?.into()),
        Type::FLOAT8 => Value::Float(f64::from_sql(ty, raw).ok()?),
        Type::TEXT | Type::VARCHAR | Type::NAME | Type::BPCHAR => Value::Text(text(raw)?),
        Type::UUID if raw.len() == 16 => {
            let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
            let (a, b, c, d, e) = (
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..],
            );
            Value::Text(format!("{a}-{b}-{c}-{d}-{e}").into())
        }
        Type::BYTEA => Value::Bytes(raw.into()),
        Type::JSON => Value::Json(text(raw)?),
        // jsonb's binary form is a version byte, then the text.
        Type::JSONB => Value::Json(text(raw.get(1..)?)?),
        Type::TIMESTAMP => Value::Timestamp(unix_micros(ty, raw)?),
        Type::TIMESTAMPTZ => Value::TimestampTz {
            micros: unix_micros(ty, raw)?,
            offset_secs: 0,
        },
        _ if matches!(ty.kind(), Kind::Enum(_)) => Value::Text(text(raw)?),
        _ => return None,
    })
}

/// A timestamp's microseconds since 1970; `None` for ±infinity, which only the server spells.
fn unix_micros(ty: &Type, raw: &[u8]) -> Option<i64> {
    const PG_EPOCH: i64 = 946_684_800_000_000;
    let micros = i64::from_sql(ty, raw).ok()?;
    (micros != i64::MAX && micros != i64::MIN).then(|| micros.checked_add(PG_EPOCH))?
}
