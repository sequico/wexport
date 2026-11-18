//! Archive data as classic scripts: one chunk per month, split when a month is huge.
//!
//! Strings are escaped so no message text can close a `<script>` tag or break a literal, and keys
//! are sorted, which is what makes two builds of the same data byte-identical.

use std::path::Path;

use serde_json::{json, Value};

use crate::error::Result;

pub const MAX_RECORDS: usize = 20_000;

#[derive(Debug, Clone)]
pub struct Chunk {
    pub name: String,
    pub messages: usize,
    pub first_ts: i64,
    pub last_ts: i64,
}

#[derive(Debug, Clone)]
pub struct Record {
    pub month: String,
    pub ts: i64,
    pub value: Value,
}

/// Writes chunks as records arrive, so a chat never has to be held whole in memory.
pub struct Writer<'a> {
    destination: &'a Path,
    max_records: usize,
    current: Option<String>,
    buffer: Vec<Record>,
    parts: usize,
    chunks: Vec<Chunk>,
}

impl<'a> Writer<'a> {
    pub fn new(destination: &'a Path, max_records: usize) -> Result<Self> {
        std::fs::create_dir_all(destination)?;
        Ok(Writer {
            destination,
            max_records,
            current: None,
            buffer: Vec::new(),
            parts: 0,
            chunks: Vec::new(),
        })
    }

    pub fn push(&mut self, record: Record) -> Result<()> {
        match &self.current {
            Some(month) if record.month == *month && self.buffer.len() < self.max_records => {}
            Some(month) => {
                let month = month.clone();
                self.flush(&month)?;
                if record.month != month {
                    self.parts = 0;
                }
            }
            None => {}
        }
        self.current = Some(record.month.clone());
        self.buffer.push(record);
        Ok(())
    }

    /// Write the manifest and return what was emitted; consumes the writer.
    pub fn finish(mut self, mut manifest: Value) -> Result<Vec<Chunk>> {
        if let Some(month) = self.current.clone() {
            self.flush(&month)?;
        }
        let chunk_list: Vec<Value> = self
            .chunks
            .iter()
            .map(|chunk| {
                json!({ "n": chunk.name, "c": chunk.messages, "f": chunk.first_ts, "l": chunk.last_ts })
            })
            .collect();
        let total: usize = self.chunks.iter().map(|chunk| chunk.messages).sum();
        manifest["chunks"] = Value::Array(chunk_list);
        manifest["messages"] = json!(total);
        std::fs::write(
            self.destination.join("manifest.js"),
            format!(
                "WEX.manifest({});\n",
                escape(&serde_json::to_string(&manifest)?)
            ),
        )?;
        Ok(self.chunks)
    }

    fn flush(&mut self, month: &str) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let name = if self.parts == 0 {
            month.to_string()
        } else {
            format!("{month}-{}", self.parts + 1)
        };
        self.parts += 1;
        let values: Vec<&Value> = self.buffer.iter().map(|record| &record.value).collect();
        let body = escape(&serde_json::to_string(&values)?);
        std::fs::write(
            self.destination.join(format!("{name}.js")),
            format!("WEX.chunk(\"{name}\",{body});\n"),
        )?;
        self.chunks.push(Chunk {
            name,
            messages: self.buffer.len(),
            first_ts: self.buffer.first().map(|record| record.ts).unwrap_or(0),
            last_ts: self.buffer.last().map(|record| record.ts).unwrap_or(0),
        });
        self.buffer.clear();
        Ok(())
    }
}

fn escape(text: &str) -> String {
    text.replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_text_cannot_close_a_script_tag() {
        let escaped = escape("hi </script> \u{2028} end");
        assert!(escaped.contains("\\u003c/script"));
        assert!(escaped.contains("\\u2028"));
    }

    #[test]
    fn a_month_over_the_limit_is_split() {
        let destination = std::env::temp_dir().join(format!("wex-chunks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&destination);
        let mut writer = Writer::new(&destination, 3).unwrap();
        for index in 0..7 {
            writer
                .push(Record {
                    month: "2026-11".into(),
                    ts: index,
                    value: json!({ "i": index, "t": index, "k": "text" }),
                })
                .unwrap();
        }
        let chunks = writer
            .finish(json!({"title": "t", "built": "fixed"}))
            .unwrap();
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.name.as_str())
                .collect::<Vec<_>>(),
            ["2026-11", "2026-11-2", "2026-11-3"]
        );
        assert_eq!(chunks.iter().map(|chunk| chunk.messages).sum::<usize>(), 7);
        let _ = std::fs::remove_dir_all(&destination);
    }

    #[test]
    fn a_new_month_starts_its_numbering_again() {
        let destination =
            std::env::temp_dir().join(format!("wex-chunks-month-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&destination);
        let mut writer = Writer::new(&destination, 1).unwrap();
        for (month, index) in [
            ("2026-11", 0),
            ("2026-11", 1),
            ("2026-12", 2),
            ("2026-12", 3),
        ] {
            writer
                .push(Record {
                    month: month.into(),
                    ts: index,
                    value: json!({ "i": index }),
                })
                .unwrap();
        }
        let chunks = writer.finish(json!({})).unwrap();
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.name.as_str())
                .collect::<Vec<_>>(),
            ["2026-11", "2026-11-2", "2026-12", "2026-12-2"]
        );
        let _ = std::fs::remove_dir_all(&destination);
    }
}
