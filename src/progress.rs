//! A one-line progress bar for the part that takes time: media, per chat.
//!
//! The reporter decides whether it draws at all: silent on a pipe, with `-q`, and with `-j`, so the
//! machine-readable stream stays clean. Throttled, so drawing costs nothing next to a thumbnail.

use std::io::Write;
use std::time::Instant;

pub struct Bar {
    label: String,
    total: usize,
    done: usize,
    enabled: bool,
    last: Instant,
}

impl Bar {
    pub fn new(label: &str, total: usize, enabled: bool) -> Self {
        let label: String = label.chars().take(24).collect();
        Bar {
            label,
            total,
            done: 0,
            enabled: enabled && total > 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_bar_draws_nothing_but_still_counts() {
        let mut bar = Bar::new("1. Alice", 10, false);
        bar.step(3);
        assert_eq!(bar.done, 3);
        assert!(!bar.enabled);
    }

    #[test]
    fn the_render_is_one_line_with_the_counts() {
        let mut bar = Bar::new("1. Alice", 4, true);
        bar.done = 2;
        let line = bar.render();
        assert!(line.contains("2/4 files"));
        assert!(line.contains("50%"));
        assert!(!line.contains('\n'));
    }
}
