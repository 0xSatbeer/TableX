//! The per-session thread that owns the connection, so an open result can keep its statement
//! stepping between `fetch`es (a rusqlite `Rows` borrows its `Statement`, which borrows the connection).

use std::sync::mpsc::Receiver;

use futures::channel::oneshot::Sender;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Rows, Statement};

use super::{ColumnMeta, Opened, Row, Value};

pub(super) enum Job {
    /// Any call that runs to completion; it runs beside the open result.
    Run(Box<dyn FnOnce(&Connection) + Send>),
    Open(String, Sender<rusqlite::Result<Opened>>),
    Fetch(usize, Sender<rusqlite::Result<Vec<Row>>>),
    Close(Sender<()>),
}

/// Serves `jobs` until every sender (the session) is gone.
pub(super) fn work(conn: Connection, jobs: Receiver<Job>) {
    let mut next = jobs.recv().ok();
    while let Some(job) = next.take() {
        next = match job {
            Job::Run(run) => {
                run(&conn);
                jobs.recv().ok()
            }
            Job::Open(sql, reply) => serve(&conn, &sql, reply, &jobs),
            Job::Fetch(_, reply) => {
                reply.send(Ok(Vec::new())).ok();
                jobs.recv().ok()
            }
            Job::Close(reply) => {
                reply.send(()).ok();
                jobs.recv().ok()
            }
        };
    }
}

/// Opens `sql` and serves jobs while its result is open; returns the next job once it is closed.
fn serve(
    conn: &Connection,
    sql: &str,
    reply: Sender<rusqlite::Result<Opened>>,
    jobs: &Receiver<Job>,
) -> Option<Job> {
    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            reply.send(Err(err)).ok();
            return jobs.recv().ok();
        }
    };
    if stmt.column_count() == 0 {
        reply.send(affected(conn, &mut stmt)).ok();
        return jobs.recv().ok();
    }
    let columns = columns(&stmt);
    let width = columns.len();
    reply.send(Ok(Opened::Rows(columns))).ok();
    let mut rows = stmt.raw_query();
    loop {
        match jobs.recv().ok()? {
            Job::Run(run) => run(conn),
            Job::Fetch(max, reply) => {
                let got = page(&mut rows, width, max);
                let open = matches!(&got, Ok(got) if got.len() >= max);
                if !open {
                    drop(rows);
                    reply.send(got).ok();
                    return jobs.recv().ok();
                }
                reply.send(got).ok();
            }
            Job::Close(reply) => {
                drop(rows);
                reply.send(()).ok();
                return jobs.recv().ok();
            }
            open @ Job::Open(..) => return Some(open),
        }
    }
}

/// Runs `stmt` to completion: the rows it changed, 0 for statements that change none (DDL),
/// where SQLite would repeat the last DML's count.
fn affected(conn: &Connection, stmt: &mut Statement) -> rusqlite::Result<Opened> {
    let before = conn.total_changes();
    let changes = stmt.execute([])?;
    let changed = conn.total_changes() != before;
    Ok(Opened::Affected(if changed { changes as u64 } else { 0 }))
}

pub(super) fn columns(stmt: &Statement) -> Vec<ColumnMeta> {
    stmt.columns()
        .iter()
        .map(|col| ColumnMeta {
            name: col.name().into(),
            data_type: col.decl_type().unwrap_or_default().into(),
            ..ColumnMeta::default()
        })
        .collect()
}

/// Up to `max` more rows of `width` cells.
pub(super) fn page(rows: &mut Rows, width: usize, max: usize) -> rusqlite::Result<Vec<Row>> {
    let mut page = Vec::new();
    while page.len() < max {
        let Some(row) = rows.next()? else {
            break;
        };
        let cells = (0..width).map(|i| row.get_ref(i).map(value));
        page.push(cells.collect::<rusqlite::Result<Row>>()?);
    }
    Ok(page)
}

fn value(cell: ValueRef) -> Value {
    match cell {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::Int(n),
        ValueRef::Real(x) => Value::Float(x),
        ValueRef::Text(text) => Value::Text(String::from_utf8_lossy(text).into()),
        ValueRef::Blob(bytes) => Value::Bytes(bytes.into()),
    }
}
