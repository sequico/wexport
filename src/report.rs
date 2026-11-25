//! What the command tells a person, and the same thing as events a script can read.
//!
//! The human rendering and the JSON lines come from the same event, so the two never drift.

use std::io::{IsTerminal, Write};

use serde_json::{json, Value};

use crate::progress::Bar;

/// One thing that happened, in the vocabulary `-j` publishes.
#[derive(Debug)]
pub enum Event {
    Found {
        source: &'static str,
        path: String,
        chats: usize,
    },
    ChatList(Vec<ChatRow>),
    Progress {
        label: String,
        done: usize,
        total: usize,
    },
    Diagnostic(String),
    Warning(String),
    Failure(String),
    Finished(Finished),
}

#[derive(Debug, Clone)]
pub struct ChatRow {
    pub code: usize,
    pub name: String,
    pub messages: i64,
    pub media: i64,
    pub bytes: i64,
    pub last: String,
}

#[derive(Debug, Default)]
pub struct Finished {
    pub archives: usize,
    pub skipped: usize,
    pub messages: usize,
    pub media: usize,
    /// Media the built chats name but the copy does not hold; `None` when the run did not look.
    pub missing: Option<usize>,
    pub failures: usize,
    pub bytes: u64,
    pub path: String,
    pub seconds: f64,
}

/// Where the words go: a terminal in the user's language, or JSON lines on stdout.
pub struct Report {
    json: bool,
    quiet: bool,
    diagnostics: bool,
    started: std::time::Instant,
}

impl Report {
    pub fn new(json: bool, quiet: bool, diagnostics: bool) -> Self {
        Report {
            json,
            quiet,
            diagnostics,
            started: std::time::Instant::now(),
        }
    }

    pub fn json(&self) -> bool {
        self.json
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

    pub fn diagnostics(&self) -> bool {
        self.diagnostics
    }

    pub fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Chatter that `-q` silences. In JSON mode it is not an event: only the listed kinds are.
    pub fn info(&self, message: impl AsRef<str>) {
        if !self.json && !self.quiet {
            println!("{}", message.as_ref());
        }
    }

    /// What `-d` adds to a run.
    pub fn detail(&self, message: impl AsRef<str>) {
        if !self.diagnostics {
            return;
        }
        if self.json {
            self.emit(Event::Diagnostic(message.as_ref().to_string()));
        } else {
            eprintln!("debug: {}", message.as_ref());
        }
    }

    /// The progress bar draws only on a terminal, and only when the human output is wanted.
    pub fn bar_enabled(&self) -> bool {
        !self.json && !self.quiet && std::io::stdout().is_terminal()
    }

    pub fn bar(&self, label: &str, total: usize) -> Bar {
        Bar::new(label, total, self.bar_enabled())
    }

    pub fn emit(&self, event: Event) {
        if self.json {
            let value = event_value(&event);
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{value}");
            let _ = stdout.flush();
            return;
        }
        match event {
            // The human rendering of progress is the bar in `progress.rs`; a piped run has none,
            // which is the point.
            Event::Found { .. }
            | Event::ChatList(_)
            | Event::Finished(_)
            | Event::Progress { .. }
            | Event::Diagnostic(_) => {}
            Event::Warning(message) => eprintln!("warning: {message}"),
            Event::Failure(message) => eprintln!("{message}"),
        }
    }
}

fn event_value(event: &Event) -> Value {
    match event {
        Event::Found {
            source,
            path,
            chats,
        } => json!({
            "kind": "found",
            "source": source,
            "path": path,
            "chats": chats,
        }),
        Event::ChatList(rows) => json!({
            "kind": "chat_list",
            "chats": rows
                .iter()
                .map(|row| json!({
                    "code": row.code,
                    "name": row.name,
                    "messages": row.messages,
                    "media": row.media,
                    "bytes": row.bytes,
                    "last": row.last,
                }))
                .collect::<Vec<_>>(),
        }),
        Event::Progress { label, done, total } => json!({
            "kind": "progress",
            "label": label,
            "done": done,
            "total": total,
        }),
        Event::Diagnostic(message) => json!({ "kind": "diagnostic", "message": message }),
        Event::Warning(message) => json!({ "kind": "warning", "message": message }),
        Event::Failure(message) => json!({ "kind": "failure", "message": message }),
        Event::Finished(finished) => json!({
            "kind": "finished",
            "archives": finished.archives,
            "skipped": finished.skipped,
            "messages": finished.messages,
            "media": finished.media,
            "missing": finished.missing,
            "failures": finished.failures,
            "bytes": finished.bytes,
            "path": finished.path,
            "seconds": (finished.seconds * 1000.0).round() / 1000.0,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_carries_its_kind_and_the_published_fields() {
        let events = [
            Event::Found {
                source: "android",
                path: "/tmp/phone".into(),
                chats: 3,
            },
            Event::ChatList(vec![ChatRow {
                code: 1,
                name: "Alice".into(),
                messages: 4,
                media: 2,
                bytes: 100,
                last: "2026-12-01 10:00:00 +01:00".into(),
            }]),
            Event::Progress {
                label: "1. Alice".into(),
                done: 1,
                total: 2,
            },
            Event::Diagnostic("media: path=Some(file_path)".into()),
            Event::Warning("careful".into()),
            Event::Failure("broken".into()),
            Event::Finished(Finished {
                archives: 1,
                skipped: 0,
                messages: 4,
                media: 2,
                missing: Some(1),
                failures: 0,
                bytes: 100,
                path: "/tmp/out".into(),
                seconds: 1.25,
            }),
        ];
        let kinds: Vec<String> = events
            .iter()
            .map(|event| {
                let value = event_value(event);
                let parsed: Value =
                    serde_json::from_str(&value.to_string()).expect("one JSON object");
                parsed["kind"].as_str().expect("a kind").to_string()
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "found",
                "chat_list",
                "progress",
                "diagnostic",
                "warning",
                "failure",
                "finished"
            ]
        );
        let finished = event_value(&events[6]);
        assert_eq!(finished["seconds"], 1.25);
        assert_eq!(finished["missing"], 1);
    }

    #[test]
    fn a_finished_event_without_a_missing_count_says_null() {
        let value = event_value(&Event::Finished(Finished {
            archives: 1,
            ..Finished::default()
        }));
        assert!(
            value["missing"].is_null(),
            "a dry run did not count the missing files"
        );
    }

    #[test]
    fn the_list_row_keeps_numbers_as_numbers() {
        let value = event_value(&Event::ChatList(vec![ChatRow {
            code: 2,
            name: "Bob".into(),
            messages: 10,
            media: 3,
            bytes: 1234,
            last: String::new(),
        }]));
        assert_eq!(value["chats"][0]["code"], 2);
        assert_eq!(value["chats"][0]["bytes"], 1234);
    }
}
