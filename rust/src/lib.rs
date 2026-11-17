//! wexport in Rust: the same product as the Python implementation, on one binary.
//!
//! Modules mirror the Python ones so a reader can move between the two: `crypt15` (the container),
//! `discover` (finding the backup), `msgstore` (WhatsApp's schema), `media` (files and derivatives),
//! `chunks` (the archive data), `archive` (one folder per chat), `ui` (the command).

pub mod archive;
pub mod chunks;
pub mod crypt15;
pub mod discover;
pub mod error;
pub mod keys;
pub mod media;
pub mod msgstore;
pub mod progress;
pub mod ui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
