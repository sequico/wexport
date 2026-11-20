//! The iPhone adapter: WhatsApp's Core Data store inside a Finder backup or an extracted app group.
//!
//! Column names are resolved by probing the schema, never by guessing the app version; a rename
//! breaks exactly here and nowhere else.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use crate::discover::{IosInputs, IOS_DOMAIN};
use crate::error::{Error, Result};
use crate::msgstore::{newest_first, Chat, ChatStat, Media, Message};
use crate::source::Store;

/// Seconds between the Unix epoch and Core Data's 2001-01-01.
pub const CORE_DATA_EPOCH: i64 = 978_307_200;

/// The columns of one iOS schema, resolved once at open.
struct Columns {
    chat_pk: String,
    chat_jid: String,
    chat_name: Option<String>,
    message_pk: String,
    message_chat: String,
    message_date: String,
    message_from_me: String,
    message_text: Option<String>,
    message_from_jid: Option<String>,
    message_media: Option<String>,
    message_deleted: Option<String>,
    message_edited: Option<String>,
    message_call: Option<String>,
    media_pk: Option<String>,
    media_path: Option<String>,
    media_thumb: Option<String>,
    media_size: Option<String>,
    media_mime: Option<String>,
    media_latitude: Option<String>,
    media_longitude: Option<String>,
    media_vcard: Option<String>,
    member_chat: Option<String>,
    member_jid: Option<String>,
    member_name: Option<String>,
}

pub struct IosStore {
    con: Connection,
    manifest: Option<Connection>,
    files: Option<PathBuf>,
    app_group: Option<PathBuf>,
    columns: Columns,
    chats: BTreeMap<i64, Chat>,
    members: HashMap<String, String>,
}

impl IosStore {
    pub fn open(inputs: &IosInputs, workspace: &Path) -> Result<Self> {
        let database = copy_database(inputs, workspace)?;
        let con = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| {
                Error::input(format!("cannot open {}: {error}", database.display()))
            })?;

        let tables: Vec<String> = {
            let mut statement = con.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.filter_map(|row| row.ok()).collect()
        };
        let has = |name: &str| tables.iter().any(|table| table == name);
        if !has("ZWAMESSAGE") || !has("ZWACHATSESSION") {
            return Err(Error::input(
                "no ZWAMESSAGE table — this is not WhatsApp's iPhone database",
            ));
        }
        let columns = probe(&con)?;

        let manifest = inputs
            .manifest
            .as_deref()
            .map(|path| copy_manifest(path, workspace))
            .transpose()?;

        let mut store = IosStore {
            con,
            manifest,
            files: inputs.files.clone(),
            app_group: inputs.app_group.clone(),
            columns,
            chats: BTreeMap::new(),
            members: HashMap::new(),
        };
        store.load_chats()?;
        store.load_members()?;
        Ok(store)
    }

    fn load_chats(&mut self) -> Result<()> {
        let c = &self.columns;
        let name_sql = c
            .chat_name
            .as_ref()
            .map(|name| format!("c.\"{name}\""))
            .unwrap_or_else(|| "NULL".into());
        let sql = format!(
            "SELECT c.\"{}\", c.\"{}\", {name_sql} FROM ZWACHATSESSION c",
            c.chat_pk, c.chat_jid
        );
        let mut statement = self.con.prepare(&sql)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let jid: Option<String> = row.get(1)?;
            let name: Option<String> = row.get(2)?;
            let Some(jid) = jid.filter(|value| !value.trim().is_empty()) else {
                continue;
            };
            let title = name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| jid.split('@').next().unwrap_or("").to_string());
            self.chats.insert(
                id,
                Chat {
                    is_group: jid.ends_with("@g.us"),
                    jid,
                    name: title,
                },
            );
        }
        Ok(())
    }

    fn load_members(&mut self) -> Result<()> {
        let c = &self.columns;
        let (Some(chat), Some(jid), Some(name)) = (&c.member_chat, &c.member_jid, &c.member_name)
        else {
            return Ok(());
        };
        let sql = format!("SELECT \"{chat}\", \"{jid}\", \"{name}\" FROM ZWAGROUPMEMBER",);
        let mut statement = self.con.prepare(&sql)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let member: Option<String> = row.get(1)?;
            let name: Option<String> = row.get(2)?;
            if let (Some(member), Some(name)) = (member, name) {
                if !name.trim().is_empty() {
                    self.members.insert(key_of(&member), name);
                }
            }
        }
        Ok(())
    }

    fn media_row(&self, id: i64) -> Option<MediaRow> {
        let c = &self.columns;
        let path = c.media_path.as_ref()?;
        let nullable = |name: &Option<String>| name.clone().unwrap_or_else(|| "NULL".into());
        let sql = format!(
            "SELECT \"{path}\", {}, {}, {}, {}, {}, {} FROM ZWAMEDIAITEM WHERE \"{}\" = ?1",
            nullable(&c.media_thumb),
            nullable(&c.media_size),
            nullable(&c.media_mime),
            nullable(&c.media_latitude),
            nullable(&c.media_longitude),
            nullable(&c.media_vcard),
            c.media_pk.as_deref().unwrap_or("Z_PK"),
        );
        let mut statement = self.con.prepare(&sql).ok()?;
        statement
            .query_row(rusqlite::params![id], |row| {
                Ok(MediaRow {
                    path: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    thumb: row.get(1)?,
                    size: row.get(2)?,
                    mime: row.get(3)?,
                    latitude: row.get(4)?,
                    longitude: row.get(5)?,
                    vcard: row.get(6)?,
                })
            })
            .ok()
    }

    /// Resolve a path from the database to the file inside the backup or the app group.
    fn resolve(&self, relative: &str) -> Option<PathBuf> {
        let relative = relative.replace('\\', "/");
        let relative = relative.trim_start_matches('/');
        if relative.is_empty() {
            return None;
        }
        let mut candidates = vec![relative.to_string()];
        if let Some(tail) = relative.strip_prefix("Media/") {
            candidates.push(format!("Message/Media/{tail}"));
            candidates.push(format!("Library/Media/{tail}"));
        }
        if let (Some(files), Some(manifest)) = (&self.files, &self.manifest) {
            for candidate in &candidates {
                if let Some(path) = manifest_file(manifest, files, candidate) {
                    return Some(path);
                }
            }
            let name = Path::new(relative).file_name()?.to_str()?;
            let sql = "SELECT fileID FROM Files WHERE domain = ?1 AND relativePath LIKE ?2 LIMIT 1";
            if let Ok(file) = manifest.query_row(
                sql,
                rusqlite::params![IOS_DOMAIN, format!("%{name}")],
                |row| row.get::<_, String>(0),
            ) {
                let path = hashed(files, &file);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        if let Some(group) = &self.app_group {
            return crate::media::contain(vec![group.join(relative)], std::slice::from_ref(group));
        }
        None
    }

    fn thumbnail(&self, row: &MediaRow) -> Option<Vec<u8>> {
        let path = row.thumb.as_deref().filter(|value| !value.is_empty())?;
        let file = self.resolve(path)?;
        std::fs::read(&file).ok().filter(|bytes| !bytes.is_empty())
    }

    fn media_of(&self, row: &MediaRow, kind: &str) -> Media {
        Media {
            rel_path: row.path.clone(),
            kind: kind.to_string(),
            mime: row.mime.clone(),
            bytes: row.size,
            width: None,
            height: None,
            duration_ms: None,
            source: self.resolve(&row.path),
            thumbnail: self.thumbnail(row),
        }
    }
}

impl Store for IosStore {
    fn chat_stats(&self) -> Result<Vec<ChatStat>> {
        let c = &self.columns;
        let media_join = match (&c.media_pk, &c.message_media) {
            (Some(pk), Some(message_media)) => {
                format!("LEFT JOIN ZWAMEDIAITEM d ON d.\"{pk}\" = m.\"{message_media}\"")
            }
            _ => String::new(),
        };
        let media_count = match &c.message_media {
            Some(column) => format!("SUM(CASE WHEN m.\"{column}\" IS NOT NULL THEN 1 ELSE 0 END)"),
            None => "0".into(),
        };
        let media_bytes = match (&c.message_media, &c.media_size) {
            (Some(message_media), Some(size)) => format!(
                "SUM(CASE WHEN m.\"{message_media}\" IS NOT NULL THEN COALESCE(d.\"{size}\", 0) ELSE 0 END)"
            ),
            _ => "0".into(),
        };
        let sql = format!(
            "SELECT c.\"{chat_pk}\", COUNT(m.\"{message_pk}\"), MIN(m.\"{date}\"), MAX(m.\"{date}\"), \
             {media_count}, {media_bytes} \
             FROM ZWACHATSESSION c LEFT JOIN ZWAMESSAGE m ON m.\"{message_chat}\" = c.\"{chat_pk}\" \
             {media_join} GROUP BY c.\"{chat_pk}\"",
            chat_pk = c.chat_pk,
            message_pk = c.message_pk,
            date = c.message_date,
            message_chat = c.message_chat,
        );
        let mut statement = self.con.prepare(&sql)?;
        let mut rows = statement.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let Some(chat) = self.chats.get(&id) else {
                continue;
            };
            out.push(ChatStat {
                chat: chat.clone(),
                messages: row.get(1)?,
                first_ts: row.get::<_, Option<i64>>(2)?.map(stamp).unwrap_or(0),
                last_ts: row.get::<_, Option<i64>>(3)?.map(stamp).unwrap_or(0),
                media_count: row.get(4).unwrap_or(0),
                media_bytes: row.get(5).unwrap_or(0),
            });
        }
        Ok(newest_first(out))
    }

    fn each_message(&self, chat: &Chat, sink: &mut dyn FnMut(Message) -> Result<()>) -> Result<()> {
        let id = self
            .chats
            .iter()
            .find(|(_, candidate)| candidate.jid == chat.jid)
            .map(|(id, _)| *id)
            .ok_or_else(|| Error::input(format!("chat {} is not in this backup", chat.jid)))?;
        let c = &self.columns;
        let nullable = |name: &Option<String>| name.clone().unwrap_or_else(|| "NULL".into());
        let sql = format!(
            "SELECT \"{pk}\", \"{date}\", \"{from_me}\", {text}, {from_jid}, {media}, {deleted}, {edited}, {call} \
             FROM ZWAMESSAGE WHERE \"{chat}\" = ?1 ORDER BY \"{date}\", \"{pk}\"",
            pk = c.message_pk,
            date = c.message_date,
            from_me = c.message_from_me,
            text = nullable(&c.message_text),
            from_jid = nullable(&c.message_from_jid),
            media = nullable(&c.message_media),
            deleted = nullable(&c.message_deleted),
            edited = nullable(&c.message_edited),
            call = nullable(&c.message_call),
            chat = c.message_chat,
        );
        let mut statement = self.con.prepare(&sql)?;
        let mut rows = statement.query(rusqlite::params![id])?;
        while let Some(row) = rows.next()? {
            let pk: i64 = row.get(0)?;
            let date: Option<i64> = row.get(1)?;
            let from_me = row.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0;
            let text: Option<String> = row.get(3)?;
            let from_jid: Option<String> = row.get(4)?;
            let media_id: Option<i64> = row.get(5)?;
            let revoked = row.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0;
            let edited = row.get::<_, Option<i64>>(7)?.unwrap_or(0) != 0;
            let call: Option<i64> = row.get(8)?;

            let media_row = media_id.and_then(|media_id| self.media_row(media_id));
            let (kind, payload) = classify(&media_row, call, revoked);
            let sender = if from_me {
                None
            } else {
                from_jid.as_deref().map(|jid| {
                    self.members
                        .get(&key_of(jid))
                        .cloned()
                        .unwrap_or_else(|| key_of(jid))
                })
            };
            let media = media_row.map(|row| self.media_of(&row, &kind));
            sink(Message {
                id: pk,
                ts: date.map(stamp).unwrap_or(0),
                from_me,
                kind: if revoked { "deleted".into() } else { kind },
                text: text.unwrap_or_default(),
                sender,
                quoted_id: None,
                edited,
                revoked,
                status: None,
                payload,
                media,
                reactions: Vec::new(),
            })?;
        }
        Ok(())
    }

    fn media_counts(&self) -> Result<(i64, i64)> {
        let Some(path) = &self.columns.media_path else {
            return Ok((0, 0));
        };
        let sql = format!(
            "SELECT \"{path}\" FROM ZWAMEDIAITEM WHERE \"{path}\" IS NOT NULL AND \"{path}\" <> ''"
        );
        let mut statement = self.con.prepare(&sql)?;
        let mut rows = statement.query([])?;
        let mut referenced = 0;
        let mut present = 0;
        while let Some(row) = rows.next()? {
            let relative: String = row.get(0)?;
            referenced += 1;
            if self.resolve(&relative).is_some() {
                present += 1;
            }
        }
        Ok((present, referenced))
    }

    fn integrity_check(&self) -> Result<()> {
        let verdict: String = self
            .con
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if verdict == "ok" {
            Ok(())
        } else {
            Err(Error::incomplete(format!(
                "the database is damaged: {verdict}"
            )))
        }
    }

    fn diagnostics(&self) -> Vec<String> {
        let c = &self.columns;
        vec![
            format!("source: iphone backup, {} chat(s) read", self.chats.len()),
            format!(
                "chat: id={} jid={} name={:?}",
                c.chat_pk, c.chat_jid, c.chat_name
            ),
            format!(
                "message: id={} chat={} date={} media={:?}",
                c.message_pk, c.message_chat, c.message_date, c.message_media
            ),
            format!(
                "media: path={:?} thumbnail={:?} size={:?}",
                c.media_path, c.media_thumb, c.media_size
            ),
            format!(
                "manifest: {}",
                if self.manifest.is_some() {
                    "Manifest.db"
                } else {
                    "an extracted app group"
                }
            ),
        ]
    }
}

struct MediaRow {
    path: String,
    thumb: Option<String>,
    size: Option<i64>,
    mime: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    vcard: Option<String>,
}

/// The kind of a message, and the payload the viewer needs for the ones that are not a file.
fn classify(row: &Option<MediaRow>, call: Option<i64>, revoked: bool) -> (String, Option<Value>) {
    if revoked {
        return ("deleted".into(), None);
    }
    if call.unwrap_or(0) != 0 {
        return (
            "call".into(),
            Some(json!({ "duration_s": 0, "video": false, "missed": false })),
        );
    }
    let Some(row) = row else {
        return ("text".into(), None);
    };
    if let Some(name) = row.vcard.as_deref().filter(|name| !name.trim().is_empty()) {
        return ("contact".into(), Some(json!({ "name": name })));
    }
    if let (Some(latitude), Some(longitude)) = (row.latitude, row.longitude) {
        if latitude != 0.0 || longitude != 0.0 {
            return (
                "location".into(),
                Some(json!({ "lat": latitude, "lon": longitude })),
            );
        }
    }
    (path_kind(&row.path, row.mime.as_deref()), None)
}

fn path_kind(path: &str, mime: Option<&str>) -> String {
    let lower = path.to_lowercase();
    let mime = mime.unwrap_or("").to_lowercase();
    let picture = mime.starts_with("image/")
        || [".jpg", ".jpeg", ".png", ".webp", ".heic"]
            .iter()
            .any(|suffix| lower.ends_with(suffix));
    if picture {
        return if lower.contains("sticker") {
            "sticker"
        } else {
            "image"
        }
        .to_string();
    }
    if mime.starts_with("video/") || lower.ends_with(".mp4") || lower.ends_with(".mov") {
        return if lower.contains("gif") {
            "gif"
        } else {
            "video"
        }
        .to_string();
    }
    if mime.starts_with("audio/") || lower.ends_with(".opus") || lower.ends_with(".m4a") {
        return "audio".to_string();
    }
    "document".to_string()
}

fn probe(con: &Connection) -> Result<Columns> {
    let columns = |table: &str| -> Vec<String> {
        let Ok(mut statement) = con.prepare(&format!("PRAGMA table_info(\"{table}\")")) else {
            return Vec::new();
        };
        let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(1)) else {
            return Vec::new();
        };
        rows.filter_map(|row| row.ok()).collect()
    };
    let pick = |table: &str, candidates: &[&str]| -> Option<String> {
        let names = columns(table);
        candidates
            .iter()
            .find(|candidate| names.iter().any(|name| name == *candidate))
            .map(|candidate| (*candidate).to_string())
    };
    let require = |table: &str, candidates: &[&str]| -> Result<String> {
        pick(table, candidates).ok_or_else(|| {
            Error::input(format!(
                "table {table} has none of the columns {}; the schema is newer or older than this build understands",
                candidates.join(", ")
            ))
        })
    };
    let message = "ZWAMESSAGE";
    let media = "ZWAMEDIAITEM";
    let member = "ZWAGROUPMEMBER";
    let media_table = pick(media, &["Z_PK"]).is_some();
    Ok(Columns {
        chat_pk: require("ZWACHATSESSION", &["Z_PK"])?,
        chat_jid: require("ZWACHATSESSION", &["ZCONTACTJID", "ZJID"])?,
        chat_name: pick("ZWACHATSESSION", &["ZPARTNERNAME", "ZNAME", "ZDISPLAYNAME"]),
        message_pk: require(message, &["Z_PK"])?,
        message_chat: require(message, &["ZCHATSESSION", "ZCHAT"])?,
        message_date: require(message, &["ZMESSAGEDATE", "ZDATE"])?,
        message_from_me: require(message, &["ZISFROMME", "ZFROM_ME"])?,
        message_text: pick(message, &["ZTEXT", "ZMESSAGE"]),
        message_from_jid: pick(message, &["ZFROMJID", "ZSENDERJID"]),
        message_media: pick(message, &["ZMEDIAITEM", "ZMEDIA"]),
        message_deleted: pick(message, &["ZDELETED", "ZISDELETED"]),
        message_edited: pick(message, &["ZEDITED", "ZISEDITED"]),
        message_call: pick(message, &["ZCALLTYPE", "ZCALLRESULT"]),
        media_pk: media_table.then(|| pick(media, &["Z_PK"])).flatten(),
        media_path: media_table
            .then(|| pick(media, &["ZMEDIALOCALPATH", "ZPATH"]))
            .flatten(),
        media_thumb: media_table
            .then(|| pick(media, &["ZTHUMBNAILLOCALPATH"]))
            .flatten(),
        media_size: media_table.then(|| pick(media, &["ZFILESIZE"])).flatten(),
        media_mime: media_table
            .then(|| pick(media, &["ZMIMETYPE", "ZMEDIAMIMETYPE"]))
            .flatten(),
        media_latitude: media_table.then(|| pick(media, &["ZLATITUDE"])).flatten(),
        media_longitude: media_table.then(|| pick(media, &["ZLONGITUDE"])).flatten(),
        media_vcard: media_table.then(|| pick(media, &["ZVCARDNAME"])).flatten(),
        member_chat: pick(member, &["ZCHATSESSION"]),
        member_jid: pick(member, &["ZMEMBERJID"]),
        member_name: pick(member, &["ZCONTACTNAME", "ZNAME"]),
    })
}

fn stamp(core_data_seconds: i64) -> i64 {
    core_data_seconds
        .saturating_add(CORE_DATA_EPOCH)
        .saturating_mul(1000)
}

fn key_of(jid: &str) -> String {
    jid.split('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
}

fn hashed(files: &Path, file_id: &str) -> PathBuf {
    let prefix: String = file_id.chars().take(2).collect();
    files.join(prefix).join(file_id)
}

fn manifest_file(manifest: &Connection, files: &Path, relative: &str) -> Option<PathBuf> {
    let sql = "SELECT fileID FROM Files WHERE domain = ?1 AND relativePath = ?2 LIMIT 1";
    let file: String = manifest
        .query_row(sql, rusqlite::params![IOS_DOMAIN, relative], |row| {
            row.get(0)
        })
        .ok()?;
    let path = hashed(files, &file);
    path.is_file().then_some(path)
}

/// The database and its log, copied into the writable workspace so SQLite can replay the WAL.
fn copy_database(inputs: &IosInputs, workspace: &Path) -> Result<PathBuf> {
    let target = workspace.join("ChatStorage.sqlite");
    std::fs::copy(&inputs.database, &target)?;
    for (suffix, from_manifest) in [("-wal", &inputs.wal), ("-shm", &inputs.shm)] {
        let source = from_manifest.clone().or_else(|| {
            let beside = inputs
                .database
                .with_file_name(format!("ChatStorage.sqlite{suffix}"));
            beside.is_file().then_some(beside)
        });
        if let Some(source) = source.filter(|path| path.is_file()) {
            std::fs::copy(
                source,
                workspace.join(format!("ChatStorage.sqlite{suffix}")),
            )?;
        }
    }
    Ok(target)
}

fn copy_manifest(manifest: &Path, workspace: &Path) -> Result<Connection> {
    let target = workspace.join("Manifest.db");
    std::fs::copy(manifest, &target)?;
    for suffix in ["-wal", "-shm"] {
        let companion = manifest.with_file_name(format!("Manifest.db{suffix}"));
        if companion.is_file() {
            std::fs::copy(&companion, workspace.join(format!("Manifest.db{suffix}")))?;
        }
    }
    Connection::open_with_flags(&target, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| Error::input(format!("cannot open the backup manifest: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_data_seconds_become_unix_milliseconds() {
        // 2026-12-01T09:00:00Z, the fixture's first December message.
        assert_eq!(stamp(817_808_400), 1_796_115_600_000);
        assert_eq!(stamp(0), 978_307_200_000);
    }

    #[test]
    fn a_media_path_is_its_kind() {
        assert_eq!(path_kind("Message/Media/a.jpg", None), "image");
        assert_eq!(path_kind("Message/Media/sticker.webp", None), "sticker");
        assert_eq!(path_kind("Message/Media/a.mp4", None), "video");
        assert_eq!(path_kind("Message/Media/GIF-x.mp4", None), "gif");
        assert_eq!(path_kind("Message/Media/a.opus", None), "audio");
        assert_eq!(path_kind("Message/Media/a.pdf", None), "document");
        assert_eq!(
            path_kind("Message/Media/no-extension", Some("image/jpeg")),
            "image",
            "the mime answers when the name does not"
        );
    }

    #[test]
    fn a_deleted_message_keeps_no_media() {
        let (kind, _) = classify(&None, None, true);
        assert_eq!(kind, "deleted");
        let (kind, payload) = classify(&None, Some(1), false);
        assert_eq!(kind, "call");
        assert!(payload.is_some());
    }
}
