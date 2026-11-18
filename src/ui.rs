//! The command itself: parse the few flags, discover, decrypt, list, ask, build, report.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::archive::{self, Clock};
use crate::crypt15;
use crate::discover;
use crate::error::{Error, Result, EXIT_INCOMPLETE};
use crate::keys;
use crate::merge;
use crate::msgstore::{ChatStat, Msgstore};

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
    sort: Sort,
    reverse: bool,
    limit: Option<usize>,
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

pub fn run(args: &[String]) -> Result<i32> {
    let options = parse(args)?;
    let folder = options.folder.clone().unwrap_or_else(|| PathBuf::from("."));

    discover::require_binaries(&["ffmpeg", "ffprobe"])?;
    let inputs = discover::discover(&folder)?;
    let mut database = std::fs::read(&inputs.database)?;
    let header = crypt15::parse_header(&database)?;
    require_openable(&header, &inputs.database)?;
    if !inputs.ignored.is_empty() {
        println!(
            "ignored      {} file(s) from the older backup chain (crypt14)",
            inputs.ignored.len()
        );
    }

    let key_spec = match options.key.clone().or_else(|| key_beside(&inputs)) {
        Some(spec) => spec,
        None => ask_for_key()?,
    };
    let key = keys::load(&key_spec)?;

    let clock = Clock::system();
    let out_root = match &options.out {
        Some(path) => discover::expand(path),
        None => inputs.root.join(OUTPUT_DIR),
    };

    let workspace = Workspace::new()?;
    println!(
        "decrypting   {}",
        inputs
            .database
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("backup")
    );
    let database_path = workspace.path.join("msgstore.db");
    crypt15::decrypt_to(
        &mut database,
        &header,
        &key,
        &mut std::fs::File::create(&database_path)?,
    )?;

    if !inputs.incrementals.is_empty() {
        println!(
            "merging      {} incremental file(s)",
            inputs.incrementals.len()
        );
        merge::apply(&database_path, &inputs.incrementals, &key, &workspace.path)?;
    }

    let mut store = Msgstore::open(&database_path)?;
    if let Some(contacts) = &inputs.contacts {
        if contacts.extension().and_then(|value| value.to_str()) == Some("crypt15") {
            match decrypt_contacts(contacts, &key, &workspace) {
                Ok(path) => store.apply_contacts(&path),
                Err(error) => {
                    eprintln!("warning: contacts not readable — names will be numbers ({error})")
                }
            }
        } else {
            store.apply_contacts(contacts);
        }
    }

    let stats = store.chat_stats()?;
    if stats.is_empty() {
        return Err(Error::input("this backup holds no chat"));
    }
    // The codes are this list's identity: assigned once, in the default order, and never moved.
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
    show_chats(&display, &clock, &numbers);

    if selected.is_empty() {
        return Ok(0);
    }
    let chosen = selected;
    std::fs::create_dir_all(&out_root)?;
    let jobs = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(2)
        .div_ceil(2)
        .clamp(1, 8);
    println!(
        "\nbuilding     {} chat(s) into {} ({jobs} workers)",
        chosen.len(),
        out_root.display()
    );
    let mut summaries = Vec::new();
    let source_version = app_version(&header);
    let context = archive::BuildContext {
        out_root: &out_root,
        clock: &clock,
        app_version: &source_version,
        copy_root: &inputs.root,
        media_root: inputs.media.as_deref(),
        jobs,
    };
    for stat in chosen.iter() {
        let number = numbers.get(&stat.chat.jid).copied().unwrap_or(0);
        let label = format!("{number}. {}", stat.chat.name);
        let summary = archive::build_chat(&store, stat, &context, &label)?;
        if summary.empty {
            println!("  {}: skipped, no messages", summary.name);
            summaries.push(summary);
            continue;
        }
        println!(
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
        );
        for failure in summary.failures.iter().take(5) {
            println!("    {failure}");
        }
        summaries.push(summary);
    }

    if !summaries.is_empty() {
        let index = archive::write_index(&out_root, &summaries)?;
        println!("\nindex        {}", index.display());
        open_if_desktop(&index);
    }
    let failures: usize = summaries.iter().map(|summary| summary.failure_count).sum();
    let skipped = summaries.iter().filter(|summary| summary.empty).count();
    let suffix = if skipped > 0 {
        format!(", {skipped} empty chat(s) skipped")
    } else {
        String::new()
    };
    println!(
        "\ndone: {} archive(s) in {}{suffix}",
        summaries.len() - skipped,
        out_root.display()
    );
    if failures > 0 {
        eprintln!("{failures} file(s) could not be processed; the archives are usable");
        return Ok(EXIT_INCOMPLETE);
    }
    Ok(0)
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
        if header.key_type.is_empty() { "unknown".to_string() } else { header.key_type.clone() }
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
) {
    let labels: Vec<&str> = stats.iter().map(label_of).collect();
    let width = labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(8);
    println!();
    println!(
        "{:>3}  {:<27} {:>7} {:>6} {:>9}  {:<width$}",
        "#", "last event", "msgs", "media", "size", "chat"
    );
    for (stat, label) in stats.iter().zip(&labels) {
        let stamp = if stat.last_ts > 0 {
            clock.local(stat.last_ts)
        } else {
            "—".to_string()
        };
        println!(
            "{:>3}  {:<27} {:>7} {:>6} {:>9}  {:<width$}",
            numbers.get(&stat.chat.jid).copied().unwrap_or(0),
            stamp,
            stat.messages,
            stat.media_count,
            human_bytes(stat.media_bytes.max(0) as u64),
            label
        );
    }
    let messages: i64 = stats.iter().map(|stat| stat.messages).sum();
    let bytes: i64 = stats.iter().map(|stat| stat.media_bytes).sum();
    println!(
        "\n{} chat(s), {} message(s), {} of media",
        stats.len(),
        messages,
        human_bytes(bytes.max(0) as u64)
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
    let _ = std::process::Command::new(opener).arg(page).spawn();
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
            Ok(number) if number > 0 => options.chats.push(number),
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
            "-k" | "-b" | "-o" => {
                let value = args.get(index + 1).cloned().ok_or_else(|| {
                    Error::input(format!("{argument} takes a value: -h shows them"))
                })?;
                index += 1;
                match argument.as_str() {
                    "-k" => options.key = Some(value),
                    "-b" => set_selection(&mut options, &value)?,
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

    if options.list && (options.all || !options.chats.is_empty()) {
        return Err(Error::input(
            "-l only lists and exits: drop -b, or build without -l",
        ));
    }
    if options.out.is_some() && !options.all && options.chats.is_empty() {
        return Err(Error::input(
            "-o says where the archives go: give -b as well",
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
    "usage: wexport [FOLDER] [flags]\n\nTurn an Android WhatsApp crypt15 backup into self-contained archives that open by\ndouble-clicking index.html: no server, no wexport and no network on the reading\nmachine. Point it at the folder you copied; without -b it lists the chats.\n\nflags:\n  -h          show this help and exit\n  -v          show the version and exit\n  -k KEY      the 64 hex digits, a screenshot of them, or encrypted_backup.key;\n              without it the key is looked up beside the backup (key.png, key.txt,\n              encrypted_backup.key) and otherwise asked once\n  -l [N] [ORDER]\n              list and exit: N rows, ordered by recent (default), size or name,\n              in either order after the flag\n  -r          reverse the listing order\n  -b CODES    build these chats, by code from the list; comma-separated, or all\n  -o DIR      where the archives go (default: <input>/output), with -b\n\nThe codes belong to that list: 1..N in the default order, and they do not move\nwhen -l orders or cuts the list. A different backup numbers its chats anew.\nOne flag per word: -l 10 size, not -ls.\n\nexit codes:\n  0  success\n  1  unreadable input, or a question that could not be asked without a terminal\n  2  unusable key: not a key, or the key does not open this backup\n  3  wrong backup type: crypt14, password-based or passkey-based backup\n  4  build incomplete: some media could not be processed, the archive is usable\n  5  missing external dependency: ffmpeg, ffprobe, or tesseract for a screenshot key\n\nrequirements: ffmpeg and ffprobe on PATH; tesseract only for a screenshot key.\n\nexamples:\n  wexport ~/Downloads/a\n  wexport ~/Downloads/a -l\n  wexport ~/Downloads/a -l 10 size\n  wexport ~/Downloads/a -l 5 recent -r\n  wexport ~/Downloads/a -b 3,42\n  wexport ~/Downloads/a -b all\n  wexport ~/Downloads/a -b 1,2 -o /media/backup\n"
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

        let error = parse(&args("-b family")).unwrap_err();
        assert!(error.message.contains("codes"));

        let error = parse(&args("-b all,3")).unwrap_err();
        assert!(error.message.contains("-b all"));

        let error = parse(&args("-b")).unwrap_err();
        assert!(error.message.contains("takes a value"));
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
}
