//! Console completion: a `CompletionProvider` over `complete::complete`, fed by a per-database
//! schema cache that fills in the background so a completion never waits on IO.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::FutureExt as _;
use gpui_kit::base::input::{CompletionProvider, Rope};
use gpui_kit::component::input::RopeExt as _;
use gpui_kit::*;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    TextEdit,
};

use super::{Console, Target};
use crate::complete::{self, Item, ItemKind, SchemaSnapshot};
use crate::db::{Driver, Reply, Session};
use crate::model::{Connection, ConnectionId, DbKind, DbObject, ObjectKind};
use crate::sessions::Sessions;
use crate::sql;

/// Connection, database, schema.
type Place = (ConnectionId, String, String);

/// What the console editors complete from, per schema.
#[derive(Default)]
pub struct Cache {
    /// Objects per schema; columns fill in as they arrive.
    schemas: HashMap<Place, Vec<DbObject>>,
    /// Cleared schemas: read from the server again, never re-seeded from the explorer's model.
    stale: HashSet<Place>,
    /// Requests in flight, by schema and table (`None`: the schema's objects), so each goes once.
    pending: HashSet<(Place, Option<String>)>,
    /// The sessions the requests run on.
    sessions: Option<WeakEntity<Sessions>>,
}

impl Global for Cache {}

/// The sessions the cache reads through from now on.
pub fn set_sessions(sessions: WeakEntity<Sessions>, cx: &mut App) {
    cx.default_global::<Cache>().sessions = Some(sessions);
}

/// Forgets the schemas under tree path `path` (connection id, then database and schema names).
pub fn invalidate(path: &[String], cx: &mut App) {
    let under = |(id, db, schema): &Place| {
        let place = [id.0.to_string(), db.clone(), schema.clone()];
        path.iter().zip(&place).all(|(a, b)| a == b)
    };
    let cache = cx.default_global::<Cache>();
    let cleared: Vec<Place> = cache.schemas.keys().filter(|p| under(p)).cloned().collect();
    cache.schemas.retain(|p, _| !under(p));
    cache.pending.retain(|(p, _)| !under(p));
    cache.stale.extend(cleared);
}

/// Whether `stmt` changes the schema.
pub fn is_ddl(stmt: &str) -> bool {
    let code = sql::mask(stmt).to_ascii_uppercase();
    let first = code.split_whitespace().next();
    matches!(first, Some("CREATE" | "ALTER" | "DROP"))
}

pub struct Provider {
    pub console: WeakEntity<Console>,
}

impl CompletionProvider for Provider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let Some(console) = self.console.upgrade() else {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        };
        let target = console.read(cx).target().clone();
        let items = items(&target, text, offset, cx);
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        let mut chars = new_text.chars();
        let typed = chars.next().filter(|_| chars.next().is_none());
        typed.is_some_and(|c| !matches!(c, '\n' | ';' | '\''))
    }
}

fn items(target: &Target, text: &Rope, offset: usize, cx: &mut App) -> Vec<CompletionItem> {
    let Some(dialect) = connection(target, cx).map(Connection::kind) else {
        return Vec::new();
    };
    let sql = text.to_string();
    let snapshot = snapshot(target, &statement_words(&sql, offset), cx);
    let start = partial_start(&sql, offset);
    let range = lsp_types::Range {
        start: text.offset_to_position(start),
        end: text.offset_to_position(offset),
    };
    let partial = &sql[start..offset];
    let items = complete::complete(&sql, offset, dialect, &snapshot);
    items
        .into_iter()
        .map(|i| lsp_item(i, range, partial))
        .collect()
}

fn connection<'a>(target: &Target, cx: &'a App) -> Option<&'a Connection> {
    let id = target.connection?;
    let ws = target.workspace.read(cx);
    ws.connections.iter().find(|c| c.id == id)
}

/// The words of the statement at `offset`, lowercased: the tables whose columns may be needed.
fn statement_words(sql: &str, offset: usize) -> HashSet<String> {
    let range = sql::at_cursor(sql, offset).unwrap_or(0..sql.len());
    let words = sql[range].split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'));
    words
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Where the identifier the cursor ends starts, an opening `"` included.
pub(crate) fn partial_start(sql: &str, offset: usize) -> usize {
    let word = |c: char| c.is_alphanumeric() || matches!(c, '_' | '$' | '"');
    let back = sql[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| word(*c));
    back.last().map_or(offset, |(i, _)| i)
}

pub(crate) fn lsp_item(item: Item, range: lsp_types::Range, partial: &str) -> CompletionItem {
    CompletionItem {
        label: item.label,
        kind: Some(lsp_kind(item.kind)),
        detail: (item.kind != ItemKind::Keyword).then_some(item.detail),
        filter_text: Some(partial.to_string()),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: item.insert,
        })),
        ..Default::default()
    }
}

fn lsp_kind(kind: ItemKind) -> CompletionItemKind {
    match kind {
        ItemKind::Keyword => CompletionItemKind::KEYWORD,
        ItemKind::Schema => CompletionItemKind::MODULE,
        ItemKind::Column => CompletionItemKind::FIELD,
        ItemKind::Object(ObjectKind::Function | ObjectKind::Procedure) => {
            CompletionItemKind::FUNCTION
        }
        ItemKind::Object(ObjectKind::Table) => CompletionItemKind::STRUCT,
        ItemKind::Object(ObjectKind::View | ObjectKind::MaterializedView) => {
            CompletionItemKind::INTERFACE
        }
        ItemKind::Object(_) => CompletionItemKind::VALUE,
    }
}

/// What completion knows of the console's database now. The console's schema, when not cached,
/// and the uncached columns of its relations named in `words` are requested in the background.
pub fn snapshot(target: &Target, words: &HashSet<String>, cx: &mut App) -> SchemaSnapshot {
    let Some(conn) = connection(target, cx).cloned() else {
        return SchemaSnapshot::default();
    };
    let current = target
        .schema
        .clone()
        .unwrap_or_else(|| default_schema(&conn));
    let place = |schema: &str| (conn.id, target.database.clone(), schema.to_string());
    let mut names = model_schemas(&conn, &target.database);
    if !names.iter().any(|(n, _)| *n == current) {
        names.push((current.clone(), None));
    }
    let cache = cx.default_global::<Cache>();
    for (name, objects) in &names {
        cache.seed(place(name), objects);
    }
    let fetch = session(target, cx).map(|s| (target.driver.clone(), s));
    if let Some(fetch) = fetch {
        request_missing(&place(&current), words, fetch, cx);
    }
    let cache = cx.default_global::<Cache>();
    let objects = names.into_iter().map(|(name, _)| {
        let objects = cache.schemas.get(&place(&name)).cloned();
        (name, objects.unwrap_or_default())
    });
    SchemaSnapshot {
        objects: objects.collect(),
        current,
    }
}

fn default_schema(conn: &Connection) -> String {
    match conn.kind() {
        DbKind::Postgres => "public".into(),
        DbKind::Sqlite => "main".into(),
    }
}

/// The schemas the explorer has listed for `database`, with their objects when loaded.
fn model_schemas(conn: &Connection, database: &str) -> Vec<(String, Option<Vec<DbObject>>)> {
    let databases = conn.databases.iter().flatten();
    let db = databases.into_iter().find(|d| d.name == database);
    let schemas = db.and_then(|d| d.schemas.as_ref()).into_iter().flatten();
    schemas
        .map(|s| (s.name.clone(), s.objects.clone()))
        .collect()
}

impl Cache {
    /// Takes `objects` from the explorer's model for a schema never cached nor cleared.
    fn seed(&mut self, place: Place, objects: &Option<Vec<DbObject>>) {
        let Some(objects) = objects else {
            return;
        };
        if self.schemas.contains_key(&place) || self.stale.contains(&place) {
            return;
        }
        self.schemas.insert(place, objects.clone());
    }
}

fn session(target: &Target, cx: &App) -> Option<Session> {
    let sessions = cx.try_global::<Cache>()?.sessions.as_ref()?.upgrade()?;
    sessions.read(cx).open(target.connection?, &target.database)
}

type Fetch = (Arc<dyn Driver>, Session);

fn request_missing(place: &Place, words: &HashSet<String>, fetch: Fetch, cx: &mut App) {
    let cache = cx.default_global::<Cache>();
    let Some(objects) = cache.schemas.get(place) else {
        return request(place.clone(), None, fetch, cx);
    };
    let uncached = objects.iter().filter(|o| {
        let relation = matches!(
            o.kind,
            ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
        );
        relation && o.columns.is_none() && words.contains(&o.name.to_lowercase())
    });
    let tables: Vec<String> = uncached.map(|o| o.name.clone()).collect();
    for table in tables {
        request(place.clone(), Some(table), fetch.clone(), cx);
    }
}

type Fill = Box<dyn FnOnce(&mut Vec<DbObject>) + Send>;

/// Reads `table`'s columns, or the schema's objects without one, unless that is already asked.
fn request(place: Place, table: Option<String>, (driver, session): Fetch, cx: &mut App) {
    let key = (place, table);
    if !cx.default_global::<Cache>().pending.insert(key.clone()) {
        return;
    }
    let schema = &key.0.2;
    let reply: Reply<Fill> = match key.1.clone() {
        None => Box::pin(driver.objects(&session, schema).map(|r| {
            r.map(|objects| Box::new(move |all: &mut Vec<DbObject>| *all = objects) as Fill)
        })),
        Some(table) => Box::pin(driver.columns(&session, schema, &table).map(move |r| {
            r.map(|columns| {
                Box::new(move |all: &mut Vec<DbObject>| fill(all, &table, columns)) as Fill
            })
        })),
    };
    cx.spawn(async move |cx| {
        let result = reply.await;
        cx.update(|cx| arrived(key, result, cx));
    })
    .detach();
}

fn fill(objects: &mut [DbObject], table: &str, columns: Vec<crate::model::Column>) {
    if let Some(object) = objects.iter_mut().find(|o| o.name == table) {
        object.columns = Some(columns);
    }
}

/// Stores a reply, unless its schema was cleared since it was asked.
fn arrived(key: (Place, Option<String>), result: Result<Fill, String>, cx: &mut App) {
    let cache = cx.default_global::<Cache>();
    if !cache.pending.remove(&key) {
        return;
    }
    let Ok(fill) = result else {
        return;
    };
    let (place, table) = key;
    if table.is_none() {
        cache.stale.remove(&place);
    }
    fill(cache.schemas.entry(place).or_default());
}
