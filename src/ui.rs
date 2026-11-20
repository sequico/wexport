//! The command itself: parse the flags, discover, check, list, build, report.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::archive::{self, Clock};
use crate::crypt15;
use crate::discover::{self, Detected, Source};
use crate::error::{Error, Result, EXIT_INCOMPLETE};
use crate::ios::IosStore;
use crate::keys;
use crate::merge;
use crate::msgstore::{ChatStat, Msgstore};
use crate::report::{ChatRow, Event, Finished, Report};
use crate::source::Store;

const OUTPUT_DIR: &str = "output";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
struct Options {
    folder: Option<PathBuf>,
    key: Option<String>,
    chats: Vec<usize>,
    all: bool,
    list: bool,
    out: Option<PathBuf>,
    zone: Option<String>,
    sort: Sort,
    reverse: bool,
    limit: Option<usize>,
    json: bool,
    quiet: bool,
    diagnostics: bool,
    dry_run: bool,
    check: bool,
}

/// The order of the listing, and of the build that follows it.
#[derive(Debug, Clone, Copy)]
enum Sort {
    Recent,
    Size,
    Name,
}

impl Sort {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "recent" => Ok(Sort::Recent),
            "size" => Ok(Sort::Size),
            "name" => Ok(Sort::Name),
            other => Err(Error::input(format!(
                "unknown sort {other}: use recent, size or name"
            ))),
        }
    }
}

fn sort_stats(stats: &mut [ChatStat], sort: Sort, reverse: bool) {
    match sort {
        Sort::Recent => {
            stats.sort_by_key(|stat| std::cmp::Reverse(stat.last_ts));
        }
        Sort::Size => {
            stats.sort_by(|left, right| {
                right
                    .media_bytes
                    .cmp(&left.media_bytes)
                    .then_with(|| left.chat.name.cmp(&right.chat.name))
            });
        }
        Sort::Name => {
            stats.sort_by(|left, right| {
                left.chat
                    .name
                    .to_lowercase()
                    .cmp(&right.chat.name.to_lowercase())
            });
        }
    }
    if reverse {
        stats.reverse();
    }
}

/// Everything the shared half of the command needs from a source it has opened.
struct Opened {
    store: Box<dyn Store>,
    kind: &'static str,
    path: String,
    app_version: String,
    default_out: PathBuf,
    ignored: Vec<PathBuf>,
    note: Option<String>,
}

pub fn run(args: &[String]) -> Result<i32> {
    let options = parse(args)?;
    let report = Report::new(options.json, options.quiet, options.diagnostics);
    let folder = options.folder.clone().unwrap_or_else(|| PathBuf::from("."));
    let source = pick_source(&folder)?;
    let workspace = Workspace::new()?;
    let opened = match source {
        Source::Android(inputs) => open_android(&inputs, &options, &report, &workspace)?,
        Source::Ios(inputs) => open_ios(&inputs, &options, &report, &workspace)?,
    };

    if !opened.ignored.is_empty() {
        report.info(format!(
            "ignored      {} file(s) from the older backup chain (crypt14)",
            opened.ignored.len()
        ));
    }
    let store = opened.store.as_ref();
    let stats = store.chat_stats()?;
    if stats.is_empty() {
        return Err(Error::input("this backup holds no chat"));
    }
    report.emit(Event::Found {
        source: opened.kind,
        path: opened.path.clone(),
        chats: stats.len(),
    });
    if report.diagnostics() {
        for line in store.diagnostics() {
            report.detail(line);
        }
    }

    let mut check_code = 0;
    if options.check {
        check_code = check_source(store, opened.note.as_deref(), &report)?;
        if options.chats.is_empty() && !options.all {
            return Ok(check_code);
        }
    }

    let clock = match &options.zone {
        Some(name) => Clock::named(name)?,
        None => Clock::system(),
    };
    let out_root = match &options.out {
        Some(path) => discover::expand(path),
        None => opened.default_out.clone(),
    };

    // The codes are this list's identity: assigned once, in the default order, and never moved.
    let total = stats.len();
    let numbers = numbered(&stats);
    let selected = if options.all {
        stats.clone()
    } else {
        resolve(&options, &stats)?
    };

    let mut display = stats;
    sort_stats(&mut display, options.sort, options.reverse);
    if let Some(limit) = options.limit {
        display.truncate(limit);
    }
    if selected.is_empty() {
        show_chats(&display, &clock, &numbers, total, &report);
        return Ok(check_code);
    }

    if options.dry_run {
        let messages: i64 = selected.iter().map(|stat| stat.messages).sum();
        let media: i64 = selected.iter().map(|stat| stat.media_count).sum();
        let bytes: i64 = selected.iter().map(|stat| stat.media_bytes).sum();
        report.info(format!(
            "would build  {} chat(s), {} message(s), {} media file(s), about {}; -c counts the files that are not in the copy",
            selected.len(),
            messages,
            media,
            human_bytes(bytes.max(0) as u64)
        ));
        report.emit(Event::Finished(Finished {
            archives: selected.len(),
            skipped: 0,
            messages: messages.max(0) as usize,
            media: media.max(0) as usize,
            missing: None,
            failures: 0,
            bytes: bytes.max(0) as u64,
            path: out_root.display().to_string(),
            seconds: report.elapsed(),
        }));
        return Ok(check_code);
    }

    let chosen = selected;
    std::fs::create_dir_all(&out_root)?;
    let jobs = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(2)
        .div_ceil(2)
        .clamp(1, 8);
    report.info(format!(
        "\nbuilding     {} chat(s) into {} ({jobs} workers)",
        chosen.len(),
        out_root.display()
    ));
    let mut summaries = Vec::new();
    let context = archive::BuildContext {
        out_root: &out_root,
        clock: &clock,
        app_version: &opened.app_version,
        jobs,
        report: &report,
    };
    for stat in chosen.iter() {
        let number = numbers.get(&stat.chat.jid).copied().unwrap_or(0);
        let label = format!("{number}. {}", stat.chat.name);
        let total_media = stat.media_count.max(0) as usize;
        report.emit(Event::Progress {
            label: label.clone(),
            done: 0,
            total: total_media,
        });
        let summary = archive::build_chat(store, stat, &context, &label)?;
        report.emit(Event::Progress {
            label,
            done: total_media,
            total: total_media,
        });
        if summary.empty {
            report.info(format!("  {}: skipped, no messages", summary.name));
            summaries.push(summary);
            continue;
        }
        report.info(format!(
            "  {}: {} messages, {} media ({} missing), {:.1} MB",
            summary
                .archive
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(""),
            summary.messages,
            summary.media,
            summary.missing,
            summary.bytes as f64 / 1e6
        ));
        for failure in &summary.failures {
            report.emit(Event::Failure(failure.clone()));
        }
        summaries.push(summary);
    }

    if !summaries.is_empty() {
        let index = archive::write_index(&out_root, &summaries)?;
        report.info(format!("\nindex        {}", index.display()));
        open_if_desktop(&index);
    }
    let failures: usize = summaries.iter().map(|summary| summary.failure_count).sum();
    let skipped = summaries.iter().filter(|summary| summary.empty).count();
    let suffix = if skipped > 0 {
        format!(", {skipped} empty chat(s) skipped")
    } else {
        String::new()
    };
    let bytes: u64 = summaries.iter().map(|summary| summary.bytes).sum();
    let messages: usize = summaries.iter().map(|summary| summary.messages).sum();
    let media: usize = summaries.iter().map(|summary| summary.media).sum();
    let missing: usize = summaries.iter().map(|summary| summary.missing).sum();
    report.info(format!(
        "\ndone: {} archive(s) in {}{suffix}",
        summaries.len() - skipped,
        out_root.display()
    ));
    report.emit(Event::Finished(Finished {
        archives: summaries.len() - skipped,
        skipped,
        messages,
        media,
        missing: Some(missing),
        failures,
        bytes,
        path: out_root.display().to_string(),
        seconds: report.elapsed(),
    }));
    if failures > 0 {
        return Ok(EXIT_INCOMPLETE);
    }
    Ok(check_code)
}

/// What `-c` asks of any source: a sound database, and every media file the rows name.
fn check_source(store: &dyn Store, note: Option<&str>, report: &Report) -> Result<i32> {
    store.integrity_check()?;
    let (present, referenced) = store.media_counts()?;
    report.info(format!(
        "check        database sound, {present} of {referenced} media file(s) present"
    ));
    let mut code = 0;
    if present < referenced {
        report.emit(Event::Warning(format!(
            "{} media file(s) named by the database are not in the copy",
            referenced - present
        )));
        code = EXIT_INCOMPLETE;
    }
    if let Some(note) = note {
        report.emit(Event::Warning(note.to_string()));
        code = EXIT_INCOMPLETE;
    }
    Ok(code)
}

fn open_android(
    inputs: &discover::Inputs,
    options: &Options,
    report: &Report,
    workspace: &Workspace,
) -> Result<Opened> {
    let mut database = std::fs::read(&inputs.database)?;
    let header = crypt15::parse_header(&database)?;
    require_openable(&header, &inputs.database)?;

    let key_spec = match options.key.clone().or_else(|| key_beside(inputs)) {
        Some(spec) => spec,
        None => ask_for_key()?,
    };
    let key = keys::load(&key_spec)?;
    let note = incremental_note(inputs);

    report.info(format!(
        "decrypting   {}",
        inputs
            .database
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("backup")
    ));
    let database_path = workspace.path.join("msgstore.db");
    crypt15::decrypt_to(
        &mut database,
        &header,
        &key,
        &mut std::fs::File::create(&database_path)?,
    )?;

    if !inputs.incrementals.is_empty() {
        report.info(format!(
            "merging      {} incremental file(s)",
            inputs.incrementals.len()
        ));
        merge::apply(&database_path, &inputs.incrementals, &key, &workspace.path)?;
    }

    let mut store = Msgstore::open(&database_path, &inputs.root, inputs.media.as_deref())?;
    if let Some(contacts) = &inputs.contacts {
        if contacts.extension().and_then(|value| value.to_str()) == Some("crypt15") {
            match decrypt_contacts(contacts, &key, workspace) {
                Ok(path) => store.apply_contacts(&path),
                Err(error) => report.emit(Event::Warning(format!(
                    "contacts not readable — names will be numbers ({error})"
                ))),
            }
        } else {
            store.apply_contacts(contacts);
        }
    }

    Ok(Opened {
        store: Box::new(store),
        kind: "android",
        path: inputs.root.display().to_string(),
        app_version: app_version(&header),
        default_out: inputs.root.join(OUTPUT_DIR),
        ignored: inputs.ignored.clone(),
        note,
    })
}

fn open_ios(
    inputs: &discover::IosInputs,
    options: &Options,
    report: &Report,
    workspace: &Workspace,
) -> Result<Opened> {
    if options.key.is_some() {
        return Err(Error::input(
            "-k is the Android key: an iPhone backup is read without one.\nDrop -k and run the same command.",
        ));
    }
    report.info(format!(
        "opening      {}",
        inputs
            .database
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("ChatStorage.sqlite")
    ));
    let store = IosStore::open(inputs, &workspace.path)?;
    Ok(Opened {
        store: Box::new(store),
        kind: "iphone",
        path: inputs.root.display().to_string(),
        app_version: "unknown".to_string(),
        default_out: inputs.root.join(OUTPUT_DIR),
        ignored: Vec::new(),
        note: None,
    })
}

/// An incremental older than the full backup: the delta was made before the base, so the database
/// it is merged into is newer than the delta.
fn incremental_note(inputs: &discover::Inputs) -> Option<String> {
    let full = std::fs::metadata(&inputs.database)
        .and_then(|meta| meta.modified())
        .ok()?;
    inputs.incrementals.iter().find_map(|path| {
        let stamp = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()?;
        (stamp < full).then(|| {
            format!(
                "{} is older than the full backup: make a fresh full backup and the delta again",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("an incremental")
            )
        })
    })
}

/// A folder that holds several device backups is not guessed: a terminal is asked, a pipe is told.
fn pick_source(folder: &Path) -> Result<Source> {
    match discover::detect(folder)? {
        Detected::One(source) => Ok(source),
        Detected::Several(folders) => {
            if !std::io::stdin().is_terminal() {
                return Err(discover::ambiguous(&folders));
            }
            eprintln!("more than one phone backup here:");
            for (index, path) in folders.iter().enumerate() {
                eprintln!("  {}. {}", index + 1, path.display());
            }
            eprint!("which one? ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            let chosen: usize = line
                .trim()
                .parse()
                .ok()
                .filter(|number| (1..=folders.len()).contains(number))
                .ok_or_else(|| Error::input("not one of the folders listed"))?;
            match discover::detect(&folders[chosen - 1])? {
                Detected::One(source) => Ok(source),
                Detected::Several(_) => Err(discover::ambiguous(&folders)),
            }
        }
    }
}

fn human_bytes(count: u64) -> String {
    let mut size = count as f64;
    for unit in ["B", "kB", "MB", "GB"] {
        if size < 1024.0 || unit == "GB" {
            return if unit == "B" {
                format!("{size:.0} {unit}")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1024.0;
    }
    format!("{size:.1} GB")
}

fn app_version(header: &crypt15::Header) -> String {
    if header.app_version.is_empty() {
        "unknown".to_string()
    } else {
        header.app_version.clone()
    }
}

fn require_openable(header: &crypt15::Header, path: &Path) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("backup");
    if !header.is_crypt15() {
        return Err(Error::wrong_type(format!(
            "{name} is a crypt14 backup: only end-to-end encrypted backups (crypt15) can be opened offline.\nTurn on end-to-end encrypted backup with the 64-digit key, run a full backup, copy again."
        )));
    }
    if header.key_type == "E2EE_ENCRYPTION_KEY" {
        return Ok(());
    }
    if header.key_type == "E2EE_PASSWORD" {
        return Err(Error::wrong_type(format!(
            "{name} was made with a password, not the 64-digit key.\nThe password only asks WhatsApp for the key, so this file cannot be opened offline.\nRe-enable end-to-end encrypted backup choosing the 64-digit key and make a new backup."
        )));
    }
    if header.key_type == "E2EE_PASSKEY" {
        return Err(Error::wrong_type(format!(
            "{name} was made with a passkey, not the 64-digit key.\nThere is no offline way to open it: re-enable end-to-end encrypted backup with the\n64-digit key and make a new backup."
        )));
    }
    Err(Error::wrong_type(format!(
        "{name} uses key type {}, which this tool cannot open.\nMake a fresh backup after enabling end-to-end encrypted backup with the 64-digit key.",
        if header.key_type.is_empty() {
            "unknown".to_string()
        } else {
            header.key_type.clone()
        }
    )))
}

fn key_beside(inputs: &discover::Inputs) -> Option<String> {
    let database_dir = inputs.database.parent()?.to_path_buf();
    let current = std::env::current_dir().ok();
    for name in [
        "key.png",
        "key.jpg",
        "key.jpeg",
        "key.txt",
        "encrypted_backup.key",
    ] {
        for folder in [&database_dir, &inputs.root] {
            let candidate = folder.join(name);
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
        if let Some(current) = &current {
            let candidate = current.join(name);
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
    }
    None
}

fn ask_for_key() -> Result<String> {
    if !std::io::stdin().is_terminal() {
        return Err(Error::key(
            "no key found and no terminal to ask on.\nPass -k with the 64 hex digits, a screenshot, or encrypted_backup.key.",
        ));
    }
    println!("\nkey: the 64 hex digits, or the path of the screenshot / encrypted_backup.key");
    print!("> ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim().to_string();
    if line.is_empty() {
        return Err(Error::key(
            "no key given. WhatsApp shows it once, when end-to-end encrypted backup is turned on.",
        ));
    }
    Ok(line)
}

/// The code of every chat in this list: 1..N in the default order, independent of any listing.
fn numbered(stats: &[ChatStat]) -> std::collections::HashMap<String, usize> {
    stats
        .iter()
        .enumerate()
        .map(|(index, stat)| (stat.chat.jid.clone(), index + 1))
        .collect()
}

/// The chats named by `-b`, looked up by code.
fn resolve(options: &Options, stats: &[ChatStat]) -> Result<Vec<ChatStat>> {
    let mut chosen = Vec::with_capacity(options.chats.len());
    for number in &options.chats {
        let stat = stats.get(number - 1).ok_or_else(|| {
            Error::input(format!(
                "chat {number} is not in this backup: -l shows the codes 1..{}",
                stats.len()
            ))
        })?;
        chosen.push(stat.clone());
    }
    Ok(chosen)
}

fn show_chats(
    stats: &[ChatStat],
    clock: &Clock,
    numbers: &std::collections::HashMap<String, usize>,
    total: usize,
    report: &Report,
) {
    let rows: Vec<ChatRow> = stats
        .iter()
        .map(|stat| ChatRow {
            code: numbers.get(&stat.chat.jid).copied().unwrap_or(0),
            name: label_of(stat).to_string(),
            messages: stat.messages,
            media: stat.media_count,
            bytes: stat.media_bytes,
            last: if stat.last_ts > 0 {
                clock.local(stat.last_ts)
            } else {
                String::new()
            },
        })
        .collect();
    report.emit(Event::ChatList(rows.clone()));
    if report.json() || report.quiet() {
        return;
    }
    let width = rows
        .iter()
        .map(|row| row.name.chars().count())
        .max()
        .unwrap_or(8);
    println!();
    println!(
        "{:>3}  {:<27} {:>7} {:>6} {:>9}  {:<width$}",
        "#", "last event", "msgs", "media", "size", "chat"
    );
    for row in &rows {
        println!(
            "{:>3}  {:<27} {:>7} {:>6} {:>9}  {:<width$}",
            row.code,
            if row.last.is_empty() {
                "—"
            } else {
                row.last.as_str()
            },
            row.messages,
            row.media,
            human_bytes(row.bytes.max(0) as u64),
            row.name
        );
    }
    let messages: i64 = rows.iter().map(|row| row.messages).sum();
    let bytes: i64 = rows.iter().map(|row| row.bytes).sum();
    println!(
        "\n{} chat(s), {} message(s), {} of media{}",
        rows.len(),
        messages,
        human_bytes(bytes.max(0) as u64),
        if rows.len() < total {
            format!(" (of {total} in this backup)")
        } else {
            String::new()
        }
    );
}

/// The name when the backup knows one, otherwise the full jid: it says who and what kind of chat.
fn label_of(stat: &ChatStat) -> &str {
    let user = stat.chat.jid.split('@').next().unwrap_or("");
    if stat.chat.name == user {
        &stat.chat.jid
    } else {
        &stat.chat.name
    }
}

fn decrypt_contacts(contacts: &Path, key: &[u8; 32], workspace: &Workspace) -> Result<PathBuf> {
    let mut bytes = std::fs::read(contacts)?;
    let header = crypt15::parse_header(&bytes)?;
    let target = workspace.path.join("wa.db");
    crypt15::decrypt_to(
        &mut bytes,
        &header,
        key,
        &mut std::fs::File::create(&target)?,
    )?;
    Ok(target)
}

fn open_if_desktop(page: &Path) {
    let has_display = std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
    if !has_display || !std::io::stdout().is_terminal() {
        return;
    }
    let opener = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // The browser is a convenience, not part of the run: its own output must not land in this
    // terminal, and it must not stay in the foreground group where a Ctrl-C would hit it.
    let mut command = std::process::Command::new(opener);
    command
        .arg(page)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let _ = command.spawn();
}

struct Workspace {
    path: PathBuf,
}

impl Workspace {
    fn new() -> Result<Self> {
        // Unique per run: tests and two invocations in the same process must never share a folder.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("wexport-{}-{stamp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        // The decrypted database lives here for the length of the build: 0700, so no other user on
        // the machine can read it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&path)?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&path)?;
        Ok(Workspace { path })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `-b 1,2` or `-b all`: only codes, because a code is what identifies a chat in this list.
fn set_selection(options: &mut Options, value: &str) -> Result<()> {
    for token in value.split(',').filter(|token| !token.is_empty()) {
        if token == "all" {
            options.all = true;
            continue;
        }
        match token.parse::<usize>() {
            Ok(number) if number > 0 => {
                if !options.chats.contains(&number) {
                    options.chats.push(number);
                }
            }
            _ => {
                return Err(Error::input(format!(
                    "-b takes chat codes or all, not {token}: -l shows the codes"
                )))
            }
        }
    }
    if options.all && !options.chats.is_empty() {
        return Err(Error::input(
            "-b all is the whole list: drop the codes, or drop all",
        ));
    }
    if !options.all && options.chats.is_empty() {
        return Err(Error::input(
            "-b takes at least one code, or all: -l shows the codes",
        ));
    }
    Ok(())
}

fn parse(args: &[String]) -> Result<Options> {
    let mut options = Options {
        folder: None,
        key: None,
        chats: Vec::new(),
        all: false,
        list: false,
        out: None,
        sort: Sort::Recent,
        reverse: false,
        limit: None,
        zone: None,
        json: false,
        quiet: false,
        diagnostics: false,
        dry_run: false,
        check: false,
    };
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].clone();
        if !argument.starts_with('-') || argument == "-" {
            set_folder(&mut options, &argument)?;
            index += 1;
            continue;
        }

        match argument.as_str() {
            "-h" => {
                print!("{}", help());
                std::process::exit(0);
            }
            "-v" => {
                println!("wexport {VERSION}");
                std::process::exit(0);
            }
            "-r" => options.reverse = true,
            "-j" => options.json = true,
            "-q" => options.quiet = true,
            "-d" => options.diagnostics = true,
            "-n" => options.dry_run = true,
            "-c" => options.check = true,
            "-l" => {
                options.list = true;
                // Up to two values, recognised by their shape, in either order.
                for _ in 0..2 {
                    let Some(next) = args.get(index + 1) else {
                        break;
                    };
                    if options.limit.is_none()
                        && !next.is_empty()
                        && next.chars().all(|c| c.is_ascii_digit())
                    {
                        options.limit = Some(count_of(next)?);
                        index += 1;
                        continue;
                    }
                    if let Ok(order) = Sort::parse(next) {
                        options.sort = order;
                        index += 1;
                        continue;
                    }
                    break;
                }
            }
            "-k" | "-b" | "-o" | "-t" => {
                let value = args.get(index + 1).cloned().ok_or_else(|| {
                    Error::input(format!("{argument} takes a value: -h shows them"))
                })?;
                index += 1;
                match argument.as_str() {
                    "-k" => options.key = Some(value),
                    "-b" => set_selection(&mut options, &value)?,
                    "-t" => options.zone = Some(value),
                    _ => options.out = Some(PathBuf::from(value)),
                }
            }
            other if other.starts_with('-') && other.len() > 2 => {
                return Err(Error::input(format!(
                    "{other}: one flag per word, like -l 10 size"
                )));
            }
            other => {
                return Err(Error::input(format!(
                    "{other} is not a flag: -h shows them"
                )));
            }
        }
        index += 1;
    }

    let building = options.all || !options.chats.is_empty();
    if options.list && building {
        return Err(Error::input(
            "-l only lists and exits: drop -b, or build without -l",
        ));
    }
    if options.list && options.check {
        return Err(Error::input(
            "-c checks the source and exits: drop -l, or check and build without it",
        ));
    }
    if options.dry_run && !building {
        return Err(Error::input(
            "-n changes a build, so it needs -b: -l lists, -n reports what -b would write",
        ));
    }
    if options.out.is_some() && !building {
        return Err(Error::input(
            "-o says where the archives go: give -b as well",
        ));
    }
    if options.reverse && building {
        return Err(Error::input(
            "-r orders the listing: -b builds and prints none, so there is nothing to reverse",
        ));
    }
    if options.quiet && options.diagnostics {
        return Err(Error::input(
            "-q silences the run and -d recounts it: use one or the other",
        ));
    }
    Ok(options)
}

/// A positive count; zero would list nothing and is a typo rather than an instruction.
fn count_of(value: &str) -> Result<usize> {
    match value.parse::<usize>() {
        Ok(count) if count > 0 => Ok(count),
        _ => Err(Error::input(format!(
            "-l needs a positive count, not {value}"
        ))),
    }
}

fn set_folder(options: &mut Options, argument: &str) -> Result<()> {
    if options.folder.is_some() {
        return Err(Error::input(format!(
            "one folder at a time, please: {argument}"
        )));
    }
    options.folder = Some(PathBuf::from(argument));
    Ok(())
}

fn help() -> &'static str {
    "usage: wexport [FOLDER] [flags]\n\nTurn a WhatsApp backup into self-contained archives that open by double-clicking\nindex.html: no server, no wexport and no network on the reading machine.\n\nAndroid: an end-to-end encrypted backup (crypt15) and its 64-digit key. A password\nor a passkey keeps the key on WhatsApp's servers, and the file on the phone cannot\nbe read offline. iPhone: a local backup made by Finder or iTunes with \"Encrypt\nlocal backup\" off, or the app's ChatStorage.sqlite; no key is involved.\n\nEvery time in the archive is written with its offset in the zone of this machine,\nunless -t names another one; the zone is recorded in the archive.\n\nflags:\n  -h          show this help and exit\n  -v          show the version and exit\n  -k KEY      the Android key: the 64 hex digits, a screenshot of them, or\n              encrypted_backup.key; without it the key is looked up beside the\n              backup (key.png, key.txt, encrypted_backup.key) and otherwise asked once\n  -l [N] [ORDER]\n              list and exit: N rows, ordered by recent (default), size or name,\n              in either order after the flag\n  -r          reverse the listing order\n  -b CODES    build these chats, by code from the list; comma-separated, or all\n  -o DIR      where the archives go (default: <input>/output), with -b\n  -t ZONE     render every time in this zone instead of the machine's, e.g.\n              Europe/Rome; each archive records the name it was rendered in\n  -j          JSON lines on stdout: found, chat_list, progress, diagnostic,\n              warning, failure, finished; errors stay on stderr\n  -q          no progress and no summary; a list or JSON still prints\n  -d          diagnostics: the files found, the columns recognised, every media\n              file placed or missing\n  -n          dry run: report what -b would build, write nothing; needs -b\n  -c          check the source and exit; with -b, check and then build\n\nThe codes belong to that list: 1..N in the default order, and they do not move\nwhen -l orders or cuts the list. A different backup numbers its chats anew.\nOne flag per word: -l 10 size, not -ls.\n\nexit codes:\n  0  success\n  1  unreadable input, or a question that could not be asked without a terminal\n  2  unusable key: not a key, or the key does not open this backup\n  3  wrong backup type: crypt14, password-based, passkey-based or encrypted iPhone\n  4  build incomplete: some media could not be processed, the archive is usable\n  5  missing external dependency: tesseract, for a key given as a screenshot\n\nrequirements: nothing to install; tesseract only for a key given as a screenshot.\n\nexamples:\n  wexport ~/Downloads/a\n  wexport ~/Downloads/a -l\n  wexport ~/Downloads/a -l 10 size\n  wexport ~/Downloads/a -l 5 recent -r\n  wexport ~/Downloads/a -b 3,42\n  wexport ~/Downloads/a -b all\n  wexport ~/Downloads/a -b all -t Europe/Rome\n  wexport ~/Downloads/a -b 1,2 -o /media/backup\n  wexport ~/Library/Application\\ Support/MobileSync/Backup/UDID -l\n  wexport ~/Library/Application\\ Support/MobileSync/Backup/UDID -b all -j\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msgstore::Chat;

    fn stat(name: &str, ts: i64, bytes: i64) -> ChatStat {
        ChatStat {
            chat: Chat {
                jid: format!("{name}@s.whatsapp.net"),
                name: name.to_string(),
                is_group: false,
            },
            messages: 1,
            first_ts: ts,
            last_ts: ts,
            media_count: 1,
            media_bytes: bytes,
        }
    }

    fn names(stats: &[ChatStat]) -> Vec<&str> {
        stats.iter().map(|stat| stat.chat.name.as_str()).collect()
    }

    #[test]
    fn the_list_can_be_ordered_by_size_name_or_recency() {
        let mut stats = vec![
            stat("beta", 300, 10),
            stat("alpha", 100, 900),
            stat("gamma", 200, 50),
        ];

        sort_stats(&mut stats, Sort::Size, false);
        assert_eq!(names(&stats), ["alpha", "gamma", "beta"]);

        sort_stats(&mut stats, Sort::Name, false);
        assert_eq!(names(&stats), ["alpha", "beta", "gamma"]);

        sort_stats(&mut stats, Sort::Recent, false);
        assert_eq!(names(&stats), ["beta", "gamma", "alpha"]);

        sort_stats(&mut stats, Sort::Size, true);
        assert_eq!(names(&stats), ["beta", "gamma", "alpha"], "reversed");
    }

    #[test]
    fn an_unknown_sort_is_refused_by_name() {
        let error = Sort::parse("biggest").unwrap_err();
        assert!(error.message.contains("recent, size or name"));
    }

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace()
            .map(|part| part.to_string())
            .collect()
    }

    #[test]
    fn the_list_reads_a_count_and_an_order_in_either_order() {
        let parsed = parse(&args("-l 10 size")).unwrap();
        assert_eq!(parsed.limit, Some(10));
        assert!(matches!(parsed.sort, Sort::Size));

        let parsed = parse(&args("-l size 10")).unwrap();
        assert_eq!(parsed.limit, Some(10));
        assert!(matches!(parsed.sort, Sort::Size));

        let parsed = parse(&args("-l name")).unwrap();
        assert_eq!(parsed.limit, None);
        assert!(matches!(parsed.sort, Sort::Name));

        let parsed = parse(&args("-l")).unwrap();
        assert!(parsed.list);
        assert_eq!(parsed.limit, None);
    }

    #[test]
    fn reverse_needs_nothing_else() {
        assert!(parse(&args("-r")).unwrap().reverse);
        assert!(parse(&args("-l 5 -r")).unwrap().reverse);
    }

    #[test]
    fn building_takes_codes_or_all() {
        let parsed = parse(&args("-b 3,42")).unwrap();
        assert_eq!(parsed.chats, [3, 42]);

        assert!(parse(&args("-b all")).unwrap().all);

        let parsed = parse(&args("-b 3,3,42,3")).unwrap();
        assert_eq!(parsed.chats, [3, 42], "a repeated code builds once");

        let error = parse(&args("-b family")).unwrap_err();
        assert!(error.message.contains("codes"));

        let error = parse(&args("-b all,3")).unwrap_err();
        assert!(error.message.contains("-b all"));

        let error = parse(&args("-b")).unwrap_err();
        assert!(error.message.contains("takes a value"));
    }

    #[test]
    fn reverse_orders_a_listing_and_a_build_has_none() {
        let error = parse(&args("-b 3 -r")).unwrap_err();
        assert!(error.message.contains("-r orders the listing"));
    }

    #[test]
    fn listing_and_building_do_not_mix() {
        let error = parse(&args("-l -b 2")).unwrap_err();
        assert!(error.message.contains("-l only lists"));
    }

    #[test]
    fn an_output_folder_needs_a_build() {
        let error = parse(&args("-o /tmp/x")).unwrap_err();
        assert!(error.message.contains("give -b"));

        assert!(parse(&args("-b all -o /tmp/x")).unwrap().out.is_some());
    }

    #[test]
    fn a_count_is_positive() {
        let error = parse(&args("-l 0")).unwrap_err();
        assert!(error.message.contains("positive count"));
    }

    #[test]
    fn one_flag_per_word() {
        let error = parse(&args("-ls size")).unwrap_err();
        assert!(error.message.contains("one flag per word"));
    }

    #[test]
    fn an_unknown_flag_names_itself() {
        let error = parse(&args("-x")).unwrap_err();
        assert!(error.message.contains("-x"));
        assert!(error.message.contains("-h"));
    }

    #[test]
    fn the_five_new_flags_are_read() {
        let parsed = parse(&args("-j -q -b all")).unwrap();
        assert!(parsed.json && parsed.quiet);
        let parsed = parse(&args("-d -b all")).unwrap();
        assert!(parsed.diagnostics);
        let parsed = parse(&args("-c")).unwrap();
        assert!(parsed.check);
        let parsed = parse(&args("-n -b all")).unwrap();
        assert!(parsed.dry_run);
    }

    #[test]
    fn a_dry_run_needs_a_build_and_a_check_does_not() {
        let error = parse(&args("-n")).unwrap_err();
        assert!(error.message.contains("needs -b"), "{}", error.message);
        assert!(parse(&args("-c")).unwrap().check);
        assert!(parse(&args("-c -n -b all")).unwrap().dry_run);
    }

    #[test]
    fn quiet_and_diagnostics_are_opposites() {
        let error = parse(&args("-q -d -b all")).unwrap_err();
        assert!(error.message.contains("one or the other"));
        assert!(parse(&args("-q -j -b all")).unwrap().json);
    }

    #[test]
    fn checking_and_listing_do_not_mix() {
        let error = parse(&args("-c -l")).unwrap_err();
        assert!(error.message.contains("checks the source"));
    }

    #[test]
    fn sizes_read_in_the_unit_that_fits() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 kB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn a_chat_without_a_name_shows_its_jid() {
        let mut named = stat("Alice", 1, 0);
        named.chat.jid = "393331234567@s.whatsapp.net".into();
        assert_eq!(label_of(&named), "Alice");

        // The adapter falls back to the number as a name, which is not a name at all.
        let anonymous = stat("393331234567", 1, 0);
        assert_eq!(label_of(&anonymous), "393331234567@s.whatsapp.net");
    }

    #[cfg(unix)]
    #[test]
    fn the_workspace_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let workspace = Workspace::new().unwrap();
        let mode = std::fs::metadata(&workspace.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "the decrypted backup is not for other users");
    }

    #[test]
    fn the_time_zone_is_a_value_the_run_passes_on() {
        let options = parse(&args("-t Europe/Rome")).unwrap();
        assert_eq!(options.zone.as_deref(), Some("Europe/Rome"));
        let error = parse(&args("-t")).unwrap_err();
        assert!(error.message.contains("takes a value"), "{}", error.message);
    }

    #[test]
    fn the_codes_start_at_one_in_list_order() {
        let stats = vec![stat("a", 3, 0), stat("b", 2, 0)];
        let numbers = numbered(&stats);
        assert_eq!(numbers["a@s.whatsapp.net"], 1);
        assert_eq!(numbers["b@s.whatsapp.net"], 2);
    }
}
