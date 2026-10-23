//! Per-dialect keyword and built-in function names offered by completion.

use crate::model::DbKind;

const COMMON: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASE",
    "CAST",
    "CHECK",
    "COLUMN",
    "COMMIT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FOREIGN",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NATURAL",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "PRIMARY",
    "REFERENCES",
    "RETURNING",
    "RIGHT",
    "ROLLBACK",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "TRIGGER",
    "TRUE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WITH",
];

const POSTGRES: &[&str] = &[
    "ANALYZE",
    "ANY",
    "ARRAY",
    "ASYMMETRIC",
    "COLLATE",
    "CONCURRENTLY",
    "COPY",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "CURRENT_USER",
    "DO",
    "FETCH",
    "FOR",
    "FUNCTION",
    "GRANT",
    "ILIKE",
    "LATERAL",
    "MATERIALIZED",
    "ONLY",
    "OVER",
    "PARTITION",
    "RECURSIVE",
    "REVOKE",
    "ROLE",
    "SCHEMA",
    "SEQUENCE",
    "SHOW",
    "SIMILAR",
    "SOME",
    "TRUNCATE",
    "VACUUM",
    "WINDOW",
];

const SQLITE: &[&str] = &[
    "ATTACH",
    "AUTOINCREMENT",
    "CONFLICT",
    "DETACH",
    "GLOB",
    "IGNORE",
    "INDEXED",
    "PRAGMA",
    "RAISE",
    "REINDEX",
    "REPLACE",
    "ROWID",
    "STRICT",
    "VACUUM",
    "WITHOUT",
];

const COMMON_FUNCTIONS: &[&str] = &[
    "abs", "avg", "coalesce", "count", "length", "lower", "max", "min", "nullif", "replace",
    "round", "substr", "sum", "trim", "upper",
];

const POSTGRES_FUNCTIONS: &[&str] = &[
    "array_agg",
    "date_trunc",
    "generate_series",
    "json_agg",
    "jsonb_build_object",
    "now",
    "string_agg",
    "to_char",
];

const SQLITE_FUNCTIONS: &[&str] = &[
    "date",
    "datetime",
    "group_concat",
    "ifnull",
    "json_extract",
    "julianday",
    "strftime",
];

pub fn keywords(dialect: DbKind) -> impl Iterator<Item = &'static str> {
    let own = match dialect {
        DbKind::Postgres => POSTGRES,
        DbKind::Sqlite => SQLITE,
    };
    COMMON.iter().chain(own).copied()
}

pub fn functions(dialect: DbKind) -> impl Iterator<Item = &'static str> {
    let own = match dialect {
        DbKind::Postgres => POSTGRES_FUNCTIONS,
        DbKind::Sqlite => SQLITE_FUNCTIONS,
    };
    COMMON_FUNCTIONS.iter().chain(own).copied()
}

pub fn is_keyword(word: &str, dialect: DbKind) -> bool {
    keywords(dialect).any(|k| k.eq_ignore_ascii_case(word))
}

/// Words that end a table reference or can't be an alias; the rest (`role`, `key`, `index`…)
/// are valid bare table names.
const RESERVED: &[&str] = &[
    "AND",
    "AS",
    "BY",
    "CASE",
    "CROSS",
    "DELETE",
    "ELSE",
    "END",
    "EXCEPT",
    "FETCH",
    "FOR",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "IN",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTO",
    "IS",
    "JOIN",
    "LATERAL",
    "LEFT",
    "LIMIT",
    "NATURAL",
    "NOT",
    "OFFSET",
    "ON",
    "ONLY",
    "OR",
    "ORDER",
    "OUTER",
    "RETURNING",
    "RIGHT",
    "SELECT",
    "SET",
    "THEN",
    "UNION",
    "UPDATE",
    "USING",
    "VALUES",
    "WHEN",
    "WHERE",
    "WINDOW",
    "WITH",
];

pub fn is_reserved(word: &str) -> bool {
    RESERVED.iter().any(|k| k.eq_ignore_ascii_case(word))
}
