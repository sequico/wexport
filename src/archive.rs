//! Turning one chat into a self-contained archive folder, and the index that lists them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::chunks::{self, Record};
use crate::error::{Error, Result};
use crate::media;
use crate::msgstore::{Chat, ChatStat, Message, Msgstore};
use crate::progress::Bar;

const INDEX_HTML: &str = include_str!("assets/index.html");
const APP_JS: &str = include_str!("assets/app.js");
const APP_CSS: &str = include_str!("assets/app.css");

pub struct Clock {
    pub tz: jiff::tz::TimeZone,
    pub name: String,
}

impl Clock {
    pub fn system() -> Self {
        if let Ok(name) = std::fs::read_to_string("/etc/timezone") {
            let name = name.trim().to_string();
            if let Ok(tz) = jiff::tz::TimeZone::get(&name) {
                return Clock { tz, name };
            }
        }
        if let Ok(target) = std::fs::canonicalize("/etc/localtime") {
            let text = target.display().to_string();
            if let Some((_, zone)) = text.split_once("zoneinfo/") {
                if let Ok(tz) = jiff::tz::TimeZone::get(zone) {
                    let name = zone.to_string();
                    return Clock { tz, name };
                }
            }
        }
        Clock {
            tz: jiff::tz::TimeZone::UTC,
            name: "UTC".into(),
        }
    }

    /// The wall clock with its explicit offset, rendered once and never recomputed by a viewer.
    pub fn local(&self, epoch_ms: i64) -> String {
        let moment =
            jiff::Timestamp::from_millisecond(epoch_ms).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        moment
            .to_zoned_datetime(self.tz.clone())
            .strftime("%Y-%m-%d %H:%M:%S %:z")
            .to_string()
    }
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub archive: PathBuf,
    pub name: String,
    pub messages: usize,
    pub media: usize,
    pub missing: usize,
    pub failures: Vec<String>,
    pub failure_count: usize,
    pub bytes: u64,
    pub first_ts: i64,
    pub last_ts: i64,
    pub people: usize,
    pub empty: bool,
}

struct Prepared {
    row: Option<Value>,
    failure: Option<String>,
}

/// Everything a chat build needs from its surroundings, so the call stays readable.
pub struct BuildContext<'a> {
    pub out_root: &'a Path,
    pub clock: &'a Clock,
    pub app_version: &'a str,
    pub copy_root: &'a Path,
    pub media_root: Option<&'a Path>,
    pub jobs: usize,
}

const BATCH: usize = 256;
const FAILURE_SAMPLES: usize = 20;

pub fn build_chat(
    store: &Msgstore,
    stat: &ChatStat,
    context: &BuildContext<'_>,
    bar_label: &str,
) -> Result<Summary> {
    let clock = context.clock;
    let chat = &stat.chat;
    let archive = context.out_root.join(archive_name(chat, stat, clock));
    std::fs::create_dir_all(&archive)?;
    // No stale chunk from a previous run: a month that no longer exists in the chat must not stay
    // loadable behind the viewer's back.
    let _ = std::fs::remove_dir_all(archive.join("data"));

    let mut summary = Summary {
        archive: archive.clone(),
        name: chat.name.clone(),
        messages: 0,
        media: 0,
        missing: 0,
        failures: Vec::new(),
        failure_count: 0,
        bytes: 0,
        first_ts: 0,
        last_ts: 0,
        people: 0,
        empty: false,
    };
    let bar = Arc::new(Mutex::new(Bar::new(bar_label, stat.media_count as usize)));
    let data = archive.join("data");
    let mut writer = chunks::Writer::new(&data, chunks::MAX_RECORDS)?;
    let mut people: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut keep: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut batch: Vec<Message> = Vec::with_capacity(BATCH);

    store.each_message(chat, &mut |message| {
        batch.push(message);
        if batch.len() >= BATCH {
            flush(
                &mut batch,
                &mut writer,
                &mut summary,
                &mut people,
                &mut keep,
                context,
                &bar,
            )?;
        }
        Ok(())
    })?;
    flush(
        &mut batch,
        &mut writer,
        &mut summary,
        &mut people,
        &mut keep,
        context,
        &bar,
    )?;
    if let Ok(mut bar) = bar.lock() {
        bar.close();
    }

    if summary.messages == 0 {
        // A rebuild after the messages were deleted on the phone must not leave the old folder
        // behind: it would still show a conversation the user no longer has.
        let _ = std::fs::remove_dir_all(&archive);
        summary.empty = true;
        return Ok(summary);
    }
    summary.people = people.len();

    let manifest = json!({
        "title": chat.name,
        "is_group": chat.is_group,
        "tz": clock.name,
        "tzdata": tzdata_version(),
        "source_app_version": context.app_version,
        "builder": format!("wexport {}", env!("CARGO_PKG_VERSION")),
        "built": jiff::Timestamp::now()
            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
            .strftime("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
        "media": summary.media,
        "missing": summary.missing,
        "people": summary.people + if chat.is_group { 1 } else { 0 },
        "first": stamp(clock, summary.first_ts),
        "last": stamp(clock, summary.last_ts),
    });
    writer.finish(manifest)?;
    std::fs::write(archive.join("index.html"), INDEX_HTML)?;
    std::fs::write(archive.join("app.js"), APP_JS)?;
    std::fs::write(archive.join("app.css"), APP_CSS)?;
    write_readme(&archive, &summary, clock, context.app_version)?;
    prune(&archive, &keep)?;
    summary.bytes = tree_bytes(&archive);
    Ok(summary)
}

/// Media for one batch, then the records for that batch. Memory is the batch, not the chat.
#[allow(clippy::too_many_arguments)]
fn flush(
    batch: &mut Vec<Message>,
    writer: &mut chunks::Writer<'_>,
    summary: &mut Summary,
    people: &mut std::collections::HashSet<String>,
    keep: &mut std::collections::HashSet<String>,
    context: &BuildContext<'_>,
    bar: &Arc<Mutex<Bar>>,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let prepared = prepare_media(
        batch,
        &summary.archive,
        context.copy_root,
        context.media_root,
        context.jobs,
        bar,
    )?;
    for message in batch.drain(..) {
        let result = prepared.get(&message.id);
        if let Some(Prepared { row, failure }) = result {
            if let Some(row) = row {
                summary.media += 1;
                if row.get("mi").and_then(Value::as_i64).unwrap_or(0) != 0 {
                    summary.missing += 1;
                }
                for key in ["p", "th"] {
                    if let Some(value) = row.get(key).and_then(Value::as_str) {
                        keep.insert(value.to_string());
                    }
                }
            }
            if let Some(failure) = failure {
                summary.failure_count += 1;
                if summary.failures.len() < FAILURE_SAMPLES {
                    summary.failures.push(failure.clone());
                }
            }
        }
        if summary.messages == 0 {
            summary.first_ts = message.ts;
        }
        summary.last_ts = message.ts;
        if let Some(sender) = &message.sender {
            people.insert(sender.clone());
        }
        summary.messages += 1;
        let stamp = context.clock.local(message.ts);
        writer.push(Record {
            month: stamp.chars().take(7).collect(),
            ts: message.ts,
            value: record(
                &message,
                &stamp,
                result.and_then(|prepared| prepared.row.clone()),
            ),
        })?;
    }
    Ok(())
}

fn prepare_media(
    messages: &[Message],
    archive: &Path,
    copy_root: &Path,
    media_root: Option<&Path>,
    jobs: usize,
    bar: &Arc<Mutex<Bar>>,
) -> Result<HashMap<i64, Prepared>> {
    let with_media: Vec<&Message> = messages
        .iter()
        .filter(|message| message.media.is_some())
        .collect();
    if with_media.is_empty() {
        return Ok(HashMap::new());
    }
    let workers = jobs.clamp(1, 8).min(with_media.len());
    let chunk_size = with_media.len().div_ceil(workers);
    let mut collected: HashMap<i64, Prepared> = HashMap::new();
    let mut worker_failed = false;

    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for slice in with_media.chunks(chunk_size) {
            let bar = Arc::clone(bar);
            handles.push(scope.spawn(move || {
                let mut out: Vec<(i64, Prepared)> = Vec::new();
                for message in slice {
                    let media = message.media.as_ref().expect("filtered by media");
                    out.push((
                        message.id,
                        one_media(message.id, media, archive, copy_root, media_root, &bar),
                    ));
                }
                out
            }));
        }
        for handle in handles {
            match handle.join() {
                Ok(results) => collected.extend(results),
                // A panicking worker would otherwise drop its files without a word.
                Err(_) => worker_failed = true,
            }
        }
    });
    if worker_failed {
        return Err(Error::input(
            "a media worker stopped unexpectedly; nothing was written for this chat",
        ));
    }
    Ok(collected)
}

fn one_media(
    id: i64,
    media_item: &crate::msgstore::Media,
    archive: &Path,
    copy_root: &Path,
    media_root: Option<&Path>,
    bar: &Arc<Mutex<Bar>>,
) -> Prepared {
    let mut row = Map::new();
    if let Some(name) = Path::new(&media_item.rel_path)
        .file_name()
        .and_then(|value| value.to_str())
    {
        row.insert("nm".into(), json!(name));
    }
    if let Some(mime) = &media_item.mime {
        row.insert("mt".into(), json!(mime));
    }
    let Some(source) = media::source_for(&media_item.rel_path, media_root, copy_root) else {
        row.insert("mi".into(), json!(1));
        if let Ok(mut bar) = bar.lock() {
            bar.step(1);
        }
        return Prepared {
            row: Some(Value::Object(row)),
            failure: None,
        };
    };
    let placed = match media::place(&source, archive, true) {
        Ok(placed) => placed,
        Err(error) => {
            if let Ok(mut bar) = bar.lock() {
                bar.step(1);
            }
            return Prepared {
                row: Some(Value::Object(row)),
                failure: Some(format!("{id}: {}: {}", media_item.rel_path, error)),
            };
        }
    };
    row.insert("p".into(), json!(placed.rel_path));
    row.insert(
        "sz".into(),
        json!(media_item
            .bytes
            .unwrap_or_else(|| std::fs::metadata(&source)
                .map(|meta| meta.len() as i64)
                .unwrap_or(0))),
    );
    let derived = media::derive(&media_item.kind, &source, archive, &placed.sha256);
    if let Ok(mut bar) = bar.lock() {
        bar.step(1);
    }
    match derived {
        Ok(derived) => {
            if let Some(width) = media_item.width.or(derived.width) {
                row.insert("w".into(), json!(width));
            }
            if let Some(height) = media_item.height.or(derived.height) {
                row.insert("h".into(), json!(height));
            }
            if let Some(duration) = media_item.duration_ms.or(derived.duration_ms) {
                row.insert("d".into(), json!(duration));
            }
            if let Some(thumb) = derived.thumb {
                row.insert("th".into(), json!(thumb));
            }
            if let Some(peaks) = derived.peaks {
                row.insert("pk".into(), json!(peaks));
            }
            Prepared {
                row: Some(Value::Object(row)),
                failure: None,
            }
        }
        Err(error) => Prepared {
            row: Some(Value::Object(row)),
            failure: Some(format!("{id}: {}: {}", media_item.rel_path, error)),
        },
    }
}

fn record(message: &Message, stamp: &str, media_row: Option<Value>) -> Value {
    let mut record = Map::new();
    record.insert("i".into(), json!(message.id));
    record.insert("t".into(), json!(message.ts));
    record.insert("s".into(), json!(stamp));
    record.insert("k".into(), json!(message.kind));
    if message.from_me {
        record.insert("f".into(), json!(1));
    }
    if let Some(sender) = &message.sender {
        record.insert("a".into(), json!(sender));
    }
    if !message.text.is_empty() {
        record.insert("x".into(), json!(message.text));
    }
    if let Some(quoted) = message.quoted_id {
        record.insert("q".into(), json!(quoted));
    }
    if message.edited {
        record.insert("e".into(), json!(1));
    }
    if message.revoked {
        record.insert("r".into(), json!(1));
    }
    if let Some(status) = message.status {
        if status != 0 {
            record.insert("st".into(), json!(status));
        }
    }
    if let Some(payload) = &message.payload {
        record.insert("pl".into(), payload.clone());
    }
    if !message.reactions.is_empty() {
        record.insert("rx".into(), json!(message.reactions));
    }
    if let Some(row) = media_row {
        record.insert("md".into(), row);
    }
    Value::Object(record)
}

/// A rendered stamp, or an empty string when the timestamp is unknown.
fn stamp(clock: &Clock, ts: i64) -> String {
    if ts > 0 {
        clock.local(ts)
    } else {
        String::new()
    }
}

fn archive_name(chat: &Chat, stat: &ChatStat, clock: &Clock) -> String {
    let stamp = if stat.last_ts > 0 {
        clock.local(stat.last_ts)[..10].to_string()
    } else {
        "0000-00-00".to_string()
    };
    format!("{stamp}-{}", slug(chat))
}

fn slug(chat: &Chat) -> String {
    let base: String = chat
        .name
        .to_lowercase()
        .replace('+', " ")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    let base = base
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base: String = base.chars().take(40).collect();
    // sha1, four hex digits: the folder name is what a person recognises on disk, and it is part of
    // the archive contract, so it stays what the first implementation produced.
    let mut digest = sha1::Sha1::new();
    use sha1::Digest;
    digest.update(chat.jid.as_bytes());
    let hash = format!("{:x}", digest.finalize());
    format!(
        "{}-{}",
        if base.is_empty() {
            "chat".to_string()
        } else {
            base
        },
        &hash[..4]
    )
}

fn write_readme(archive: &Path, summary: &Summary, clock: &Clock, app_version: &str) -> Result<()> {
    let text = format!(
        "wexport archive — {}\n\nOpen index.html in any browser. No server is needed, nothing needs to be installed, and\nnothing is loaded from the network.\n\nBuilt ......... {} by wexport {}\nSource app .... WhatsApp {}\nTime zone ..... {} (timestamps are fixed strings with their UTC offset)\nMessages ...... {}\nMedia files ... {} ({} referenced but not present in the copied folder)\n\nThis folder is complete on its own: messages, data chunks, images, video and voice notes.\nKeep it as a folder; if you share it, share the whole folder (a zip of it works).\n",
        summary.name,
        jiff::Timestamp::now()
            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
            .strftime("%Y-%m-%dT%H:%M:%S%:z"),
        env!("CARGO_PKG_VERSION"),
        app_version,
        clock.name,
        summary.messages,
        summary.media,
        summary.missing
    );
    std::fs::write(archive.join("README.txt"), text)?;
    Ok(())
}

fn prune(archive: &Path, keep: &std::collections::HashSet<String>) -> Result<()> {
    for folder in ["media", "deriv"] {
        let root = archive.join(folder);
        prune_tree(archive, &root, keep);
        let _ = std::fs::remove_dir(&root);
    }
    Ok(())
}

/// Delete every file this build does not reference, then every folder that became empty.
fn prune_tree(archive: &Path, folder: &Path, keep: &std::collections::HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            prune_tree(archive, &path, keep);
            let _ = std::fs::remove_dir(&path);
        } else if let Ok(relative) = path.strip_prefix(archive) {
            if !keep.contains(relative.to_string_lossy().as_ref()) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn tree_bytes(root: &Path) -> u64 {
    let mut total = 0;
    let mut queue = vec![root.to_path_buf()];
    while let Some(folder) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push(path);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

fn tzdata_version() -> String {
    std::fs::read_to_string("/usr/share/zoneinfo/tzdata.zi")
        .ok()
        .and_then(|text| text.lines().next().map(|line| line.trim().to_string()))
        .unwrap_or_default()
}

pub fn write_index(out_root: &Path, summaries: &[Summary]) -> Result<PathBuf> {
    let items: String = summaries
        .iter()
        .filter(|summary| !summary.empty)
        .map(|summary| {
            let href = summary
                .archive
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            format!(
                "<li><a href=\"{}/index.html\">{}</a> <span>{} messages</span></li>",
                escape_html(&href),
                escape_html(&summary.name),
                summary.messages
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let page = format!(
        "<!doctype html>\n<html lang=\"en\"><meta charset=\"utf-8\"><title>wexport archives</title>\n<style>body{{font:16px/1.5 system-ui,sans-serif;max-width:44rem;margin:4rem auto;padding:0 1rem}}\nli{{margin:.4rem 0}}span{{color:#666;font-size:.9em}}</style>\n<h1>Archives</h1>\n<p>Each folder below is a complete archive: open its <code>index.html</code>. Nothing here needs a\nserver or wexport installed.</p>\n<ul>\n{items}\n</ul>\n<p>Built with wexport {}.</p>\n</html>\n",
        env!("CARGO_PKG_VERSION")
    );
    let target = out_root.join("index.html");
    std::fs::write(&target, page)?;
    Ok(target)
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
