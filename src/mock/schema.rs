// ---------------------------------------------------------------- schema tree

use super::*;

pub fn workspaces() -> Vec<Workspace> {
    let mut next_id = 0;
    let mut conn = |name: &str, color: u8, prod: bool, status, params, databases| {
        next_id += 1;
        Connection {
            id: ConnectionId(next_id),
            name: name.into(),
            color,
            is_production: prod,
            status,
            params,
            databases: Some(databases),
        }
    };
    let pg = |host: &str, db: &str| ConnectionParams::Postgres {
        host: host.into(),
        port: 5432,
        user: "app".into(),
        password: None,
        database: db.into(),
        ssl: SslMode::Prefer,
    };
    let sqlite = |path: &str| ConnectionParams::Sqlite {
        path: PathBuf::from(path),
    };
    use ConnectionStatus::*;

    vec![
        Workspace {
            id: 1,
            name: "Acme".into(),
            connections: vec![
                conn(
                    "prod-pg",
                    0,
                    true,
                    Connected,
                    pg("db.acme.internal", "app_db"),
                    pg_databases(),
                ),
                conn(
                    "staging-pg",
                    2,
                    false,
                    Error("timeout expired after 10 s".into()),
                    pg("staging.acme.internal", "app_db"),
                    vec![database("app_db", vec![schema("public", 1, 40)])],
                ),
                conn(
                    "local.db",
                    4,
                    false,
                    Disconnected,
                    sqlite("~/acme/local.db"),
                    vec![database("main", vec![schema("main", 2, 12)])],
                ),
            ],
        },
        Workspace {
            id: 2,
            name: "Side projects".into(),
            connections: vec![
                conn(
                    "dev-pg",
                    5,
                    false,
                    Error("connection refused (localhost:5432)".into()),
                    pg("localhost", "dev"),
                    vec![database("dev", vec![schema("public", 3, 8)])],
                ),
                conn(
                    "notes.db",
                    6,
                    false,
                    Connecting,
                    sqlite("~/notes/notes.db"),
                    vec![database("main", vec![schema("main", 4, 5)])],
                ),
            ],
        },
        Workspace {
            id: 3,
            name: "Analytics".into(),
            connections: vec![conn(
                "warehouse-pg",
                7,
                false,
                Disconnected,
                pg("wh.acme.internal", "warehouse"),
                vec![database("warehouse", vec![schema("marts", 5, 25)])],
            )],
        },
    ]
}

/// prod-pg: 3 databases × 4 schemas; `app_db.public` holds the 2,000-table stress case.
fn pg_databases() -> Vec<Database> {
    let mut salt = 100;
    let mut schemas = |big: bool| {
        ["public", "billing", "audit", "staging"]
            .iter()
            .map(|name| {
                salt += 1;
                schema(name, salt, if big && *name == "public" { 2000 } else { 15 })
            })
            .collect()
    };
    vec![
        database("app_db", schemas(true)),
        database("analytics", schemas(false)),
        database("auth", schemas(false)),
    ]
}

fn database(name: &str, schemas: Vec<Schema>) -> Database {
    Database {
        name: name.into(),
        schemas: Some(schemas),
    }
}

const NOUNS: &[&str] = &[
    "users", "orders", "invoices", "payments", "products", "sessions", "events", "accounts",
    "teams", "projects", "tasks", "comments", "tags", "files", "plans", "coupons",
];

fn schema(name: &str, salt: u64, tables: usize) -> Schema {
    let mut objects: Vec<DbObject> = (0..tables)
        .map(|i| {
            let noun = NOUNS[i % NOUNS.len()];
            let name = if i < NOUNS.len() {
                noun.to_string()
            } else {
                format!("{noun}_{i}")
            };
            table(name, mix(salt ^ i as u64))
        })
        .collect();
    let plain = |kind, name: &str| DbObject {
        kind,
        name: name.into(),
        columns: Some(vec![]),
        keys: vec![],
        indexes: vec![],
        row_estimate: None,
    };
    for view in [
        "active_users",
        "daily_signups",
        "monthly_revenue",
        "order_summary",
        "top_customers",
    ] {
        let mut v = table(view.into(), mix(salt ^ 0xFEED));
        v.kind = ObjectKind::View;
        v.keys.clear();
        v.indexes.clear();
        v.row_estimate = None;
        v.columns.iter_mut().flatten().for_each(|c| c.is_pk = false);
        objects.push(v);
    }
    objects.push(plain(ObjectKind::MaterializedView, "revenue_by_month"));
    for seq in [
        "invoices_id_seq",
        "orders_id_seq",
        "payments_id_seq",
        "users_id_seq",
    ] {
        objects.push(plain(ObjectKind::Sequence, seq));
    }
    for func in [
        "calculate_tax(numeric)",
        "current_tenant()",
        "refresh_stats()",
        "touch_updated_at()",
    ] {
        objects.push(plain(ObjectKind::Function, func));
    }
    objects.push(plain(ObjectKind::Procedure, "archive_old_orders(interval)"));
    objects.push(plain(ObjectKind::Type, "order_status"));
    Schema {
        name: name.into(),
        objects: Some(objects),
    }
}

fn table(name: String, r: u64) -> DbObject {
    const EXTRA: &[(&str, &str)] = &[
        ("email", "text"),
        ("status", "text"),
        ("amount", "numeric(12,2)"),
        ("is_active", "boolean"),
        ("metadata", "jsonb"),
        ("owner_id", "bigint"),
        ("notes", "text"),
        ("updated_at", "timestamptz"),
    ];
    let col = |name: &str, ty: &str, nullable, default: Option<&str>, is_pk| Column {
        name: name.into(),
        data_type: ty.into(),
        nullable,
        default: default.map(Into::into),
        is_pk,
        enum_values: Vec::new(),
        identity: false,
    };
    let mut columns = vec![
        col(
            "id",
            "bigint",
            false,
            Some(&format!("nextval('{name}_id_seq')")),
            true,
        ),
        col("created_at", "timestamptz", false, Some("now()"), false),
    ];
    let extra = 2 + (r % 6) as usize;
    columns.extend(
        EXTRA
            .iter()
            .take(extra)
            .enumerate()
            .map(|(i, (n, t))| col(n, t, (r >> i) & 1 == 1, None, false)),
    );

    let mut keys = vec![Key::Primary {
        name: format!("{name}_pkey"),
        columns: vec!["id".into()],
    }];
    let mut indexes = vec![Index {
        name: format!("{name}_pkey"),
        columns: vec!["id".into()],
        unique: true,
    }];
    if extra > 5 {
        keys.push(Key::Foreign {
            name: format!("{name}_owner_id_fkey"),
            columns: vec!["owner_id".into()],
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
        });
        indexes.push(Index {
            name: format!("{name}_owner_id_idx"),
            columns: vec!["owner_id".into()],
            unique: false,
        });
    }
    DbObject {
        kind: ObjectKind::Table,
        name,
        columns: Some(columns),
        keys,
        indexes,
        row_estimate: Some(mix(r) % 3_000_000),
    }
}
