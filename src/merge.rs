//! Replaying incremental backups into the full one.
//!
//! Incrementals are SQLite deltas — rows for the tables WhatsApp changed since the last full backup —
//! so the input here is already-decrypted databases, oldest delta first.
//!
//! shortcut: INSERT OR REPLACE by primary key, not a semantic replay. A message deleted from the
//! phone is absent from the delta rather than marked, so it stays in the archive. Upgrade when a
//! real incremental proves this wrong (plan §10, "incremental merge ordering").

use std::path::{Path, PathBuf};

use crate::crypt15;
use crate::error::{Error, Result};

/// The tables we read, in the order later tables reference earlier ones; merged generically anyway.
const MERGED_TABLES: [&str; 15] = [
    "jid",
    "chat",
    "message",
    "message_text",
    "message_media",
    "message_quoted",
    "message_revoked",
    "message_add_on",
    "message_add_on_reaction",
    "message_system",
    "message_location",
    "message_vcard",
    "receipt_user",
    "group_participant_user",
    "call_log",
];

/// Decrypt every increment and replay it into `base`, modifying it in place.
pub fn apply(base: &Path, increments: &[PathBuf], key: &[u8; 32], workspace: &Path) -> Result<()> {
    let connection = rusqlite::Connection::open(base)?;
    for (index, increment) in increments.iter().enumerate() {
        let bytes = std::fs::read(increment)?;
        let header = crypt15::parse_header(&bytes)?;
        let delta = workspace.join(format!("increment-{index}.db"));
        std::fs::write(&delta, crypt15::decrypt(&bytes, &header, key)?)?;
        replay(&connection, &delta)?;
    }
    Ok(())
}

fn replay(connection: &rusqlite::Connection, delta: &Path) -> Result<()> {
    connection.execute("ATTACH DATABASE ?1 AS inc", [delta.display().to_string()])?;
    for table in MERGED_TABLES {
        if !has_table(connection, "inc", table)? {
            continue;
        }
        if !has_table(connection, "main", table)? {
            return Err(Error::incomplete(format!(
                "{} carries table {table}, which the full backup does not have: make a fresh full backup",
                delta.display()
            )));
        }
        if columns_of(connection, "main", table)? != columns_of(connection, "inc", table)? {
            return Err(Error::incomplete(format!(
                "table {table} has different columns between the full backup and {}: make a fresh full backup",
                delta.display()
            )));
        }
        connection.execute(
            &format!("INSERT OR REPLACE INTO main.\"{table}\" SELECT * FROM inc.\"{table}\""),
            [],
        )?;
    }
    connection.execute("DETACH DATABASE inc", [])?;
    Ok(())
}

fn has_table(connection: &rusqlite::Connection, schema: &str, table: &str) -> Result<bool> {
    let count: i64 = connection.query_row(
        &format!("SELECT COUNT(*) FROM {schema}.sqlite_master WHERE type='table' AND name = ?1"),
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn columns_of(connection: &rusqlite::Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA {schema}.table_info(\"{table}\")"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.filter_map(|row| row.ok()).collect())
}
