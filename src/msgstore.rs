//! The only module that knows WhatsApp's table and column names.
//!
//! Column names are resolved by probing the schema, never by guessing the app version; a rename
//! breaks exactly here and nowhere else.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub jid: String,
    pub name: String,
    pub is_group: bool,
}

#[derive(Debug, Clone)]
pub struct Media {
    pub rel_path: String,
    pub kind: String,
    pub mime: Option<String>,
    pub bytes: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: i64,
    pub ts: i64,
    pub from_me: bool,
    pub kind: String,
    pub text: String,
    pub sender: Option<String>,
    pub quoted_id: Option<i64>,
    pub edited: bool,
    pub revoked: bool,
    pub status: Option<i64>,
    pub payload: Option<Value>,
    pub media: Option<Media>,
    pub reactions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ChatStat {
    pub chat: Chat,
    pub messages: i64,
    pub first_ts: i64,
    pub last_ts: i64,
    pub media_count: i64,
    pub media_bytes: i64,
}

#[derive(Debug, Default)]
struct MediaRow {
    rel_path: String,
    name: String,
    mime: Option<String>,
    bytes: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    duration: Option<i64>,
    caption: Option<String>,
}

pub struct Msgstore {
    con: Connection,
    tables: HashSet<String>,
    // message columns
    c_id: String,
    c_ts: String,
    c_from_me: Option<String>,
    c_text: Option<String>,
    c_status: Option<String>,
    c_remote: Option<String>,
    c_sender_row: Option<String>,
    c_sender_text: Option<String>,
    jid_raw: Option<String>,
    c_quoted: Option<String>,
    c_edit: Option<String>,
    c_lat: Option<String>,
    c_lon: Option<String>,
    c_wa_type: Option<String>,
    c_chat_row: Option<String>,
    c_chat_jid: Option<String>,
    // media columns
    m_message: Option<String>,
    m_path: Option<String>,
    m_name: Option<String>,
    m_mime: Option<String>,
    m_bytes: Option<String>,
    m_width: Option<String>,
    m_height: Option<String>,
    m_duration: Option<String>,
    m_caption: Option<String>,
    // rows
    chats: BTreeMap<i64, Chat>,
    jid_row: HashMap<i64, i64>,
    contacts: HashMap<String, String>,
    revoked: HashSet<i64>,
    systems: HashSet<i64>,
    media_rows: HashMap<i64, MediaRow>,
    vcards: HashMap<i64, String>,
    reactions: HashMap<i64, Vec<String>>,
}

impl Msgstore {
    pub fn open(path: &Path) -> Result<Self> {
        let con = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| Error::input(format!("cannot open {}: {error}", path.display())))?;
        let tables: HashSet<String> = {
            let mut statement = con.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.filter_map(|row| row.ok()).collect()
        };
        if !tables.contains("message") {
            return Err(Error::input("no message table — not a msgstore database"));
        }

        let mut store = Msgstore {
            c_id: String::new(),
            c_ts: String::new(),
            c_from_me: None,
            c_text: None,
            c_status: None,
            c_remote: None,
            c_sender_row: None,
            c_sender_text: None,
            jid_raw: None,
            c_quoted: None,
            c_edit: None,
            c_lat: None,
            c_lon: None,
            c_wa_type: None,
            c_chat_row: None,
            c_chat_jid: None,
            m_message: None,
            m_path: None,
            m_name: None,
            m_mime: None,
            m_bytes: None,
            m_width: None,
            m_height: None,
            m_duration: None,
            m_caption: None,
            chats: BTreeMap::new(),
            jid_row: HashMap::new(),
            contacts: HashMap::new(),
            revoked: HashSet::new(),
            systems: HashSet::new(),
            media_rows: HashMap::new(),
            vcards: HashMap::new(),
            reactions: HashMap::new(),
            tables,
            con,
        };

        store.c_id = store.require("message", &["_id"])?;
        store.c_ts = store.require("message", &["timestamp"])?;
        store.c_from_me = store.pick("message", &["from_me", "key_from_me"]);
        store.c_text = store.pick("message", &["text_data", "data"]);
        store.c_status = store.pick("message", &["status"]);
        store.c_remote = store.pick("message", &["remote_resource"]);
        // Modern group messages carry the author as a jid row, not as a text field.
        store.c_sender_row = store.pick("message", &["sender_jid_row_id"]);
        store.c_sender_text = store.pick("message", &["sender_jid"]);
        store.jid_raw = store.pick("jid", &["raw_string", "user"]);
        store.c_quoted = store.pick("message", &["quoted_row_id"]);
        store.c_edit = store.pick("message", &["edit_version"]);
        store.c_lat = store.pick("message", &["latitude"]);
        store.c_lon = store.pick("message", &["longitude"]);
        store.c_wa_type = store.pick("message", &["media_wa_type", "message_type"]);
        store.c_chat_row = store.pick("message", &["chat_row_id"]);
        store.c_chat_jid = store.pick("message", &["key_remote_jid"]);

        store.m_message = store.pick("message_media", &["message_row_id", "message_id"]);
        store.m_path = store.pick("message_media", &["file_path"]);
        store.m_name = store.pick("message_media", &["media_name", "file_name"]);
        store.m_mime = store.pick("message_media", &["mime_type", "media_mime_type"]);
        store.m_bytes = store.pick("message_media", &["file_size", "media_size"]);
        store.m_width = store.pick("message_media", &["width"]);
        store.m_height = store.pick("message_media", &["height"]);
        store.m_duration = store.pick("message_media", &["media_duration", "duration"]);
        store.m_caption = store.pick("message_media", &["caption", "media_caption"]);

        store.load_chats()?;
        store.revoked = store.id_set("message_revoked", "message_row_id");
        store.systems = store.id_set("message_system", "message_row_id");
        store.load_media_rows()?;
        store.load_vcards()?;
        store.load_reactions()?;
        Ok(store)
    }

    fn columns_of(&self, table: &str) -> Vec<String> {
        let mut statement = match self.con.prepare(&format!("PRAGMA table_info(\"{table}\")")) {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };
        let collected: Vec<String> = match statement.query_map([], |row| row.get::<_, String>(1)) {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(_) => Vec::new(),
        };
        collected
    }

    fn pick(&self, table: &str, candidates: &[&str]) -> Option<String> {
        if !self.tables.contains(table) {
            return None;
        }
        let columns = self.columns_of(table);
        candidates
            .iter()
            .find(|name| columns.iter().any(|column| column == *name))
            .map(|name| (*name).to_string())
    }

    fn require(&self, table: &str, candidates: &[&str]) -> Result<String> {
        self.pick(table, candidates).ok_or_else(|| {
            Error::input(format!(
                "table {table} has none of the columns {}; the schema is newer or older than this build understands",
                candidates.join(", ")
            ))
        })
    }

    fn id_set(&self, table: &str, column: &str) -> HashSet<i64> {
        if !self.tables.contains(table) {
            return HashSet::new();
        }
        if !self.columns_of(table).iter().any(|name| name == column) {
            return HashSet::new();
        }
        let mut statement = match self
            .con
            .prepare(&format!("SELECT \"{column}\" FROM \"{table}\""))
        {
            Ok(statement) => statement,
            Err(_) => return HashSet::new(),
        };
        let collected: HashSet<i64> = match statement.query_map([], |row| row.get::<_, i64>(0)) {
            Ok(rows) => rows.filter_map(|row| row.ok()).collect(),
            Err(_) => HashSet::new(),
        };
        collected
    }

    fn load_chats(&mut self) -> Result<()> {
        let (Some(jid_row), Some(jid_raw)) = (
            self.pick("chat", &["jid_row_id"]),
            self.pick("jid", &["raw_string", "user"]),
        ) else {
            return Ok(());
        };
        let subject_sql = self
            .pick("chat", &["subject"])
            .map(|name| format!("c.\"{name}\""))
            .unwrap_or_else(|| "NULL".into());
        let sql = format!(
            "SELECT c.\"_id\", j.\"{jid_raw}\", {subject_sql}, c.\"{jid_row}\" FROM chat c JOIN jid j ON j.\"_id\" = c.\"{jid_row}\""
        );
        let mut statement = self.con.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for (chat_id, raw, subject, jid_row_id) in rows.flatten() {
            let Some(jid) = raw
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(jid_row_id) = jid_row_id else {
                continue;
            };
            let name = subject
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| jid.split('@').next().unwrap_or("").to_string());
            self.chats.insert(
                chat_id,
                Chat {
                    is_group: jid.ends_with("@g.us"),
                    jid,
                    name,
                },
            );
            self.jid_row.insert(chat_id, jid_row_id);
        }
        Ok(())
    }

    fn load_media_rows(&mut self) -> Result<()> {
        let (Some(key), Some(path)) = (self.m_message.clone(), self.m_path.clone()) else {
            return Ok(());
        };
        let column = |name: &Option<String>| name.clone().unwrap_or_else(|| "NULL".to_string());
        let sql = format!(
            "SELECT \"{key}\", \"{path}\", {name}, {}, {}, {}, {}, {}, {} FROM message_media",
            column(&self.m_mime),
            column(&self.m_bytes),
            column(&self.m_width),
            column(&self.m_height),
            column(&self.m_duration),
            column(&self.m_caption),
            name = column(&self.m_name),
        );
        let mut statement = self.con.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })?;
        for (message, path, name, mime, bytes, width, height, duration, caption) in rows.flatten() {
            // A row whose path is empty has no file to place: treating it as media would put an
            // empty `md` in the archive and a missing file in every count.
            let Some(rel_path) = path
                .map(|value| value.replace('\\', "/"))
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            self.media_rows.insert(
                message,
                MediaRow {
                    rel_path,
                    name: name.unwrap_or_default(),
                    mime,
                    bytes,
                    width,
                    height,
                    duration,
                    caption,
                },
            );
        }
        Ok(())
    }

    fn load_vcards(&mut self) -> Result<()> {
        let (Some(key), Some(name)) = (
            self.pick("message_vcard", &["message_row_id", "message_id"]),
            self.pick(
                "message_vcard",
                &["display_name", "formatted_name", "vcard"],
            ),
        ) else {
            return Ok(());
        };
        let mut statement = self
            .con
            .prepare(&format!("SELECT \"{key}\", \"{name}\" FROM message_vcard"))?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for (id, value) in rows.flatten() {
            self.vcards.insert(id, value.unwrap_or_default());
        }
        Ok(())
    }

    fn load_reactions(&mut self) -> Result<()> {
        let Some(message_col) = self.pick("message_add_on", &["message_row_id", "message_id"])
        else {
            return Ok(());
        };
        if self
            .columns_of("message_add_on")
            .iter()
            .any(|name| name == "reaction")
        {
            let mut statement = self.con.prepare(&format!(
                "SELECT \"{message_col}\", reaction FROM message_add_on WHERE reaction IS NOT NULL"
            ))?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for (id, emoji) in rows.flatten() {
                if let Some(emoji) = emoji {
                    self.reactions.entry(id).or_default().push(emoji);
                }
            }
            return Ok(());
        }
        let (Some(id_col), Some(parent)) = (
            self.pick("message_add_on", &["_id", "id", "message_add_on_id"]),
            self.pick(
                "message_add_on_reaction",
                &[
                    "message_add_on_row_id",
                    "message_add_on_id",
                    "parent_row_id",
                ],
            ),
        ) else {
            return Ok(());
        };
        let sql = format!(
            "SELECT a.\"{message_col}\", r.reaction FROM message_add_on a JOIN message_add_on_reaction r ON r.\"{parent}\" = a.\"{id_col}\""
        );
        let mut statement = self.con.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for (id, emoji) in rows.flatten() {
            if let Some(emoji) = emoji {
                self.reactions.entry(id).or_default().push(emoji);
            }
        }
        Ok(())
    }

    pub fn apply_contacts(&mut self, path: &Path) {
        let Ok(con) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            return;
        };
        let mut columns: Vec<String> = Vec::new();
        if let Ok(mut statement) = con.prepare("PRAGMA table_info(\"wa_contacts\")") {
            if let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(1)) {
                columns = rows.filter_map(|row| row.ok()).collect();
            }
        }
        if !columns.iter().any(|name| name == "jid") {
            return;
        }
        // The name column travels between versions; ask the schema which one is there.
        let name_column = ["display_name", "wa_name", "given_name"]
            .into_iter()
            .find(|candidate| columns.iter().any(|name| name == candidate));
        let Some(name_column) = name_column else {
            return;
        };
        let Ok(mut statement) =
            con.prepare(&format!("SELECT jid, \"{name_column}\" FROM wa_contacts"))
        else {
            return;
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        }) else {
            return;
        };
        for (jid, name) in rows.flatten() {
            if let Some(name) = name.filter(|value| !value.trim().is_empty()) {
                self.contacts.insert(jid_key(&jid), name);
            }
        }
        for chat in self.chats.values_mut() {
            if let Some(name) = self.contacts.get(&jid_key(&chat.jid)) {
                chat.name = name.clone();
            }
        }
    }

    pub fn chat_stats(&self) -> Result<Vec<ChatStat>> {
        if let (Some(column), false) = (&self.c_chat_row, self.chats.is_empty()) {
            let grouping = format!("m.\"{column}\"");
            let totals: HashMap<i64, (i64, i64, i64)> = self.totals(&grouping)?;
            let media: HashMap<i64, (i64, i64)> = self.media_totals(&grouping);
            let calls: HashMap<i64, i64> = self.call_times("jid_row_id");
            let mut out = Vec::new();
            for (chat_id, chat) in &self.chats {
                let (count, first, last) = totals.get(chat_id).copied().unwrap_or((0, 0, 0));
                let last = self
                    .jid_row
                    .get(chat_id)
                    .and_then(|jid| calls.get(jid).copied())
                    .map(|call| last.max(call))
                    .unwrap_or(last);
                let first = if first == 0 { last } else { first };
                let (media_count, media_bytes) = media.get(chat_id).copied().unwrap_or((0, 0));
                out.push(ChatStat {
                    chat: chat.clone(),
                    messages: count,
                    first_ts: first,
                    last_ts: last,
                    media_count,
                    media_bytes,
                });
            }
            return Ok(newest_first(out));
        }

        let Some(column) = &self.c_chat_jid else {
            return Err(Error::input("cannot tell which chat a message belongs to"));
        };
        let grouping = format!("m.\"{column}\"");
        let totals: HashMap<String, (i64, i64, i64)> = self.totals(&grouping)?;
        let media: HashMap<String, (i64, i64)> = self.media_totals(&grouping);
        let calls: HashMap<String, i64> = self.call_times(column);
        let mut out = Vec::new();
        for (jid, (count, first, last)) in totals {
            let jid = jid.trim().to_string();
            if jid.is_empty() {
                continue;
            }
            let last = last.max(calls.get(&jid).copied().unwrap_or(0));
            let first = if first == 0 { last } else { first };
            let (media_count, media_bytes) = media.get(&jid).copied().unwrap_or((0, 0));
            out.push(ChatStat {
                chat: Chat {
                    is_group: jid.ends_with("@g.us"),
                    name: jid.split('@').next().unwrap_or("").to_string(),
                    jid,
                },
                messages: count,
                first_ts: first,
                last_ts: last,
                media_count,
                media_bytes,
            });
        }
        // Ties in the last event are broken by jid: the codes are an identity, so the list they come
        // from has to be the same list on every run.
        out.sort_by(|left, right| left.chat.jid.cmp(&right.chat.jid));
        Ok(newest_first(out))
    }

    /// Message count and time range per chat, grouped by whichever column identifies a chat.
    fn totals<K>(&self, grouping: &str) -> Result<HashMap<K, (i64, i64, i64)>>
    where
        K: rusqlite::types::FromSql + std::hash::Hash + Eq,
    {
        let sql = format!(
            "SELECT {grouping}, COUNT(*), MIN(m.\"{ts}\"), MAX(m.\"{ts}\") FROM message m GROUP BY 1",
            ts = self.c_ts
        );
        let mut statement = self.con.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, K>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (key, count, first, last) = row?;
            out.insert(key, (count, first.unwrap_or(0), last.unwrap_or(0)));
        }
        Ok(out)
    }

    /// Media count and weight per chat, keyed the same way as `totals`.
    fn media_totals<K>(&self, grouping: &str) -> HashMap<K, (i64, i64)>
    where
        K: rusqlite::types::FromSql + std::hash::Hash + Eq,
    {
        let (Some(message_col), Some(size_col)) = (&self.m_message, &self.m_bytes) else {
            return HashMap::new();
        };
        let sql = format!(
            "SELECT {grouping}, COUNT(*), COALESCE(SUM(d.\"{size_col}\"), 0) FROM message_media d JOIN message m ON m.\"{}\" = d.\"{message_col}\" GROUP BY 1",
            self.c_id
        );
        let Ok(mut statement) = self.con.prepare(&sql) else {
            return HashMap::new();
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, K>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) else {
            return HashMap::new();
        };
        rows.filter_map(|row| row.ok())
            .map(|(key, count, size)| (key, (count, size)))
            .collect()
    }

    /// Newest call per chat, in one read: a chat whose last event was a call still sorts correctly.
    fn call_times<K>(&self, column: &str) -> HashMap<K, i64>
    where
        K: rusqlite::types::FromSql + std::hash::Hash + Eq,
    {
        if !self.tables.contains("call_log") {
            return HashMap::new();
        }
        let Ok(mut statement) = self.con.prepare(&format!(
            "SELECT \"{column}\", MAX(timestamp) FROM call_log GROUP BY 1"
        )) else {
            return HashMap::new();
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, K>(0)?, row.get::<_, Option<i64>>(1)?))
        }) else {
            return HashMap::new();
        };
        rows.filter_map(|row| row.ok())
            .map(|(jid, stamp)| (jid, stamp.unwrap_or(0)))
            .collect()
    }

    /// Every message of one chat, in time order, handed to `sink` one at a time.
    ///
    /// Streaming is what keeps a chat out of memory: the archive writes each record as it arrives,
    /// and the only bound is one chunk's worth. Calls are read from their own table and merged here,
    /// by the timestamp the database ordered both sides by.
    pub fn each_message(
        &self,
        chat: &Chat,
        sink: &mut dyn FnMut(Message) -> Result<()>,
    ) -> Result<()> {
        let alias = |name: &str, column: &Option<String>, default: &str| match column {
            Some(column) => format!("m.\"{column}\" AS {name}"),
            None => format!("{default} AS {name}"),
        };
        let sender_row = match (&self.c_sender_row, &self.jid_raw) {
            (Some(_), Some(raw)) => format!("s.\"{raw}\""),
            _ => "NULL".into(),
        };
        let select = [
            format!("m.\"{}\" AS id", self.c_id),
            format!("m.\"{}\" AS ts", self.c_ts),
            alias("from_me", &self.c_from_me, "0"),
            alias("text", &self.c_text, "NULL"),
            alias("status", &self.c_status, "NULL"),
            alias("remote", &self.c_remote, "NULL"),
            alias("quoted", &self.c_quoted, "NULL"),
            alias("edit", &self.c_edit, "0"),
            alias("lat", &self.c_lat, "NULL"),
            alias("lon", &self.c_lon, "NULL"),
            alias("wa_type", &self.c_wa_type, "NULL"),
            format!("{sender_row} AS sender_row"),
            alias("sender_text", &self.c_sender_text, "NULL"),
        ];
        let join = match (&self.c_sender_row, &self.jid_raw) {
            (Some(row), Some(_)) => format!(" LEFT JOIN jid s ON s.\"_id\" = m.\"{row}\""),
            _ => String::new(),
        };
        let (where_sql, params) = self.chat_filter(chat);
        let sql = format!(
            "SELECT {} FROM message m{join} WHERE {where_sql} ORDER BY m.\"{}\", m.\"{}\"",
            select.join(", "),
            self.c_ts,
            self.c_id
        );
        let mut statement = self.con.prepare(&sql)?;
        let mut rows = statement.query(rusqlite::params_from_iter(params))?;

        let layout = self.call_layout();
        // A call log is small next to a message table, so its rows can be collected; the messages,
        // which are not small, stay streamed.
        let calls: Vec<Message> = match &layout {
            Some(layout) => {
                let (filter, params) = self.call_filter(chat, layout);
                let mut statement = self.con.prepare(&format!(
                    // Calls with one timestamp keep the order of the table, so a rebuild is identical.
                    "SELECT * FROM call_log WHERE {filter} ORDER BY timestamp, \"_id\""
                ))?;
                let mut rows = statement.query(rusqlite::params_from_iter(params))?;
                let mut collected = Vec::new();
                while let Some(row) = rows.next()? {
                    collected.push(self.call_from_row(row, layout)?);
                }
                collected
            }
            None => Vec::new(),
        };
        let mut calls = calls.into_iter().peekable();

        let mut pending_message = match rows.next()? {
            Some(row) => Some(self.message_from_row(row)?),
            None => None,
        };
        loop {
            let take_call = match (&pending_message, calls.peek()) {
                (Some(message), Some(call)) => (call.ts, call.id) < (message.ts, message.id),
                (None, Some(_)) => true,
                (Some(_), None) => false,
                (None, None) => break,
            };
            if take_call {
                sink(calls.next().expect("peeked above"))?;
            } else {
                sink(pending_message.take().expect("checked above"))?;
                pending_message = match rows.next()? {
                    Some(row) => Some(self.message_from_row(row)?),
                    None => None,
                };
            }
        }
        Ok(())
    }

    /// The same chat as a list, for tests and for callers that want it whole.
    pub fn messages(&self, chat: &Chat) -> Result<Vec<Message>> {
        let mut out = Vec::new();
        self.each_message(chat, &mut |message| {
            out.push(message);
            Ok(())
        })?;
        Ok(out)
    }

    fn message_from_row(&self, row: &rusqlite::Row<'_>) -> Result<Message> {
        let id: i64 = row.get(0)?;
        let ts: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
        let from_me = row.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0;
        let text: Option<String> = row.get(3)?;
        let status: Option<i64> = row.get(4)?;
        let remote: Option<String> = row.get(5)?;
        let quoted: Option<i64> = row.get(6)?;
        // 0 means "not a reply" in every schema WhatsApp has shipped; carrying it would put a quote
        // pointing at message 0 in the archive.
        let quoted = quoted.filter(|value| *value > 0);
        let edited = row.get::<_, Option<i64>>(7)?.unwrap_or(0) > 0;
        let lat: Option<f64> = row.get(8)?;
        let lon: Option<f64> = row.get(9)?;
        let wa_type: Option<i64> = row.get(10)?;
        let sender_row: Option<String> = row.get(11)?;
        let sender_text: Option<String> = row.get(12)?;
        let author = remote.or(sender_row).or(sender_text);

        let media = self.media_of(id, wa_type);
        let caption = self
            .media_rows
            .get(&id)
            .and_then(|row| row.caption.clone())
            .unwrap_or_default();
        let (kind, payload) = self.classify(id, &media, lat, lon);
        let revoked = self.revoked.contains(&id);
        Ok(Message {
            id,
            ts,
            from_me,
            kind: if revoked { "deleted".to_string() } else { kind },
            text: text.filter(|value| !value.is_empty()).unwrap_or(caption),
            sender: if from_me {
                None
            } else {
                author.as_deref().map(|value| self.person_name(value))
            },
            quoted_id: quoted,
            edited,
            revoked,
            status,
            payload,
            media,
            reactions: self.reactions.get(&id).cloned().unwrap_or_default(),
        })
    }

    /// The `chat` table row for a chat, when this schema has that table.
    fn chat_row_of(&self, chat: &Chat) -> Option<i64> {
        self.chats
            .iter()
            .find(|(_, candidate)| candidate.jid == chat.jid)
            .map(|(id, _)| *id)
    }

    fn chat_filter(&self, chat: &Chat) -> (String, Vec<SqlValue>) {
        if let (Some(column), false) = (&self.c_chat_row, self.chats.is_empty()) {
            return match self.chat_row_of(chat) {
                Some(id) => (format!("m.\"{column}\" = ?"), vec![SqlValue::Integer(id)]),
                None => ("0".to_string(), Vec::new()),
            };
        }
        let Some(column) = &self.c_chat_jid else {
            return ("0".to_string(), Vec::new());
        };
        let c_us = chat.jid.replace("@s.whatsapp.net", "@c.us");
        if c_us != chat.jid {
            (
                format!("m.\"{column}\" IN (?, ?)"),
                vec![SqlValue::Text(chat.jid.clone()), SqlValue::Text(c_us)],
            )
        } else {
            (
                format!("m.\"{column}\" = ?"),
                vec![SqlValue::Text(chat.jid.clone())],
            )
        }
    }

    fn media_of(&self, message_id: i64, wa_type: Option<i64>) -> Option<Media> {
        let row = self.media_rows.get(&message_id)?;
        Some(Media {
            rel_path: row.rel_path.clone(),
            kind: media_kind(row, wa_type),
            mime: row.mime.clone(),
            bytes: row.bytes,
            width: row.width,
            height: row.height,
            duration_ms: row.duration,
        })
    }

    fn classify(
        &self,
        message_id: i64,
        media: &Option<Media>,
        lat: Option<f64>,
        lon: Option<f64>,
    ) -> (String, Option<Value>) {
        if self.systems.contains(&message_id) {
            return ("system".to_string(), None);
        }
        if let Some(media) = media {
            return (media.kind.clone(), None);
        }
        if let (Some(lat), Some(lon)) = (lat, lon) {
            return (
                "location".to_string(),
                Some(json!({ "lat": lat, "lon": lon })),
            );
        }
        if let Some(name) = self.vcards.get(&message_id) {
            return ("contact".to_string(), Some(json!({ "name": name })));
        }
        ("text".to_string(), None)
    }

    fn person_name(&self, jid: &str) -> String {
        self.contacts
            .get(&jid_key(jid))
            .cloned()
            .unwrap_or_else(|| jid.split('@').next().unwrap_or("").to_string())
    }

    /// Which columns this schema's call log has, when it has a usable one at all.
    fn call_layout(&self) -> Option<CallLayout> {
        if !self.tables.contains("call_log") {
            return None;
        }
        let columns = self.columns_of("call_log");
        let has = |name: &str| columns.iter().any(|column| column == name);
        if !has("timestamp") {
            return None;
        }
        let jid_column = ["jid_row_id", "key_remote_jid"]
            .into_iter()
            .find(|name| has(name))?
            .to_string();
        let find = |names: &[&str]| {
            names
                .iter()
                .find(|name| has(name))
                .map(|name| name.to_string())
        };
        Some(CallLayout {
            jid_column,
            duration: find(&["duration", "call_duration"]),
            video: find(&["video_call", "is_video"]),
            missed: find(&["missed_call", "is_missed"]),
            from_me: has("from_me"),
        })
    }

    /// The WHERE clause that selects this chat's calls, in whichever shape the table uses.
    fn call_filter(&self, chat: &Chat, layout: &CallLayout) -> (String, Vec<SqlValue>) {
        if layout.jid_column == "jid_row_id" {
            let jid_row = self
                .chat_row_of(chat)
                .and_then(|chat_row| self.jid_row.get(&chat_row).copied());
            return match jid_row {
                Some(jid_row) => (
                    "\"jid_row_id\" = ?1".to_string(),
                    vec![SqlValue::Integer(jid_row)],
                ),
                None => ("0".to_string(), Vec::new()),
            };
        }
        let c_us = chat.jid.replace("@s.whatsapp.net", "@c.us");
        if c_us == chat.jid {
            (
                "\"key_remote_jid\" = ?1".to_string(),
                vec![SqlValue::Text(chat.jid.clone())],
            )
        } else {
            (
                "\"key_remote_jid\" IN (?1, ?2)".to_string(),
                vec![SqlValue::Text(chat.jid.clone()), SqlValue::Text(c_us)],
            )
        }
    }

    fn call_from_row(&self, row: &rusqlite::Row<'_>, layout: &CallLayout) -> Result<Message> {
        let id: i64 = row.get("_id")?;
        let ts: i64 = row.get::<_, Option<i64>>("timestamp")?.unwrap_or(0);
        let read = |column: &Option<String>| -> i64 {
            column
                .as_ref()
                .and_then(|name| row.get::<_, Option<i64>>(name.as_str()).ok().flatten())
                .unwrap_or(0)
        };
        Ok(Message {
            id: -id,
            ts,
            from_me: layout.from_me && row.get::<_, Option<i64>>("from_me")?.unwrap_or(0) != 0,
            kind: "call".to_string(),
            text: String::new(),
            sender: None,
            quoted_id: None,
            edited: false,
            revoked: false,
            status: None,
            payload: Some(json!({
                "duration_s": read(&layout.duration),
                "video": read(&layout.video) != 0,
                "missed": read(&layout.missed) != 0,
            })),
            media: None,
            reactions: Vec::new(),
        })
    }
}

/// The call-log shape of one schema: which column names to read, and whether it knows the caller.
struct CallLayout {
    jid_column: String,
    duration: Option<String>,
    video: Option<String>,
    missed: Option<String>,
    from_me: bool,
}

fn jid_key(jid: &str) -> String {
    jid.split('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
}

/// Newest event first: the order the list shows, and the order the chat codes are assigned in.
fn newest_first(mut stats: Vec<ChatStat>) -> Vec<ChatStat> {
    stats.sort_by_key(|stat| std::cmp::Reverse(stat.last_ts));
    stats
}

fn media_kind(row: &MediaRow, wa_type: Option<i64>) -> String {
    if let Some(numeric) = wa_type {
        let kind = match numeric {
            1 => Some("image"),
            2 => Some("audio"),
            3 => Some("video"),
            4 => Some("contact"),
            5 => Some("location"),
            8 => Some("document"),
            13 => Some("gif"),
            20 => Some("sticker"),
            _ => None,
        };
        if let Some(kind) = kind {
            return kind.to_string();
        }
    }
    let mime = row.mime.clone().unwrap_or_default();
    let path = row.rel_path.to_lowercase();
    let name = row.name.to_lowercase();
    if mime.starts_with("image/") {
        return if mime == "image/webp" && (path.contains("sticker") || name.contains("sticker")) {
            "sticker"
        } else {
            "image"
        }
        .to_string();
    }
    if mime.starts_with("video/") {
        return if name.starts_with("gif") || path.contains("animated") {
            "gif"
        } else {
            "video"
        }
        .to_string();
    }
    if mime.starts_with("audio/") {
        return "audio".to_string();
    }
    "document".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schema of the backups this app refuses to open is the only one the fixture does not
    /// carry: no `chat`/`jid` tables, the jid written on every row, calls pointing at it by text.
    fn legacy_store() -> (Msgstore, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("wexport-legacy-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let con = Connection::open(&path).unwrap();
        con.execute_batch(
            "CREATE TABLE message (_id INTEGER PRIMARY KEY, key_remote_jid TEXT,
                 timestamp INTEGER, from_me INTEGER, data TEXT, media_wa_type INTEGER);
             CREATE TABLE message_media (message_id INTEGER, file_path TEXT,
                 mime_type TEXT, file_size INTEGER);
             CREATE TABLE call_log (_id INTEGER PRIMARY KEY, key_remote_jid TEXT, timestamp INTEGER,
                 duration INTEGER, video_call INTEGER, missed_call INTEGER, from_me INTEGER);
             INSERT INTO message VALUES
                 (1, '393331234567@s.whatsapp.net', 1000, 0, 'hello', 0),
                 (2, '393331234567@s.whatsapp.net', 2000, 0, NULL, 1);
             INSERT INTO message_media VALUES
                 (2, 'Media/WhatsApp Images/x.jpg', 'image/jpeg', 1234);
             INSERT INTO call_log VALUES
                 (1, '393331234567@s.whatsapp.net', 9000, 5, 0, 1, 1),
                 (2, '393331234567@s.whatsapp.net', 500, 9, 0, 0, 0);",
        )
        .unwrap();
        drop(con);
        (Msgstore::open(&path).unwrap(), path)
    }

    #[test]
    fn chats_with_the_same_last_event_come_out_in_the_same_order_every_run() {
        let path = std::env::temp_dir().join(format!("wexport-ties-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let con = Connection::open(&path).unwrap();
        con.execute_batch(
            "CREATE TABLE message (_id INTEGER PRIMARY KEY, key_remote_jid TEXT,
                 timestamp INTEGER, from_me INTEGER, data TEXT);
             INSERT INTO message VALUES
                 (1, '999@lid', 5000, 0, 'b'),
                 (2, '111@s.whatsapp.net', 5000, 0, 'a'),
                 (3, '555@g.us', 5000, 0, 'c');",
        )
        .unwrap();
        drop(con);
        let store = Msgstore::open(&path).unwrap();
        let jids: Vec<String> = store
            .chat_stats()
            .unwrap()
            .into_iter()
            .map(|stat| stat.chat.jid)
            .collect();
        assert_eq!(
            jids,
            ["111@s.whatsapp.net", "555@g.us", "999@lid"],
            "equal timestamps are ordered by jid, never by hash order"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn media_kinds_come_from_the_numeric_type_first_then_the_mime() {
        let row = |path: &str, name: &str, mime: &str| MediaRow {
            rel_path: path.into(),
            name: name.into(),
            mime: if mime.is_empty() {
                None
            } else {
                Some(mime.into())
            },
            ..MediaRow::default()
        };
        let numeric = |kind: i64| media_kind(&row("x", "x", ""), Some(kind));
        assert_eq!(numeric(1), "image");
        assert_eq!(numeric(2), "audio");
        assert_eq!(numeric(3), "video");
        assert_eq!(numeric(4), "contact");
        assert_eq!(numeric(5), "location");
        assert_eq!(numeric(8), "document");
        // A document whose mime says video is still a document: the phone's type wins.
        assert_eq!(
            media_kind(&row("Media/Documents/x.mp4", "x.mp4", "video/mp4"), Some(8)),
            "document"
        );
        assert_eq!(numeric(13), "gif");
        assert_eq!(numeric(20), "sticker");
        assert_eq!(
            numeric(99),
            "document",
            "an unknown type falls through to the mime"
        );

        assert_eq!(
            media_kind(&row("Media/x.jpg", "", "image/jpeg"), None),
            "image"
        );
        assert_eq!(
            media_kind(&row("Media/Stickers/x.webp", "", "image/webp"), None),
            "sticker"
        );
        assert_eq!(
            media_kind(&row("Media/x.webp", "sticker-1.webp", "image/webp"), None),
            "sticker"
        );
        assert_eq!(
            media_kind(&row("Media/x.mp4", "", "video/mp4"), None),
            "video"
        );
        assert_eq!(
            media_kind(
                &row("Media/WhatsApp Animated Gifs/x.mp4", "", "video/mp4"),
                None
            ),
            "gif"
        );
        assert_eq!(
            media_kind(&row("Media/x.mp4", "GIF-2026.mp4", "video/mp4"), None),
            "gif"
        );
        assert_eq!(
            media_kind(&row("Media/x.opus", "", "audio/ogg"), None),
            "audio"
        );
        assert_eq!(
            media_kind(&row("Media/x.pdf", "", "application/pdf"), None),
            "document"
        );
    }

    #[test]
    fn a_shared_contact_is_a_contact_and_a_name_is_read_for_it() {
        let (store, path) = rich_store();
        let message = store
            .messages(&store.chat_stats().unwrap()[0].chat)
            .unwrap()
            .into_iter()
            .find(|message| message.id == 13)
            .expect("the vcard row is there");
        assert_eq!(message.kind, "contact");
        assert_eq!(
            message
                .payload
                .as_ref()
                .and_then(|payload| payload.get("name")),
            Some(&serde_json::json!("Bob"))
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_quote_and_an_edit_are_read_only_when_they_mean_something() {
        let (store, path) = rich_store();
        let messages = store
            .messages(&store.chat_stats().unwrap()[0].chat)
            .unwrap();
        let by_id = |id: i64| messages.iter().find(|message| message.id == id).unwrap();

        assert_eq!(by_id(1).quoted_id, None, "quoted_row_id 0 is not a reply");
        assert_eq!(by_id(2).quoted_id, Some(1));
        assert!(!by_id(1).edited, "edit_version 0 is not an edit");
        assert!(by_id(3).edited);
        let _ = std::fs::remove_file(path);
    }

    /// A database with the modern join, a vcard, a quote, an edit and a call without values.
    fn rich_store() -> (Msgstore, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Cargo runs these tests as threads: the pid alone is the same for all of them.
        let path = std::env::temp_dir().join(format!(
            "wexport-rich-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let con = Connection::open(&path).unwrap();
        con.execute_batch(
            "CREATE TABLE jid (_id INTEGER PRIMARY KEY, raw_string TEXT);
             CREATE TABLE chat (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, subject TEXT);
             CREATE TABLE message (_id INTEGER PRIMARY KEY, chat_row_id INTEGER, timestamp INTEGER,
                 from_me INTEGER, data TEXT, quoted_row_id INTEGER, edit_version INTEGER DEFAULT 0,
                 sender_jid_row_id INTEGER);
             CREATE TABLE message_vcard (message_row_id INTEGER PRIMARY KEY, display_name TEXT);
             CREATE TABLE call_log (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, timestamp INTEGER,
                 duration INTEGER, video_call INTEGER, missed_call INTEGER, from_me INTEGER);
             INSERT INTO jid VALUES (1, '120363@g.us'), (2, '393331234567@s.whatsapp.net');
             INSERT INTO chat VALUES (1, 1, 'Family');
             INSERT INTO message VALUES
                 (1, 1, 1000, 0, 'first', 0, 0, 2),
                 (2, 1, 2000, 0, 'reply', 1, 0, 2),
                 (3, 1, 3000, 0, 'edited', NULL, 2, 2),
                 (13, 1, 4000, 0, NULL, NULL, 0, 2);
             INSERT INTO message_vcard VALUES (13, 'Bob');
             INSERT INTO call_log VALUES (1, 1, 5000, NULL, NULL, NULL, NULL);",
        )
        .unwrap();
        drop(con);
        (Msgstore::open(&path).unwrap(), path)
    }

    #[test]
    fn a_group_author_is_read_from_the_jid_row_when_there_is_no_remote_resource() {
        let path = std::env::temp_dir().join(format!("wexport-author-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let con = Connection::open(&path).unwrap();
        con.execute_batch(
            "CREATE TABLE jid (_id INTEGER PRIMARY KEY, raw_string TEXT);
             CREATE TABLE chat (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, subject TEXT);
             CREATE TABLE message (_id INTEGER PRIMARY KEY, chat_row_id INTEGER, timestamp INTEGER,
                 from_me INTEGER, data TEXT, sender_jid_row_id INTEGER);
             INSERT INTO jid VALUES (1, '120363@g.us'), (2, '393331234567@s.whatsapp.net');
             INSERT INTO chat VALUES (1, 1, 'Family');
             INSERT INTO message VALUES (1, 1, 1000, 0, 'ciao', 2);",
        )
        .unwrap();
        drop(con);
        let store = Msgstore::open(&path).unwrap();
        let stats = store.chat_stats().unwrap();
        let messages = store.messages(&stats[0].chat).unwrap();
        assert_eq!(
            messages[0].sender.as_deref(),
            Some("393331234567"),
            "a group message names its author by the jid row"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_older_schema_groups_by_jid_and_counts_its_media() {
        let (store, path) = legacy_store();
        let stats = store.chat_stats().unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].chat.jid, "393331234567@s.whatsapp.net");
        assert_eq!(stats[0].messages, 2);
        assert_eq!(stats[0].media_count, 1);
        assert_eq!(stats[0].media_bytes, 1234);
        assert_eq!(stats[0].first_ts, 1000);
        assert_eq!(stats[0].last_ts, 9000, "the call is the newest event");

        let messages = store.messages(&stats[0].chat).unwrap();
        assert_eq!(messages.len(), 4);
        // The call table is read on its own, so its rows must still arrive in time order.
        let order: Vec<(&str, i64)> = messages
            .iter()
            .map(|message| (message.kind.as_str(), message.ts))
            .collect();
        assert_eq!(
            order,
            [
                ("call", 500),
                ("text", 1000),
                ("image", 2000),
                ("call", 9000)
            ]
        );
        assert!(messages[3].payload.is_some());
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod range_and_contacts {
    use super::*;

    fn scratch(name: &str) -> (std::path::PathBuf, Connection) {
        let path = std::env::temp_dir().join(format!(
            "wexport-{name}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        let con = Connection::open(&path).unwrap();
        (path, con)
    }

    #[test]
    fn a_chat_whose_only_event_is_a_call_has_that_call_as_its_range() {
        let (path, con) = scratch("call-range");
        con.execute_batch(
            "CREATE TABLE jid (_id INTEGER PRIMARY KEY, raw_string TEXT);
             CREATE TABLE chat (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, subject TEXT);
             CREATE TABLE message (_id INTEGER PRIMARY KEY, chat_row_id INTEGER, timestamp INTEGER);
             CREATE TABLE call_log (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, timestamp INTEGER,
                 duration INTEGER, video_call INTEGER, missed_call INTEGER, from_me INTEGER);
             INSERT INTO jid VALUES (1, '393331234567@s.whatsapp.net');
             INSERT INTO chat VALUES (1, 1, 'Dad');
             INSERT INTO call_log VALUES (1, 1, 5000, NULL, NULL, NULL, 1);",
        )
        .unwrap();
        drop(con);
        let store = Msgstore::open(&path).unwrap();
        let stats = store.chat_stats().unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].messages, 0);
        assert_eq!(stats[0].last_ts, 5000);
        assert_eq!(stats[0].first_ts, 5000, "the call is the whole range");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_call_with_empty_columns_still_renders() {
        let (path, con) = scratch("call-empty");
        con.execute_batch(
            "CREATE TABLE jid (_id INTEGER PRIMARY KEY, raw_string TEXT);
             CREATE TABLE chat (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, subject TEXT);
             CREATE TABLE message (_id INTEGER PRIMARY KEY, chat_row_id INTEGER, timestamp INTEGER);
             CREATE TABLE call_log (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, timestamp INTEGER,
                 duration INTEGER, video_call INTEGER, missed_call INTEGER, from_me INTEGER);
             INSERT INTO jid VALUES (1, '393331234567@s.whatsapp.net');
             INSERT INTO chat VALUES (1, 1, 'Dad');
             INSERT INTO call_log VALUES (1, 1, 5000, NULL, NULL, NULL, NULL);",
        )
        .unwrap();
        drop(con);
        let store = Msgstore::open(&path).unwrap();
        let stats = store.chat_stats().unwrap();
        let messages = store.messages(&stats[0].chat).unwrap();
        let call = messages.first().expect("the call is a message");
        let payload = call.payload.as_ref().unwrap();
        assert_eq!(payload["duration_s"], 0);
        assert_eq!(payload["video"], false);
        assert_eq!(payload["missed"], false);
        assert!(!call.from_me, "a NULL from_me is not a call the user made");
        assert_eq!(call.id, -1, "call ids are negative so they never collide");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn contacts_name_the_chats_and_the_column_travels() {
        let (path, con) = scratch("contacts");
        con.execute_batch(
            "CREATE TABLE jid (_id INTEGER PRIMARY KEY, raw_string TEXT);
             CREATE TABLE chat (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, subject TEXT);
             CREATE TABLE message (_id INTEGER PRIMARY KEY, chat_row_id INTEGER, timestamp INTEGER);
             INSERT INTO jid VALUES (1, '393331234567@s.whatsapp.net');
             INSERT INTO chat VALUES (1, 1, NULL);",
        )
        .unwrap();
        drop(con);

        let wa = path.with_file_name("wa.db");
        let _ = std::fs::remove_file(&wa);
        let contacts = Connection::open(&wa).unwrap();
        contacts
            .execute_batch(
                "CREATE TABLE wa_contacts (jid TEXT, wa_name TEXT);
                 INSERT INTO wa_contacts VALUES ('393331234567@s.whatsapp.net', 'Alice');",
            )
            .unwrap();
        drop(contacts);

        let mut store = Msgstore::open(&path).unwrap();
        assert_eq!(store.chat_stats().unwrap()[0].chat.name, "393331234567");
        store.apply_contacts(&wa);
        assert_eq!(
            store.chat_stats().unwrap()[0].chat.name,
            "Alice",
            "wa_name is the fallback column"
        );
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(wa);
    }

    #[test]
    fn contacts_without_a_jid_column_change_nothing() {
        let (path, con) = scratch("contacts-no-jid");
        con.execute_batch(
            "CREATE TABLE jid (_id INTEGER PRIMARY KEY, raw_string TEXT);
             CREATE TABLE chat (_id INTEGER PRIMARY KEY, jid_row_id INTEGER, subject TEXT);
             CREATE TABLE message (_id INTEGER PRIMARY KEY, chat_row_id INTEGER, timestamp INTEGER);
             INSERT INTO jid VALUES (1, '393331234567@s.whatsapp.net');
             INSERT INTO chat VALUES (1, 1, NULL);",
        )
        .unwrap();
        drop(con);

        let wa = path.with_file_name("wa-no-jid.db");
        let _ = std::fs::remove_file(&wa);
        let contacts = Connection::open(&wa).unwrap();
        contacts
            .execute_batch(
                "CREATE TABLE wa_contacts (raw_string TEXT, wa_name TEXT);
                 INSERT INTO wa_contacts VALUES ('393331234567@s.whatsapp.net', 'Alice');",
            )
            .unwrap();
        drop(contacts);

        let mut store = Msgstore::open(&path).unwrap();
        store.apply_contacts(&wa);
        assert_eq!(store.chat_stats().unwrap()[0].chat.name, "393331234567");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(wa);
    }

    #[test]
    fn a_row_written_in_the_c_us_form_belongs_to_the_same_person() {
        let (path, con) = scratch("c-us");
        con.execute_batch(
            "CREATE TABLE message (_id INTEGER PRIMARY KEY, key_remote_jid TEXT, timestamp INTEGER,
                 from_me INTEGER, data TEXT, media_wa_type INTEGER);
             CREATE TABLE call_log (_id INTEGER PRIMARY KEY, key_remote_jid TEXT, timestamp INTEGER,
                 duration INTEGER, video_call INTEGER, missed_call INTEGER, from_me INTEGER);
             INSERT INTO message VALUES
                 (1, '393331234567@s.whatsapp.net', 1000, 0, 'hello', 0),
                 (2, '393331234567@c.us', 2000, 0, 'the old form', 0);
             INSERT INTO call_log VALUES (1, '393331234567@c.us', 9000, 5, 0, 1, 1);",
        )
        .unwrap();
        drop(con);

        let store = Msgstore::open(&path).unwrap();
        let chat = store
            .chat_stats()
            .unwrap()
            .into_iter()
            .find(|stat| stat.chat.jid == "393331234567@s.whatsapp.net")
            .expect("the chat is listed")
            .chat;
        let messages = store.messages(&chat).unwrap();
        assert_eq!(
            messages.iter().filter(|m| m.kind != "call").count(),
            2,
            "both jid forms are this person's messages"
        );
        assert!(
            messages.iter().any(|m| m.kind == "call"),
            "the call stored in the other form is this chat's too"
        );
        let _ = std::fs::remove_file(path);
    }
}
