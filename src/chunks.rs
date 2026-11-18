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

pub fn emit(
    mut records: Vec<Record>,
    destination: &Path,
    mut manifest: Value,
    max_records: usize,
) -> Result<Vec<Chunk>> {
    std::fs::create_dir_all(destination)?;
    let mut chunks = Vec::new();
    let mut current: Option<String> = None;
    let mut buffer: Vec<Record> = Vec::new();
    let mut parts: usize = 0;

    let flush = |month: &str,
                 buffer: &mut Vec<Record>,
                 parts: &mut usize,
                 chunks: &mut Vec<Chunk>|
     -> Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }
        let name = if *parts == 0 {
            month.to_string()
        } else {
            format!("{month}-{}", *parts + 1)
        };
        *parts += 1;
        let values: Vec<&Value> = buffer.iter().map(|record| &record.value).collect();
        let body = escape(&serde_json::to_string(&values)?);
        std::fs::write(
            destination.join(format!("{name}.js")),
            format!("WEX.chunk(\"{name}\",{body});\n"),
        )?;
        chunks.push(Chunk {
            name,
            messages: buffer.len(),
            first_ts: buffer.first().map(|record| record.ts).unwrap_or(0),
            last_ts: buffer.last().map(|record| record.ts).unwrap_or(0),
        });
        buffer.clear();
        Ok(())
    };

    for record in records.drain(..) {
        match &current {
            Some(month) if record.month == *month && buffer.len() < max_records => {}
            Some(month) => {
                flush(month, &mut buffer, &mut parts, &mut chunks)?;
                if record.month != *month {
                    parts = 0;
                }
            }
            None => {}
        }
        current = Some(record.month.clone());
        buffer.push(record);
    }
    if let Some(month) = &current {
        flush(month, &mut buffer, &mut parts, &mut chunks)?;
    }

    let chunk_list: Vec<Value> = chunks
        .iter()
        .map(|chunk| json!({ "n": chunk.name, "c": chunk.messages, "f": chunk.first_ts, "l": chunk.last_ts }))
        .collect();
    let total: usize = chunks.iter().map(|chunk| chunk.messages).sum();
    manifest["chunks"] = Value::Array(chunk_list);
    manifest["messages"] = json!(total);
    std::fs::write(
        destination.join("manifest.js"),
        format!(
            "WEX.manifest({});\n",
            escape(&serde_json::to_string(&manifest)?)
        ),
    )?;
    Ok(chunks)
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
        let records: Vec<Record> = (0..7)
            .map(|index| Record {
                month: "2026-11".into(),
                ts: index,
                value: json!({ "i": index, "t": index, "k": "text" }),
            })
            .collect();
        let chunks = emit(
            records,
            &destination,
            json!({"title": "t", "built": "fixed"}),
            3,
        )
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
}
