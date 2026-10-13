//! Pure DDL builders: turn the object dialogs' input into the Postgres statements to run.
//! Type and default text is SQL the user typed, so it passes through raw.
#![allow(dead_code, reason = "the object dialogs call these")]

use crate::model::{Column, ForeignKey, Index, TableDef};

/// A table as edited: `origins[i]` is the original name of `def.columns[i]`, `None` if new.
#[derive(Clone, Debug, PartialEq)]
pub struct TableEdit {
    pub def: TableDef,
    pub origins: Vec<Option<String>>,
}

pub fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
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
    let columns: Vec<String> = def.columns.iter().map(column_def).collect();
    let create = format!(
        "CREATE TABLE {} ({})",
        qualified(schema, &def.name),
        columns.join(", ")
    );
    let indexes = def
        .indexes
        .iter()
        .map(|i| create_index(schema, &def.name, i));
    let fks = def
        .foreign_keys
        .iter()
        .map(|f| add_fk(schema, &def.name, f));
    std::iter::once(create).chain(indexes).chain(fks).collect()
}

/// Statements in a fixed order: rename table, drop FKs, drop indexes, drop columns, rename
/// columns, alter columns, add columns, create indexes, add FKs. An edited index or FK is
/// dropped and recreated.
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
    let gone_ix = removed(&before.indexes, &new.indexes, |i| &i.name);
    out.extend(gone_ix.map(|i| format!("DROP INDEX {}", qualified(schema, &i.name))));

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
    }
    for (_, c) in pairs.iter().filter(|(old, _)| old.is_none()) {
        out.push(alter(format!("ADD COLUMN {}", column_def(c))));
    }

    let new_ix = removed(&new.indexes, &before.indexes, |i| &i.name);
    out.extend(new_ix.map(|i| create_index(schema, &new.name, i)));
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

fn column_def(c: &Column) -> String {
    let mut out = format!("{} {}", ident(&c.name), c.data_type);
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
