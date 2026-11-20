//! wexport: an Android WhatsApp crypt15 backup to self-contained offline chat archives.
//!
//! One module per stage of the pipeline (plan §2): `crypt15` (the container), `discover` (finding the
//! backup), `merge` (incrementals), `msgstore` (WhatsApp's schema), `media` (files and derivatives),
//! `chunks` (the archive data), `archive` (one folder per chat), `progress`, `ui` (the command).

// A production build may not unwrap or panic: the error type carries the message and the exit code.
// Tests are exempt, and so are integration tests, which are separate crates.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::panic))]

pub mod archive;
pub mod chunks;
pub mod container;
pub mod crypt15;
pub mod discover;
pub mod error;
pub mod ios;
pub mod keys;
pub mod media;
pub mod merge;
pub mod msgstore;
pub mod progress;
pub mod report;
pub mod source;
pub mod ui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
