use super::testing::{Scratch, admin, test_pg};
use super::*;
use crate::model::Secret;
use futures::executor::block_on;

mod catalogs;
mod columns;
mod connect;
mod ddl;
mod objects;
mod query;

/// `pg` logging in as `user` with `password`.
fn as_user(pg: &ConnectionParams, user: &str, password: Option<&str>) -> ConnectionParams {
    let ConnectionParams::Postgres {
        host,
        port,
        database,
        ssl,
        ..
    } = pg.clone()
    else {
        unreachable!()
    };
    ConnectionParams::Postgres {
        host,
        port,
        user: user.into(),
        password: password.map(Secret::new),
        database,
        ssl,
    }
}

/// `pg` opened on `database` instead.
fn on(pg: &ConnectionParams, database: &str) -> ConnectionParams {
    let ConnectionParams::Postgres {
        host,
        port,
        user,
        password,
        ssl,
        ..
    } = pg.clone()
    else {
        unreachable!()
    };
    ConnectionParams::Postgres {
        host,
        port,
        user,
        password,
        database: database.into(),
        ssl,
    }
}

fn session(params: &ConnectionParams) -> Session {
    block_on(Postgres::default().connect(params)).expect("connect")
}
