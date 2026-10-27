//! Filter bar completion and highlighting: the console's completer and the editor's SQL
//! highlighter, run as if the box's text followed `SELECT * FROM <table> WHERE` (or `ORDER BY`),
//! since a bare fragment doesn't parse.

use gpui_kit::base::input::{
    CompletionProvider, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter,
    InputHighlighterFactory, Rope,
};
use gpui_kit::component::highlighter::SyntaxHighlighter;
use gpui_kit::component::input::RopeExt as _;
use lsp_types::{CompletionContext, CompletionItem, CompletionResponse};

use super::*;
use crate::complete::{self, SchemaSnapshot};
use crate::ddl::ident;
use crate::model::{Column, DbKind, DbObject, ObjectKind};
use crate::ui::console::completion::{lsp_item, partial_start};

pub(super) struct Provider {
    pub(super) grid: WeakEntity<Grid>,
    /// `"WHERE"` or `"ORDER BY"`.
    pub(super) clause: &'static str,
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
        let items = self
            .grid
            .upgrade()
            .map(|g| items(g.read(cx), self.clause, text, offset))
            .unwrap_or_default();
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        let mut chars = new_text.chars();
        let typed = chars.next().filter(|_| chars.next().is_none());
        typed.is_some_and(|c| !matches!(c, '\n' | ';' | '\''))
    }
}

fn items(grid: &Grid, clause: &str, text: &Rope, offset: usize) -> Vec<CompletionItem> {
    let Some((table, _, _)) = grid.source() else {
        return Vec::new();
    };
    let schema = (table.schema.as_str(), table.name.as_str(), table.dialect);
    filter_items(schema, grid.all.columns(), clause, text, offset)
}

/// Completions at `offset` in filter box `clause` of table (schema, name, dialect) with `columns`.
pub(super) fn filter_items(
    (schema, name, dialect): (&str, &str, DbKind),
    columns: &[ColumnMeta],
    clause: &str,
    text: &Rope,
    offset: usize,
) -> Vec<CompletionItem> {
    let filter = text.to_string();
    let head = format!("SELECT * FROM {} {clause} ", ident(name));
    let sql = format!("{head}{filter}");
    let snapshot = SchemaSnapshot {
        objects: [(schema.to_string(), vec![object(name, columns)])].into(),
        current: schema.to_string(),
    };
    let start = partial_start(&filter, offset);
    let range = lsp_types::Range {
        start: text.offset_to_position(start),
        end: text.offset_to_position(offset),
    };
    let partial = &filter[start..offset];
    complete::complete(&sql, head.len() + offset, dialect, &snapshot)
        .into_iter()
        .map(|i| lsp_item(i, range, partial))
        .collect()
}

fn object(name: &str, columns: &[ColumnMeta]) -> DbObject {
    let column = |c: &ColumnMeta| Column {
        name: c.name.to_string(),
        data_type: c.data_type.to_string(),
        nullable: true,
        default: None,
        is_pk: c.is_pk,
        enum_values: c.enum_values.iter().map(|v| v.to_string()).collect(),
        identity: false,
    };
    DbObject {
        kind: ObjectKind::Table,
        name: name.to_string(),
        columns: Some(columns.iter().map(column).collect()),
        keys: Vec::new(),
        indexes: Vec::new(),
    }
}

/// Highlights a filter box's text as the tail of `SELECT * FROM t <clause>`.
pub(super) fn highlighter(clause: &'static str) -> InputHighlighterFactory {
    Rc::new(move |_| {
        let head = format!("SELECT * FROM t {clause} ");
        let inner = SyntaxHighlighter::new("sql");
        Some(Box::new(Highlighter { head, inner }) as Box<dyn InputHighlighter>)
    })
}

struct Highlighter {
    head: String,
    inner: SyntaxHighlighter,
}

impl InputHighlighter for Highlighter {
    fn language(&self) -> SharedString {
        "sql".into()
    }

    fn update(
        &mut self,
        _: Option<InputEdit>,
        text: &Rope,
        _: bool,
        _: &mut Window,
        _: &mut Context<EditorState>,
    ) {
        // One line: a full reparse is cheaper than shifting each edit past the head.
        let sql = Rope::from(format!("{}{text}", self.head));
        self.inner.update(None, &sql, None);
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let at = self.head.len();
        let styles = self
            .inner
            .styles(&(range.start + at..range.end + at), resolver);
        styles
            .into_iter()
            .map(|(r, style)| (r.start.saturating_sub(at)..r.end.saturating_sub(at), style))
            .collect()
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}
