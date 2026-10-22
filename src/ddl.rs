//! Pure DDL builders: turn the object dialogs' input into the Postgres statements to run.
//! Type and default text is SQL the user typed, so it passes through raw.
#![allow(dead_code, reason = "the object dialogs call these")]

use crate::model::{Column, ForeignKey, Index, ObjectKind, TableDef, Trigger};

/// A table as edited: `origins[i]` is the original name of `def.columns[i]`, `None` if new.
#[derive(Clone, Debug, PartialEq)]
pub struct TableEdit {
    pub def: TableDef,
    pub origins: Vec<Option<String>>,
}

pub fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Object `name` as SQL names it: `"schema"."name"`, or `"name"` without a schema (SQLite);
/// a function's argument types (`f(integer)`) stay outside the quotes.
pub fn object_ref(kind: ObjectKind, schema: Option<&str>, name: &str) -> String {
    let (base, args) = match name.split_once('(') {
        Some((base, args)) if kind == ObjectKind::Function => (base, format!("({args}")),
        _ => (name, String::new()),
    };
    let named = schema.map_or_else(|| ident(base), |schema| qualified(schema, base));
    named + &args
}

/// `DROP <kind> <object>`; `None` for other kinds and for a function without its signature.
pub fn drop_object(kind: ObjectKind, schema: Option<&str>, name: &str) -> Option<String> {
    let what = match kind {
        ObjectKind::Table => "TABLE",
        ObjectKind::View => "VIEW",
        ObjectKind::MaterializedView => "MATERIALIZED VIEW",
        ObjectKind::Sequence => "SEQUENCE",
        ObjectKind::Function if name.contains('(') => "FUNCTION",
        _ => return None,
    };
    Some(format!("DROP {what} {}", object_ref(kind, schema, name)))
}

pub fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

pub fn alter_database(name: &str, new_name: &str, owner: &str, new_owner: &str) -> Vec<String> {
    rename_and_own("DATABASE", name, new_name, owner, new_owner)
}

pub fn alter_schema(name: &str, new_name: &str, owner: &str, new_owner: &str) -> Vec<String> {
    rename_and_own("SCHEMA", name, new_name, owner, new_owner)
}

fn rename_and_own(
    kind: &str,
    name: &str,
    new_name: &str,
    owner: &str,
    new_owner: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    if name != new_name {
        out.push(format!(
            "ALTER {kind} {} RENAME TO {}",
            ident(name),
            ident(new_name)
        ));
    }
    if owner != new_owner {
        out.push(format!(
            "ALTER {kind} {} OWNER TO {}",
            ident(new_name),
            ident(new_owner)
        ));
    }
    out
}

pub fn create_database(name: &str, owner: &str) -> Vec<String> {
    vec![format!(
        "CREATE DATABASE {} OWNER {}",
        ident(name),
        ident(owner)
    )]
}

/// A user when `password` is given (LOGIN), else a NOLOGIN role; then one GRANT per membership.
pub fn create_role(name: &str, password: Option<&str>, member_of: &[String]) -> Vec<String> {
    let login = match password {
        Some(p) => format!("LOGIN PASSWORD {}", literal(p)),
        None => "NOLOGIN".into(),
    };
    let create = format!("CREATE ROLE {} {login}", ident(name));
    let grants = member_of
        .iter()
        .map(|m| format!("GRANT {} TO {}", ident(m), ident(name)));
    std::iter::once(create).chain(grants).collect()
}

pub fn create_table(schema: &str, def: &TableDef) -> Vec<String> {
    let mut columns: Vec<String> = def.columns.iter().map(column_def).collect();
    let pk = pk_names(&def.columns);
    if !pk.is_empty() {
        columns.push(format!("PRIMARY KEY ({})", idents(&pk)));
    }
    let create = format!(
        "CREATE TABLE {} ({})",
        qualified(schema, &def.name),
        columns.join(", ")
    );
    let uniques = def
        .unique_keys
        .iter()
        .map(|u| add_unique(schema, &def.name, u));
    let indexes = def
        .indexes
        .iter()
        .map(|i| create_index(schema, &def.name, i));
    let fks = def
        .foreign_keys
        .iter()
        .map(|f| add_fk(schema, &def.name, f));
    std::iter::once(create)
        .chain(uniques)
        .chain(indexes)
        .chain(fks)
        .collect()
}

/// The DDL tab's script: `create_table`'s statements, then each trigger's definition.
pub fn table_ddl(schema: &str, def: &TableDef, triggers: &[Trigger]) -> String {
    let triggers = triggers.iter().map(|t| t.definition.clone());
    create_table(schema, def)
        .into_iter()
        .chain(triggers)
        .map(|s| format!("{};\n", s.trim_end().trim_end_matches(';')))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Statements in a fixed order: rename table, drop FKs, drop the PK, drop indexes and UNIQUE
/// constraints, drop columns, rename columns, alter columns, add columns, add the PK, create
/// indexes and UNIQUE constraints, add FKs. An edited index, UNIQUE or FK is dropped and
/// recreated.
pub fn alter_table(schema: &str, before: &TableDef, after: &TableEdit) -> Vec<String> {
    let new = &after.def;
    let table = qualified(schema, &new.name);
    let alter = |rest: String| format!("ALTER TABLE {table} {rest}");
    let mut out = Vec::new();
    if before.name != new.name {
        out.push(format!(
            "ALTER TABLE {} RENAME TO {}",
            qualified(schema, &before.name),
            ident(&new.name)
        ));
    }
    let gone_fks = removed(&before.foreign_keys, &new.foreign_keys, |f| &f.name);
    out.extend(gone_fks.map(|f| alter(format!("DROP CONSTRAINT {}", ident(&f.name)))));
    // By original name, so renaming a PK column alone leaves the PK as it is.
    let pk_origins: Vec<Option<&String>> = new
        .columns
        .iter()
        .zip(&after.origins)
        .filter(|(c, _)| c.is_pk)
        .map(|(_, o)| o.as_ref())
        .collect();
    let pk_changed = pk_origins
        != pk_names(&before.columns)
            .iter()
            .map(Some)
            .collect::<Vec<_>>();
    if let Some(n) = before.primary_key.as_ref().filter(|_| pk_changed) {
        out.push(alter(format!("DROP CONSTRAINT {}", ident(n))));
    }
    let gone_ix = removed(&before.indexes, &new.indexes, |i| &i.name);
    out.extend(gone_ix.map(|i| format!("DROP INDEX {}", qualified(schema, &i.name))));
    let gone_uniques = removed(&before.unique_keys, &new.unique_keys, |i| &i.name);
    out.extend(gone_uniques.map(|u| alter(format!("DROP CONSTRAINT {}", ident(&u.name)))));

    let pairs: Vec<(Option<&Column>, &Column)> = new
        .columns
        .iter()
        .zip(&after.origins)
        .map(|(c, o)| {
            (
                o.as_ref()
                    .and_then(|o| before.columns.iter().find(|b| &b.name == o)),
                c,
            )
        })
        .collect();
    let kept = |b: &Column| {
        pairs
            .iter()
            .any(|(old, _)| old.is_some_and(|o| o.name == b.name))
    };
    for b in before.columns.iter().filter(|b| !kept(b)) {
        out.push(alter(format!("DROP COLUMN {}", ident(&b.name))));
    }
    for (old, c) in &pairs {
        if let Some(old) = old.filter(|o| o.name != c.name) {
            out.push(alter(format!(
                "RENAME COLUMN {} TO {}",
                ident(&old.name),
                ident(&c.name)
            )));
        }
    }
    for (old, c) in &pairs {
        let Some(old) = old else { continue };
        let col = format!("ALTER COLUMN {}", ident(&c.name));
        if old.data_type != c.data_type {
            out.push(alter(format!("{col} TYPE {}", c.data_type)));
        }
        if old.nullable != c.nullable {
            let verb = if c.nullable { "DROP" } else { "SET" };
            out.push(alter(format!("{col} {verb} NOT NULL")));
        }
        if old.default != c.default {
            out.push(alter(match &c.default {
                Some(d) => format!("{col} SET DEFAULT {d}"),
                None => format!("{col} DROP DEFAULT"),
            }));
        }
        if old.identity != c.identity {
            out.push(alter(match c.identity {
                true => format!("{col} ADD GENERATED BY DEFAULT AS IDENTITY"),
                false => format!("{col} DROP IDENTITY"),
            }));
        }
    }
    for (_, c) in pairs.iter().filter(|(old, _)| old.is_none()) {
        out.push(alter(format!("ADD COLUMN {}", column_def(c))));
    }
    let pk = pk_names(&new.columns);
    if pk_changed && !pk.is_empty() {
        out.push(alter(format!("ADD PRIMARY KEY ({})", idents(&pk))));
    }

    let new_ix = removed(&new.indexes, &before.indexes, |i| &i.name);
    out.extend(new_ix.map(|i| create_index(schema, &new.name, i)));
    let new_uniques = removed(&new.unique_keys, &before.unique_keys, |i| &i.name);
    out.extend(new_uniques.map(|u| add_unique(schema, &new.name, u)));
    let new_fks = removed(&new.foreign_keys, &before.foreign_keys, |f| &f.name);
    out.extend(new_fks.map(|f| add_fk(schema, &new.name, f)));
    out
}

/// Items of `from` that `to` lacks by name or holds in a changed form.
fn removed<'a, T: PartialEq>(
    from: &'a [T],
    to: &'a [T],
    name: fn(&T) -> &String,
) -> impl Iterator<Item = &'a T> {
    from.iter()
        .filter(move |a| to.iter().find(|b| name(b) == name(a)) != Some(*a))
}

fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", ident(schema), ident(name))
}

fn idents(names: &[String]) -> String {
    names
        .iter()
        .map(|n| ident(n))
        .collect::<Vec<_>>()
        .join(", ")
}

fn pk_names(columns: &[Column]) -> Vec<String> {
    columns
        .iter()
        .filter(|c| c.is_pk)
        .map(|c| c.name.clone())
        .collect()
}

fn column_def(c: &Column) -> String {
    let mut out = format!("{} {}", ident(&c.name), c.data_type);
    if c.identity {
        out.push_str(" GENERATED BY DEFAULT AS IDENTITY");
    }
    if !c.nullable {
        out.push_str(" NOT NULL");
    }
    if let Some(d) = &c.default {
        out.push_str(&format!(" DEFAULT {d}"));
    }
    out
}

fn create_index(schema: &str, table: &str, i: &Index) -> String {
    format!(
        "CREATE {}INDEX {} ON {} ({})",
        if i.unique { "UNIQUE " } else { "" },
        ident(&i.name),
        qualified(schema, table),
        idents(&i.columns)
    )
}

fn add_unique(schema: &str, table: &str, u: &Index) -> String {
    format!(
        "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({})",
        qualified(schema, table),
        ident(&u.name),
        idents(&u.columns)
    )
}

fn add_fk(schema: &str, table: &str, f: &ForeignKey) -> String {
    format!(
        "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {} ON UPDATE {}",
        qualified(schema, table),
        ident(&f.name),
        idents(&f.columns),
        qualified(&f.ref_schema, &f.ref_table),
        idents(&f.ref_columns),
        f.on_delete,
        f.on_update
    )
}

#[cfg(test)]
mod tests;
