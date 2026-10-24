//! Result formats, written to any `Write` a batch of rows at a time, and streaming a whole
//! query to a file. Copy writes one batch to a string; export streams batches to disk.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::db::{Driver, Opened, Session};
use crate::model::{ColumnMeta, Value};

/// Rows each `fetch` of a streaming export asks for.
pub const BATCH: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Csv,
    Tsv,
    Json,
    SqlInsert,
    Markdown,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::Csv,
        Format::Tsv,
        Format::Json,
        Format::SqlInsert,
        Format::Markdown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Tsv => "TSV",
            Format::Json => "JSON",
            Format::SqlInsert => "SQL INSERT",
            Format::Markdown => "Markdown",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Tsv => "tsv",
            Format::Json => "json",
            Format::SqlInsert => "sql",
            Format::Markdown => "md",
        }
    }
}

/// Writes `format`: `header` once, `rows` per batch, then `finish` for the footer.
pub struct Writer<W: Write> {
    format: Format,
    out: W,
    columns: Vec<ColumnMeta>,
    /// What SQL INSERT inserts into, as written into the statement.
    table: String,
    pub written: usize,
}

impl<W: Write> Writer<W> {
    pub fn new(format: Format, out: W, columns: Vec<ColumnMeta>, table: &str) -> Self {
        Self {
            format,
            out,
            columns,
            table: table.to_string(),
            written: 0,
        }
    }

    pub fn header(&mut self) -> io::Result<()> {
        let names = self.columns.iter().map(|c| c.name.to_string());
        match self.format {
            Format::Csv => {
                let line: Vec<String> = names.map(csv_field).collect();
                writeln!(self.out, "{}", line.join(","))
            }
            Format::Tsv => {
                let line: Vec<String> = names.map(tsv_field).collect();
                writeln!(self.out, "{}", line.join("\t"))
            }
            Format::Json => self.out.write_all(b"[\n"),
            Format::SqlInsert => Ok(()),
            Format::Markdown => {
                let n = self.columns.len();
                write!(self.out, "{}", md_line(names.map(md_cell).collect()))?;
                write!(self.out, "{}", md_line(vec!["---".to_string(); n]))
            }
        }
    }

    pub fn rows<R: AsRef<[Value]>>(&mut self, rows: &[R]) -> io::Result<()> {
        for row in rows {
            let line = self.line(row.as_ref());
            self.out.write_all(line.as_bytes())?;
            self.written += 1;
        }
        Ok(())
    }

    /// Writes the footer and hands back the output, flushed.
    pub fn finish(mut self) -> io::Result<W> {
        if self.format == Format::Json {
            self.out.write_all(b"\n]\n")?;
        }
        self.out.flush()?;
        Ok(self.out)
    }

    fn line(&self, row: &[Value]) -> String {
        let text = |v: &Value| match v {
            Value::Null => String::new(),
            v => v.to_string(),
        };
        match self.format {
            Format::Csv => {
                let fields: Vec<String> = row.iter().map(|v| csv_field(text(v))).collect();
                fields.join(",") + "\n"
            }
            Format::Tsv => {
                let fields: Vec<String> = row.iter().map(|v| tsv_field(text(v))).collect();
                fields.join("\t") + "\n"
            }
            Format::Json => {
                let fields: Vec<String> = self
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(c, v)| format!("{}: {}", json_string(&c.name), json_value(v)))
                    .collect();
                let sep = if self.written > 0 { ",\n" } else { "" };
                format!("{sep}  {{{}}}", fields.join(", "))
            }
            Format::SqlInsert => {
                let names: Vec<&str> = self.columns.iter().map(|c| c.name.as_ref()).collect();
                let values: Vec<String> = row.iter().map(sql_literal).collect();
                format!(
                    "INSERT INTO {} ({}) VALUES ({});\n",
                    self.table,
                    names.join(", "),
                    values.join(", ")
                )
            }
            Format::Markdown => md_line(row.iter().map(|v| md_cell(v.to_string())).collect()),
        }
    }
}

/// Spreadsheet-pasteable TSV: fields with tabs, newlines or quotes are quoted with doubled
/// quotes, which Sheets/Excel/LibreOffice all read back.
fn tsv_field(s: String) -> String {
    if s.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

fn csv_field(s: String) -> String {
    if s.contains([',', '\n', '\r', '"']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

fn md_cell(s: String) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn md_line(cells: Vec<String>) -> String {
    format!("| {} |\n", cells.join(" | "))
}

pub fn json_string(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if c.is_control() => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

pub fn json_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) | Value::Int(_) => v.to_string(),
        Value::Float(x) if x.is_finite() => v.to_string(),
        Value::Json(s) => s.to_string(),
        v => json_string(&v.to_string()),
    }
}

pub fn sql_literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        Value::Int(_) => v.to_string(),
        Value::Float(x) if x.is_finite() => v.to_string(),
        v => format!("'{}'", v.to_string().replace('\'', "''")),
    }
}

/// One export of a whole query, on a session of its own.
pub struct Job {
    pub driver: Arc<dyn Driver>,
    pub session: Session,
    pub sql: String,
    pub format: Format,
    pub table: String,
    pub path: PathBuf,
    /// Set by Cancel; checked after every batch.
    pub cancelled: Arc<AtomicBool>,
}

/// Streams `job.sql` to `job.path` in `BATCH`-row fetches, calling `progress` with the rows
/// written after each, and closes the result. Once the file is created, any failure, Cancel or
/// drop of this future removes it; a file that was there before then is left alone.
pub async fn run(job: Job, progress: impl FnMut(usize)) -> Result<usize, String> {
    let result = stream(&job, progress).await;
    _ = job.driver.close(&job.session).await;
    result
}

/// A file this export created, removed when dropped unless `keep` is called.
pub struct Partial(Option<PathBuf>);

impl Partial {
    pub fn create(path: &Path) -> Result<(File, Partial), String> {
        let file = File::create(path).map_err(|e| e.to_string())?;
        Ok((file, Partial(Some(path.to_path_buf()))))
    }

    pub fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            _ = std::fs::remove_file(path);
        }
    }
}

async fn stream(job: &Job, mut progress: impl FnMut(usize)) -> Result<usize, String> {
    let cancelled = || job.cancelled.load(Ordering::Relaxed);
    if cancelled() {
        return Err("Cancelled".into());
    }
    let opened = job.driver.open(&job.session, job.sql.clone(), true).await;
    let Opened::Rows(columns) = opened.map_err(|e| e.message)? else {
        return Err("The statement returns no rows".into());
    };
    let (file, partial) = Partial::create(&job.path)?;
    let mut out = Writer::new(job.format, BufWriter::new(file), columns, &job.table);
    out.header().map_err(|e| e.to_string())?;
    loop {
        let rows = job.driver.fetch(&job.session, BATCH).await;
        if cancelled() {
            return Err("Cancelled".into());
        }
        let rows = rows.map_err(|e| e.message)?;
        if !rows.is_empty() {
            out.rows(&rows).map_err(|e| e.to_string())?;
            progress(out.written);
        }
        if rows.len() < BATCH {
            break;
        }
    }
    let written = out.written;
    out.finish().map_err(|e| e.to_string())?;
    partial.keep();
    Ok(written)
}

#[cfg(test)]
mod tests;
