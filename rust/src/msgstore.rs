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
        if columns.is_empty() || !columns.iter().any(|name| name == "jid") {
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

    pub fn chats(&self) -> Vec<Chat> {
        if !self.chats.is_empty() {
            let mut out: Vec<Chat> = self.chats.values().cloned().collect();
            out.sort_by_key(|chat| chat.name.to_lowercase());
            return out;
        }
        let Some(column) = &self.c_chat_jid else {
            return Vec::new();
        };
        let mut statement = match self
            .con
            .prepare(&format!("SELECT DISTINCT \"{column}\" FROM message"))
        {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };
        let Ok(rows) = statement.query_map([], |row| row.get::<_, Option<String>>(0)) else {
            return Vec::new();
        };
        let mut seen: BTreeMap<String, Chat> = BTreeMap::new();
        for raw in rows.flatten().flatten() {
            let jid = raw.trim().to_string();
            if jid.is_empty() {
                continue;
            }
            let name = jid.split('@').next().unwrap_or("").to_string();
            seen.insert(
                jid.clone(),
                Chat {
                    is_group: jid.ends_with("@g.us"),
                    jid,
                    name,
                },
            );
        }
        seen.into_values().collect()
    }

    pub fn chat_stats(&self) -> Result<Vec<ChatStat>> {
        if let (Some(column), false) = (&self.c_chat_row, self.chats.is_empty()) {
            let sql = format!(
                "SELECT m.\"{column}\", COUNT(*), MIN(m.\"{ts}\"), MAX(m.\"{ts}\") FROM message m GROUP BY 1",
                ts = self.c_ts
            );
            let mut statement = self.con.prepare(&sql)?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })?;
            let mut totals: HashMap<i64, (i64, i64, i64)> = HashMap::new();
            for (chat_id, count, first, last) in rows.flatten() {
                totals.insert(chat_id, (count, first.unwrap_or(0), last.unwrap_or(0)));
            }

            let mut media: HashMap<i64, (i64, i64)> = HashMap::new();
            if let (Some(message_col), Some(size_col)) = (&self.m_message, &self.m_bytes) {
                let sql = format!(
                    "SELECT m.\"{column}\", COUNT(*), COALESCE(SUM(d.\"{size_col}\"), 0) FROM message_media d JOIN message m ON m.\"{}\" = d.\"{message_col}\" GROUP BY 1",
                    self.c_id
                );
                if let Ok(mut statement) = self.con.prepare(&sql) {
                    if let Ok(rows) = statement.query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    }) {
                        for (chat_id, count, size) in rows.flatten() {
                            media.insert(chat_id, (count, size));
                        }
                    }
                }
            }

            let mut out = Vec::new();
            for (chat_id, chat) in &self.chats {
                let (count, first, last) = totals.get(chat_id).copied().unwrap_or((0, 0, 0));
                let last = self
                    .jid_row
                    .get(chat_id)
                    .map(|jid| last.max(self.call_time(*jid)))
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
            return Ok(out);
        }

        let Some(column) = &self.c_chat_jid else {
            return Err(Error::input("cannot tell which chat a message belongs to"));
        };
        let sql = format!(
            "SELECT m.\"{column}\", COUNT(*), MIN(m.\"{ts}\"), MAX(m.\"{ts}\") FROM message m GROUP BY 1",
            ts = self.c_ts
        );
        let mut statement = self.con.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for (jid, count, first, last) in rows.flatten() {
            let Some(jid) = jid
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let name = jid.split('@').next().unwrap_or("").to_string();
            out.push(ChatStat {
                chat: Chat {
                    is_group: jid.ends_with("@g.us"),
                    name,
                    jid,
                },
                messages: count,
                first_ts: first.unwrap_or(0),
                last_ts: last.unwrap_or(0),
                media_count: 0,
                media_bytes: 0,
            });
        }
        Ok(out)
    }

    fn call_time(&self, jid_row: i64) -> i64 {
        if !self.tables.contains("call_log") {
            return 0;
        }
        let columns = self.columns_of("call_log");
        if !columns.iter().any(|name| name == "jid_row_id")
            || !columns.iter().any(|name| name == "timestamp")
        {
            return 0;
        }
        self.con
            .query_row(
                "SELECT MAX(timestamp) FROM call_log WHERE jid_row_id = ?1",
                [jid_row],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    pub fn messages(&self, chat: &Chat) -> Result<Vec<Message>> {
        let alias = |name: &str, column: &Option<String>, default: &str| match column {
            Some(column) => format!("m.\"{column}\" AS {name}"),
            None => format!("{default} AS {name}"),
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
        ];
        let (where_sql, params) = self.chat_filter(chat);
        let sql = format!(
            "SELECT {} FROM message m WHERE {where_sql} ORDER BY m.\"{}\", m.\"{}\"",
            select.join(", "),
            self.c_ts,
            self.c_id
        );
        let mut statement = self.con.prepare(&sql)?;
        let mut rows = statement.query(rusqlite::params_from_iter(params))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let ts: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
            let from_me = row.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0;
            let text: Option<String> = row.get(3)?;
            let status: Option<i64> = row.get(4)?;
            let remote: Option<String> = row.get(5)?;
            let quoted: Option<i64> = row.get(6)?;
            // 0 means "not a reply" in every schema WhatsApp has shipped; carrying it would put a
            // quote pointing at message 0 in the archive.
            let quoted = quoted.filter(|value| *value > 0);            let edited = row.get::<_, Option<i64>>(7)?.unwrap_or(0) > 0;
            let lat: Option<f64> = row.get(8)?;
            let lon: Option<f64> = row.get(9)?;
            let wa_type: Option<i64> = row.get(10)?;

            let media = self.media_of(id, wa_type);
            let caption = self
                .media_rows
                .get(&id)
                .and_then(|row| row.caption.clone())
                .unwrap_or_default();
            let (kind, payload) = self.classify(id, &media, lat, lon);
            let revoked = self.revoked.contains(&id);
            out.push(Message {
                id,
                ts,
                from_me,
                kind: if revoked { "deleted".to_string() } else { kind },
                text: text.filter(|value| !value.is_empty()).unwrap_or(caption),
                sender: if from_me {
                    None
                } else {
                    remote.as_deref().map(|value| self.person_name(value))
                },
                quoted_id: quoted,
                edited,
                revoked,
                status,
                payload,
                media,
                reactions: self.reactions.get(&id).cloned().unwrap_or_default(),
            });
        }
        drop(rows);
        out.extend(self.calls(chat)?);
        Ok(out)
    }

    fn chat_filter(&self, chat: &Chat) -> (String, Vec<SqlValue>) {
        if let (Some(column), false) = (&self.c_chat_row, self.chats.is_empty()) {
            if let Some((id, _)) = self
                .chats
                .iter()
                .find(|(_, candidate)| candidate.jid == chat.jid)
            {
                return (format!("m.\"{column}\" = ?"), vec![SqlValue::Integer(*id)]);
            }
            return ("0".to_string(), Vec::new());
        }
        match &self.c_chat_jid {
            Some(column) => {
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
            None => ("0".to_string(), Vec::new()),
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

    fn calls(&self, chat: &Chat) -> Result<Vec<Message>> {
        let mut out = Vec::new();
        if !self.tables.contains("call_log") {
            return Ok(out);
        }
        let columns = self.columns_of("call_log");
        if !columns.iter().any(|name| name == "timestamp") {
            return Ok(out);
        }
        let Some(jid_col) = ["jid_row_id", "key_remote_jid"]
            .into_iter()
            .find(|name| columns.iter().any(|column| column == name))
        else {
            return Ok(out);
        };
        let chat_row = self
            .chats
            .iter()
            .find(|(_, candidate)| candidate.jid == chat.jid)
            .map(|(id, _)| *id);
        let Some(chat_row) = chat_row else {
            return Ok(out);
        };
        let Some(jid_row) = self.jid_row.get(&chat_row).copied() else {
            return Ok(out);
        };

        let find = |names: &[&str]| {
            names
                .iter()
                .find(|name| columns.iter().any(|column| column == *name))
                .map(|name| name.to_string())
        };
        let duration = find(&["duration", "call_duration"]);
        let video = find(&["video_call", "is_video"]);
        let missed = find(&["missed_call", "is_missed"]);
        let has_from_me = columns.iter().any(|name| name == "from_me");

        let mut statement = self.con.prepare(&format!(
            "SELECT * FROM call_log WHERE \"{jid_col}\" = ?1 ORDER BY timestamp"
        ))?;
        let mut rows = statement.query([jid_row])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get("_id")?;
            let ts: i64 = row.get::<_, Option<i64>>("timestamp")?.unwrap_or(0);
            let read = |column: &Option<String>| -> i64 {
                column
                    .as_ref()
                    .and_then(|name| row.get::<_, Option<i64>>(name.as_str()).ok().flatten())
                    .unwrap_or(0)
            };
            out.push(Message {
                id: -id,
                ts,
                from_me: has_from_me && row.get::<_, Option<i64>>("from_me")?.unwrap_or(0) != 0,
                kind: "call".to_string(),
                text: String::new(),
                sender: None,
                quoted_id: None,
                edited: false,
                revoked: false,
                status: None,
                payload: Some(json!({
                    "duration_s": read(&duration),
                    "video": read(&video) != 0,
                    "missed": read(&missed) != 0,
                })),
                media: None,
                reactions: Vec::new(),
            });
        }
        Ok(out)
    }
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
