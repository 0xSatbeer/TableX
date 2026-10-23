//! SQL completion: from the tokens around the cursor, the keywords, schemas, objects, columns
//! and functions that fit there, ranked by the partial word under the cursor. Pure; never does IO.
#![allow(dead_code, reason = "the console editor calls this")]

mod keywords;

use std::collections::HashMap;

use crate::fuzzy;
use crate::model::{DbKind, DbObject, ObjectKind};
use crate::sql;

/// What completion knows of a connection's metadata; whatever isn't loaded yet is just absent.
#[derive(Clone, Debug, Default)]
pub struct SchemaSnapshot {
    /// Objects per schema; a schema whose objects aren't loaded maps to an empty list.
    pub objects: HashMap<String, Vec<DbObject>>,
    /// The console's schema: unqualified names resolve here first.
    pub current: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Keyword,
    Schema,
    Object(ObjectKind),
    Column,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: ItemKind,
    /// Column type, or "table", "view", "function", "schema", "keyword".
    pub detail: String,
    /// The text that replaces the partial word, quoted when the name needs it.
    pub insert: String,
}

pub fn complete(sql: &str, offset: usize, dialect: DbKind, snapshot: &SchemaSnapshot) -> Vec<Item> {
    let range = sql::at_cursor(sql, offset).unwrap_or(offset..offset);
    let start = range.start.min(offset);
    let prefix = &sql[start..offset];
    if in_string_or_comment(prefix) {
        return Vec::new();
    }
    let mut before = statement_tail(tokens(prefix));
    let partial = take_partial(&mut before, prefix.len());
    let ctx = Ctx {
        snapshot,
        dialect,
        quoted: partial.as_ref().is_some_and(|p| p.kind == TokKind::Quoted),
        refs: walk(&statement_tail(tokens(&sql[start..range.end.max(offset)]))).refs,
    };
    let candidates = match context(&before) {
        Context::Start => ctx.keywords(),
        Context::Objects => ctx.objects(),
        Context::Members(qualifier) => ctx.members(&qualifier),
        Context::Expression => ctx.expression(),
    };
    rank(candidates, partial.as_ref().map_or("", |p| p.text.as_str()))
}

fn in_string_or_comment(prefix: &str) -> bool {
    let code = sql::mask(prefix);
    let (Some(&last), Some(&raw)) = (code.as_bytes().last(), prefix.as_bytes().last()) else {
        return false;
    };
    let in_comment = last == b' ' && !raw.is_ascii_whitespace();
    let in_string = tokens(prefix)
        .last()
        .is_some_and(|t| t.kind == TokKind::Literal && t.end == prefix.len());
    in_comment || in_string
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokKind {
    Word,
    Quoted,
    Literal,
    Punct(u8),
}

#[derive(Clone, Debug)]
struct Tok {
    kind: TokKind,
    /// Quoted identifiers unquoted; everything else as written.
    text: String,
    end: usize,
}

/// Same quoting rules as `sql::split`: masked quoted runs and comments never yield punctuation.
fn tokens(text: &str) -> Vec<Tok> {
    let code = sql::mask(text);
    let masked = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < masked.len() {
        if masked[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let word = masked[i..].iter().take_while(|c| is_word_byte(**c)).count();
        let end = i + word.max(1);
        out.push(token(&text[i..end], masked[i], end));
        i = end;
    }
    out
}

fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

fn token(raw: &str, masked_first: u8, end: usize) -> Tok {
    let first = raw.as_bytes()[0];
    let (kind, text) = match first {
        b'"' => (TokKind::Quoted, unquote(raw)),
        b'\'' => (TokKind::Literal, raw.to_string()),
        b'$' if masked_first == b'_' => (TokKind::Literal, raw.to_string()),
        c if is_word_byte(c) => (TokKind::Word, raw.to_string()),
        c => (TokKind::Punct(c), raw.to_string()),
    };
    Tok { kind, text, end }
}

fn unquote(raw: &str) -> String {
    let body = &raw[1..];
    let body = body.strip_suffix('"').unwrap_or(body);
    body.replace("\"\"", "\"")
}

/// The tokens after the last `;`: the statement the cursor is in.
fn statement_tail(mut toks: Vec<Tok>) -> Vec<Tok> {
    if let Some(semi) = toks.iter().rposition(|t| t.kind == TokKind::Punct(b';')) {
        toks.drain(..=semi);
    }
    toks
}

/// The identifier the cursor sits at the end of, if any.
fn take_partial(before: &mut Vec<Tok>, cursor: usize) -> Option<Tok> {
    let last = before.last()?;
    let is_name = matches!(last.kind, TokKind::Word | TokKind::Quoted);
    (is_name && last.end == cursor)
        .then(|| before.pop())
        .flatten()
}

enum Context {
    Start,
    Objects,
    Members(String),
    Expression,
}

fn context(before: &[Tok]) -> Context {
    if before.is_empty() {
        return Context::Start;
    }
    if let [.., qualifier, dot] = before
        && dot.kind == TokKind::Punct(b'.')
        && is_name(qualifier)
    {
        return Context::Members(qualifier.text.clone());
    }
    if walk(before).expect_object {
        return Context::Objects;
    }
    Context::Expression
}

fn is_name(t: &Tok) -> bool {
    match t.kind {
        TokKind::Quoted => true,
        TokKind::Word => !keywords::is_reserved(&t.text),
        _ => false,
    }
}

#[derive(Debug)]
struct TableRef {
    schema: Option<String>,
    name: String,
    alias: Option<String>,
}

#[derive(Default)]
struct Walk {
    refs: Vec<TableRef>,
    /// The last token leaves the statement waiting for a table name.
    expect_object: bool,
}

/// Keywords that don't end a FROM list: a `,` after them still lists tables.
const FROM_LIST: &[&str] = &[
    "AS", "LEFT", "RIGHT", "INNER", "OUTER", "FULL", "CROSS", "NATURAL", "LATERAL", "ONLY",
];

fn walk(toks: &[Tok]) -> Walk {
    let mut walk = Walk::default();
    let mut in_from = false;
    let mut i = 0;
    while i < toks.len() {
        if walk.expect_object && is_name(&toks[i]) {
            let (table, next) = table_ref(toks, i);
            walk.refs.push(table);
            walk.expect_object = false;
            i = next;
            continue;
        }
        let word = toks[i].text.to_ascii_uppercase();
        walk.expect_object = match word.as_str() {
            "FROM" | "JOIN" | "UPDATE" | "INTO" => true,
            "," => in_from,
            _ => false,
        };
        in_from = match word.as_str() {
            "FROM" | "JOIN" => true,
            w if FROM_LIST.contains(&w) => in_from,
            w => in_from && !keywords::is_reserved(w),
        };
        i += 1;
    }
    walk
}

/// `[schema.]name [[AS] alias]` starting at `i`; returns it and the index after it.
fn table_ref(toks: &[Tok], i: usize) -> (TableRef, usize) {
    let dotted = toks
        .get(i + 1)
        .is_some_and(|t| t.kind == TokKind::Punct(b'.'))
        && toks.get(i + 2).is_some_and(is_name);
    let (schema, name, mut j) = match dotted {
        true => (Some(toks[i].text.clone()), toks[i + 2].text.clone(), i + 3),
        false => (None, toks[i].text.clone(), i + 1),
    };
    if toks
        .get(j)
        .is_some_and(|t| t.text.eq_ignore_ascii_case("AS"))
    {
        j += 1;
    }
    let alias = toks.get(j).filter(|t| is_name(t)).map(|t| t.text.clone());
    let next = j + usize::from(alias.is_some());
    (
        TableRef {
            schema,
            name,
            alias,
        },
        next,
    )
}

/// A candidate and its group: lower groups rank first among equal matches.
type Candidate = (u8, Item);

struct Ctx<'a> {
    snapshot: &'a SchemaSnapshot,
    dialect: DbKind,
    /// The partial word opened a `"`, so every name goes in quoted.
    quoted: bool,
    refs: Vec<TableRef>,
}

impl Ctx<'_> {
    fn keywords(&self) -> Vec<Candidate> {
        keywords::keywords(self.dialect)
            .map(|k| (2, item(k, ItemKind::Keyword, "keyword", k.to_string())))
            .collect()
    }

    fn objects(&self) -> Vec<Candidate> {
        let current = self.schema_objects(&self.snapshot.current);
        let relations = current.iter().filter(|o| is_relation(o.kind));
        let schemas = self.snapshot.objects.keys();
        relations
            .map(|o| (0, self.object(o)))
            .chain(schemas.map(|s| (1, self.named(s, ItemKind::Schema, "schema"))))
            .collect()
    }

    fn members(&self, qualifier: &str) -> Vec<Candidate> {
        if let Some(columns) = self.resolve(qualifier).and_then(|o| o.columns.as_ref()) {
            return columns
                .iter()
                .map(|c| (0, self.named(&c.name, ItemKind::Column, &c.data_type)))
                .collect();
        }
        self.schema_objects(qualifier)
            .iter()
            .filter(|o| is_relation(o.kind) || o.kind == ObjectKind::Function)
            .map(|o| (0, self.object(o)))
            .collect()
    }

    fn expression(&self) -> Vec<Candidate> {
        let columns = self
            .refs
            .iter()
            .filter_map(|r| self.lookup(r)?.columns.as_ref());
        let columns = columns
            .flatten()
            .map(|c| (0, self.named(&c.name, ItemKind::Column, &c.data_type)));
        let current = self.schema_objects(&self.snapshot.current);
        let own_functions = current.iter().filter(|o| o.kind == ObjectKind::Function);
        let function = ItemKind::Object(ObjectKind::Function);
        let builtins = keywords::functions(self.dialect)
            .map(|f| (1, item(f, function, "function", f.to_string())));
        columns
            .chain(own_functions.map(|o| (1, self.object(o))))
            .chain(builtins)
            .chain(self.keywords())
            .collect()
    }

    /// The object behind alias or table name `name` in this statement.
    fn resolve(&self, name: &str) -> Option<&DbObject> {
        let by_alias = self.refs.iter().find(|r| {
            r.alias
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(name))
        });
        let by_name = || self.refs.iter().find(|r| r.name.eq_ignore_ascii_case(name));
        self.lookup(by_alias.or_else(by_name)?)
    }

    /// Unqualified names: the current schema first, then any schema.
    fn lookup(&self, table: &TableRef) -> Option<&DbObject> {
        let find = |objects: &[DbObject]| -> Option<usize> {
            objects
                .iter()
                .position(|o| o.name.eq_ignore_ascii_case(&table.name))
        };
        let schema = table.schema.as_deref().unwrap_or(&self.snapshot.current);
        let home = self.schema_objects(schema);
        if let Some(i) = find(home) {
            return Some(&home[i]);
        }
        if table.schema.is_some() {
            return None;
        }
        let mut all = self.snapshot.objects.values();
        all.find_map(|objects| find(objects).map(|i| &objects[i]))
    }

    fn schema_objects(&self, schema: &str) -> &[DbObject] {
        let mut all = self.snapshot.objects.iter();
        all.find(|(name, _)| name.eq_ignore_ascii_case(schema))
            .map_or(&[], |(_, objects)| objects.as_slice())
    }

    fn object(&self, o: &DbObject) -> Item {
        self.named(&o.name, ItemKind::Object(o.kind), o.kind.as_str())
    }

    fn named(&self, name: &str, kind: ItemKind, detail: &str) -> Item {
        item(name, kind, detail, self.insert(name))
    }

    fn insert(&self, name: &str) -> String {
        if !self.quoted && self.is_plain(name) {
            return name.to_string();
        }
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    /// Typed bare, `name` still means this object: Postgres folds bare names to lowercase.
    fn is_plain(&self, name: &str) -> bool {
        let folds = |c: char| self.dialect == DbKind::Sqlite || !c.is_ascii_uppercase();
        let first_ok = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        let rest_ok = name
            .chars()
            .all(|c| (c.is_ascii_alphanumeric() || c == '_') && folds(c));
        first_ok && rest_ok && !keywords::is_keyword(name, self.dialect)
    }
}

fn is_relation(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
    )
}

fn item(label: &str, kind: ItemKind, detail: &str, insert: String) -> Item {
    Item {
        label: label.to_string(),
        kind,
        detail: detail.to_string(),
        insert,
    }
}

/// Case-insensitive prefix matches first, by group then name; then fuzzy subsequence matches,
/// by group then score. Duplicates (a column in two joined tables) appear once.
fn rank(candidates: Vec<Candidate>, partial: &str) -> Vec<Item> {
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .filter_map(|(group, item)| Some((sort_key(group, &item.label, partial)?, item)))
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0));
    ranked.dedup_by(|a, b| a.1.label == b.1.label && a.1.kind == b.1.kind);
    ranked.into_iter().map(|(_, item)| item).collect()
}

fn sort_key(group: u8, label: &str, partial: &str) -> Option<(bool, u8, i32, String)> {
    let prefix = label
        .get(..partial.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(partial));
    if prefix {
        return Some((false, group, 0, label.to_lowercase()));
    }
    let (score, _) = fuzzy::score(partial, label)?;
    Some((true, group, -score, label.to_lowercase()))
}

#[cfg(test)]
mod tests;
