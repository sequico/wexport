//! A one-line progress bar for the part that takes time: media, per chat.
//!
//! Silent when stdout is not a terminal, so a log file or a pipe stays clean; throttled, so drawing
//! costs nothing next to a thumbnail or a transcode.

use std::io::{IsTerminal, Write};
use std::time::Instant;

pub struct Bar {
    label: String,
    total: usize,
    done: usize,
    enabled: bool,
    last: Instant,
}

impl Bar {
    pub fn new(label: &str, total: usize) -> Self {
        let label: String = label.chars().take(24).collect();
        Bar {
            label,
            total,
            done: 0,
            enabled: total > 0 && std::io::stdout().is_terminal(),
            last: Instant::now(),
        }
    }

    pub fn step(&mut self, count: usize) {
        self.done += count;
        if !self.enabled || self.done >= self.total {
            return; // the final line belongs to close(), drawn once
        }
        if self.last.elapsed().as_millis() < 100 {
            return;
        }
        self.last = Instant::now();
        print!("\r{}", self.render());
        let _ = std::io::stdout().flush();
    }

    pub fn render(&self) -> String {
        let width = 24;
        let ratio = if self.total == 0 {
            1.0
        } else {
            (self.done as f64 / self.total as f64).min(1.0)
        };
        let filled = (width as f64 * ratio) as usize;
        format!(
            "  {:<24} [{}{}] {:3}% {}/{} files",
            self.label,
            "#".repeat(filled),
            "-".repeat(width - filled),
            (ratio * 100.0) as u32,
            self.done,
            self.total
        )
    }

    pub fn close(&mut self) {
        if !self.enabled {
            return;
        }
        self.done = self.total;
        print!("\r{}\n", self.render());
        let _ = std::io::stdout().flush();
    }
}
