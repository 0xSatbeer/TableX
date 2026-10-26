//! The one-line summaries the editor's lists show, with the `highlight.syntax` key of each span.

use std::ops::Range;

use crate::model::{Column, ForeignKey, Index};
use crate::ui::panel::sql::sql_spans;

pub type Spans = Vec<(Range<usize>, &'static str)>;

#[derive(Default)]
struct Line {
    text: String,
    spans: Spans,
}

impl Line {
    fn push(&mut self, text: &str, key: Option<&'static str>) -> &mut Self {
        let start = self.text.len();
        self.text.push_str(text);
        if let Some(key) = key.filter(|_| !text.is_empty()) {
            self.spans.push((start..self.text.len(), key));
        }
        self
    }

    /// `sql` with its strings, numbers and keywords colored.
    fn sql(&mut self, sql: &str) -> &mut Self {
        let start = self.text.len();
        self.text.push_str(sql);
        self.spans.extend(
            sql_spans(sql)
                .into_iter()
                .map(|(r, key)| (r.start + start..r.end + start, key)),
        );
        self
    }

    /// `name` padded to `pad` characters, so what follows lines up across rows.
    fn name(&mut self, name: &str, pad: usize) -> &mut Self {
        if name.is_empty() {
            return self.push(&format!("{:pad$} ", "(unnamed)"), Some("comment"));
        }
        self.push(&format!("{name:pad$} "), None)
    }

    fn done(&mut self) -> (String, Spans) {
        (
            std::mem::take(&mut self.text),
            std::mem::take(&mut self.spans),
        )
    }
}

/// The width the names of a list pad to.
pub fn pad<'a>(names: impl Iterator<Item = &'a str>) -> usize {
    names
        .map(|n| if n.is_empty() { "(unnamed)" } else { n }.chars().count())
        .max()
        .unwrap_or(0)
}

/// `id  integer default nextval('s') not null identity unique -- part of primary key`.
pub fn column_line(c: &Column, unique: bool, pad: usize) -> (String, Spans) {
    let mut line = Line::default();
    line.name(&c.name, pad).push(&c.data_type, Some("type"));
    if let Some(default) = &c.default {
        line.push(" default ", Some("keyword")).sql(default);
    }
    if !c.nullable {
        line.push(" not null", Some("keyword"));
    }
    if c.identity {
        line.push(" identity", Some("keyword"));
    }
    if unique {
        line.push(" unique", Some("keyword"));
    }
    if c.is_pk {
        line.push(" -- part of primary key", Some("comment"));
    }
    line.done()
}

/// `t_a_idx  unique (a, b)`.
pub fn index_line(i: &Index, pad: usize) -> (String, Spans) {
    let mut line = Line::default();
    line.name(&i.name, pad);
    if i.unique {
        line.push("unique ", Some("keyword"));
    }
    line.push(&format!("({})", i.columns.join(", ")), None)
        .done()
}

/// `t_ab_key  unique (a, b) -- constraint`: a UNIQUE constraint.
pub fn constraint_line(u: &Index, pad: usize) -> (String, Spans) {
    let (mut text, mut spans) = index_line(u, pad);
    let start = text.len();
    text.push_str(" -- constraint");
    spans.push((start..text.len(), "comment"));
    (text, spans)
}

/// `t_b_fk  (b) references public.users (id) on delete cascade`.
pub fn fk_line(f: &ForeignKey, pad: usize) -> (String, Spans) {
    let mut line = Line::default();
    line.name(&f.name, pad)
        .push(&format!("({})", f.columns.join(", ")), None)
        .push(" references ", Some("keyword"))
        .push(
            &format!(
                "{}.{} ({})",
                f.ref_schema,
                f.ref_table,
                f.ref_columns.join(", ")
            ),
            None,
        );
    for (label, action) in [(" on delete ", &f.on_delete), (" on update ", &f.on_update)] {
        if action != "NO ACTION" {
            line.push(label, Some("keyword"))
                .push(&action.to_lowercase(), Some("keyword"));
        }
    }
    line.done()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed((text, spans): &(String, Spans)) -> Vec<(&str, &'static str)> {
        spans.iter().map(|(r, k)| (&text[r.clone()], *k)).collect()
    }

    fn column(name: &str, ty: &str, nullable: bool, default: Option<&str>, is_pk: bool) -> Column {
        Column {
            name: name.into(),
            data_type: ty.into(),
            nullable,
            default: default.map(String::from),
            is_pk,
            enum_values: Vec::new(),
            identity: false,
        }
    }

    #[test]
    fn column_lines_pad_the_name_and_color_type_default_and_pk() {
        let mut flagged = column("n", "int", false, None, true);
        flagged.identity = true;
        // (column, unique, text, colored spans)
        let cases: [(Column, bool, &str, &[(&str, &str)]); 5] = [
            (
                column("id", "integer", false, Some("nextval('s'::regclass)"), true),
                false,
                "id    integer default nextval('s'::regclass) not null -- part of primary key",
                &[
                    ("integer", "type"),
                    (" default ", "keyword"),
                    ("'s'", "string"),
                    (" not null", "keyword"),
                    (" -- part of primary key", "comment"),
                ],
            ),
            (
                column("code", "varchar(255)", true, None, false),
                false,
                "code  varchar(255)",
                &[("varchar(255)", "type")],
            ),
            (
                column("", "", true, None, false),
                false,
                "(unnamed) ",
                &[("(unnamed) ", "comment")],
            ),
            (
                column("n", "int", true, Some("1"), false),
                false,
                "n     int default 1",
                &[("int", "type"), (" default ", "keyword"), ("1", "number")],
            ),
            (
                flagged,
                true,
                "n     int not null identity unique -- part of primary key",
                &[
                    ("int", "type"),
                    (" not null", "keyword"),
                    (" identity", "keyword"),
                    (" unique", "keyword"),
                    (" -- part of primary key", "comment"),
                ],
            ),
        ];
        for (c, unique, text, spans) in cases {
            let line = column_line(&c, unique, 5);
            assert_eq!(line.0, text);
            assert_eq!(keyed(&line), spans, "{text}");
        }
    }

    #[test]
    fn index_and_fk_lines_show_unique_and_non_default_actions() {
        let i = Index {
            name: "t_ab".into(),
            columns: vec!["a".into(), "b".into()],
            unique: true,
        };
        assert_eq!(index_line(&i, 4).0, "t_ab unique (a, b)");
        let line = constraint_line(&i, 4);
        assert_eq!(line.0, "t_ab unique (a, b) -- constraint");
        assert_eq!(keyed(&line)[1], (" -- constraint", "comment"));
        let f = ForeignKey {
            name: "t_b_fk".into(),
            columns: vec!["b".into()],
            ref_schema: "public".into(),
            ref_table: "users".into(),
            ref_columns: vec!["id".into()],
            on_delete: "CASCADE".into(),
            on_update: "NO ACTION".into(),
        };
        assert_eq!(
            fk_line(&f, 6).0,
            "t_b_fk (b) references public.users (id) on delete cascade"
        );
        assert_eq!(pad(["a", "", "abc"].into_iter()), 9);
    }
}
