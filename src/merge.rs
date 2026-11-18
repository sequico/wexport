//! Replaying incremental backups into the full one.
//!
//! Incrementals are SQLite deltas — rows for the tables WhatsApp changed since the last full backup —
//! so the input here is already-decrypted databases, oldest delta first.
//!
//! A delta carries the rows that changed, not a record of what was removed, so `INSERT OR REPLACE`
//! is the whole replay: a message deleted on the phone simply does not appear in the delta and stays
//! in the archive. Picking up deletions would need a tombstone the format does not carry.
//!
//! The set is replayed oldest first: a `Databases/` folder holds the increments WhatsApp wrote after
//! the last full backup, and one kept from before it would replace newer rows with older ones.

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
        let mut bytes = std::fs::read(increment)?;
        let header = crypt15::parse_header(&bytes)?;
        let delta = workspace.join(format!("increment-{index}.db"));
        crypt15::decrypt_to(
            &mut bytes,
            &header,
            key,
            &mut std::fs::File::create(&delta)?,
        )?;
        replay(&connection, &delta)?;
    }
    Ok(())
}

fn replay(connection: &rusqlite::Connection, delta: &Path) -> Result<()> {
    // ATTACH cannot run inside a transaction, so it brackets one: the deltas are large and one
    // commit at the end is the difference between seconds and minutes.
    connection.execute("ATTACH DATABASE ?1 AS inc", [delta.display().to_string()])?;
    let transaction = connection.unchecked_transaction()?;
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
        let query =
            format!("INSERT OR REPLACE INTO main.\"{table}\" SELECT * FROM inc.\"{table}\"");
        transaction.execute(&query, [])?;
    }
    transaction.commit()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn database(name: &str, sql: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("wexport-merge-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch(sql).unwrap();
        path
    }

    fn text_of(path: &Path, id: i64) -> String {
        rusqlite::Connection::open(path)
            .unwrap()
            .query_row("SELECT text FROM message WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn a_delta_replaces_the_row_with_the_same_key() {
        let base = database(
            "base",
            "CREATE TABLE message (id INTEGER PRIMARY KEY, text TEXT);
             INSERT INTO message VALUES (1, 'old'), (2, 'kept');",
        );
        let delta = database(
            "delta",
            "CREATE TABLE message (id INTEGER PRIMARY KEY, text TEXT);
             INSERT INTO message VALUES (1, 'new');",
        );
        let connection = rusqlite::Connection::open(&base).unwrap();
        replay(&connection, &delta).unwrap();
        assert_eq!(text_of(&base, 1), "new");
        assert_eq!(text_of(&base, 2), "kept");
        let _ = std::fs::remove_file(base);
        let _ = std::fs::remove_file(delta);
    }

    #[test]
    fn a_delta_with_a_table_the_base_lacks_is_incomplete() {
        let base = database(
            "base-missing",
            "CREATE TABLE message (id INTEGER PRIMARY KEY);",
        );
        let delta = database(
            "delta-missing",
            "CREATE TABLE message (id INTEGER PRIMARY KEY);
             CREATE TABLE message_vcard (message_row_id INTEGER);",
        );
        let connection = rusqlite::Connection::open(&base).unwrap();
        let error = replay(&connection, &delta).unwrap_err();
        assert_eq!(error.code, crate::error::EXIT_INCOMPLETE);
        let _ = std::fs::remove_file(base);
        let _ = std::fs::remove_file(delta);
    }

    #[test]
    fn a_delta_with_other_columns_is_incomplete() {
        let base = database(
            "base-cols",
            "CREATE TABLE message (id INTEGER PRIMARY KEY, text TEXT);",
        );
        let delta = database(
            "delta-cols",
            "CREATE TABLE message (id INTEGER PRIMARY KEY, text TEXT, extra TEXT);",
        );
        let connection = rusqlite::Connection::open(&base).unwrap();
        let error = replay(&connection, &delta).unwrap_err();
        assert!(error.message.contains("different columns"));
        let _ = std::fs::remove_file(base);
        let _ = std::fs::remove_file(delta);
    }

    #[test]
    fn a_table_outside_the_read_set_is_ignored() {
        let base = database(
            "base-extra",
            "CREATE TABLE message (id INTEGER PRIMARY KEY, text TEXT);
             INSERT INTO message VALUES (1, 'kept');",
        );
        let delta = database(
            "delta-extra",
            "CREATE TABLE message (id INTEGER PRIMARY KEY, text TEXT);
             CREATE TABLE something_new (id INTEGER);
             INSERT INTO something_new VALUES (1);",
        );
        let connection = rusqlite::Connection::open(&base).unwrap();
        replay(&connection, &delta).unwrap();
        assert_eq!(text_of(&base, 1), "kept");
        let _ = std::fs::remove_file(base);
        let _ = std::fs::remove_file(delta);
    }
}
