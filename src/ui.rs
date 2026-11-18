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

const OUTPUT_DIR: &str = "archives";
const DEFAULT_RECENT: usize = 10;
const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Options {
    folder: Option<PathBuf>,
    key: Option<String>,
    chats: Vec<String>,
    list: bool,
    yes: bool,
    out: Option<PathBuf>,
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

    if options.list || (options.chats.is_empty() && !options.yes) {
        show_chats(&stats, &clock);
    } else {
        println!("\n{} chat(s) in the backup; --list shows them", stats.len());
    }
    if options.list {
        return Ok(0);
    }

    let chosen = choose(&stats, &options)?;
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
    for (position, stat) in chosen.iter().enumerate() {
        let label = format!("{}. {}", position + 1, stat.chat.name);
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
            "no key found and no terminal to ask on.\nPass --key with the 64 hex digits, a screenshot, or encrypted_backup.key.",
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

fn show_chats(stats: &[ChatStat], clock: &Clock) {
    let width = stats
        .iter()
        .map(|stat| stat.chat.name.chars().count())
        .max()
        .unwrap_or(8);
    println!();
    println!(
        "{:>3}  {:<27} {:>7} {:>6} {:>9}  {:<width$}  jid",
        "#", "last event", "msgs", "media", "size", "chat"
    );
    for (index, stat) in stats.iter().enumerate() {
        let stamp = if stat.last_ts > 0 {
            clock.local(stat.last_ts)
        } else {
            "—".to_string()
        };
        println!(
            "{:>3}  {:<27} {:>7} {:>6} {:>9}  {:<width$}  {}",
            index + 1,
            stamp,
            stat.messages,
            stat.media_count,
            human_bytes(stat.media_bytes.max(0) as u64),
            stat.chat.name,
            stat.chat.jid
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

fn choose<'a>(stats: &'a [ChatStat], options: &Options) -> Result<Vec<&'a ChatStat>> {
    if options.yes {
        return Ok(stats.iter().collect());
    }
    if !options.chats.is_empty() {
        let matched = match_rows(stats, &options.chats);
        if matched.is_empty() {
            // A typo in a script must not end with "0 archives" and exit 0.
            return Err(Error::input(format!(
                "no chat matches {}: --list shows the numbers and names",
                options.chats.join(", ")
            )));
        }
        return Ok(matched);
    }
    if !std::io::stdin().is_terminal() {
        return Err(Error::input(
            "no terminal to ask which chats to build.\nPass --chat <number|name> or --yes (or --list to only look).",
        ));
    }
    let recent: Vec<&ChatStat> = stats.iter().take(DEFAULT_RECENT).collect();
    let default = recent
        .iter()
        .map(|stat| format!("{}", row_of(stats, stat)))
        .collect::<Vec<_>>()
        .join(", ");
    loop {
        println!(
            "which? numbers or part of a name [the {} most recent: {}]",
            recent.len(),
            default
        );
        print!("> ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        let tokens: Vec<String> = line
            .split(|character: char| character == ',' || character.is_whitespace())
            .filter(|token| !token.is_empty())
            .map(|token| token.to_string())
            .collect();
        if tokens.is_empty() {
            return Ok(recent);
        }
        let matches = match_rows(stats, &tokens);
        if !matches.is_empty() {
            return Ok(matches);
        }
        println!("  nothing matched: try a number from the list, part of a name, or 'all'");
    }
}

fn row_of(stats: &[ChatStat], stat: &ChatStat) -> usize {
    stats
        .iter()
        .position(|candidate| candidate.chat.jid == stat.chat.jid)
        .unwrap_or(0)
        + 1
}

fn match_rows<'a>(stats: &'a [ChatStat], tokens: &[String]) -> Vec<&'a ChatStat> {
    let mut selected: Vec<usize> = Vec::new();
    for token in tokens {
        let lowered = token.to_lowercase();
        if matches!(lowered.as_str(), "all" | "tutte" | "*") {
            return stats.iter().collect();
        }
        if let Ok(index) = lowered.parse::<usize>() {
            if index >= 1 && index <= stats.len() && !selected.contains(&(index - 1)) {
                selected.push(index - 1);
            }
            continue;
        }
        let needle = fold(&lowered);
        for (index, stat) in stats.iter().enumerate() {
            if (fold(&stat.chat.name).contains(&needle) || fold(&stat.chat.jid).contains(&needle))
                && !selected.contains(&index)
            {
                selected.push(index);
            }
        }
    }
    selected.sort_unstable();
    selected.into_iter().map(|index| &stats[index]).collect()
}

fn fold(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .flat_map(|character| character.to_lowercase())
        .collect()
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

fn parse(args: &[String]) -> Result<Options> {
    let mut options = Options {
        folder: None,
        key: None,
        chats: Vec::new(),
        list: false,
        yes: false,
        out: None,
    };
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "-h" | "--help" => {
                print!("{}", help());
                std::process::exit(0);
            }
            "--version" => {
                println!("wexport {VERSION}");
                std::process::exit(0);
            }
            "--list" => options.list = true,
            "--yes" => options.yes = true,
            "--key" | "--chat" | "--out" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| Error::input(format!("{argument} needs a value")))?;
                match argument.as_str() {
                    "--key" => options.key = Some(value.clone()),
                    "--chat" => options.chats.push(value.clone()),
                    _ => options.out = Some(PathBuf::from(value)),
                }
                index += 1;
            }
            other if other.starts_with("--key=") => {
                options.key = Some(other["--key=".len()..].to_string())
            }
            other if other.starts_with("--chat=") => {
                options.chats.push(other["--chat=".len()..].to_string())
            }
            other if other.starts_with("--out=") => {
                options.out = Some(PathBuf::from(&other["--out=".len()..]))
            }
            other if other.starts_with('-') => {
                return Err(Error::input(format!(
                    "unknown option {other}: see wexport --help"
                )))
            }
            other => {
                if matches!(other, "check" | "build" | "serve" | "list") {
                    return Err(Error::input(format!(
                        "'{other}' is not a command any more: point wexport at the folder instead.\n  wexport <folder>              (asks which chats)\n  wexport <folder> --list       (only lists)\n  wexport <folder> --chat 2     (builds one)"
                    )));
                }
                if options.folder.is_some() {
                    return Err(Error::input(format!(
                        "one folder at a time, please: {other}"
                    )));
                }
                options.folder = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }
    Ok(options)
}

fn help() -> String {
    format!(
        "usage: wexport [FOLDER] [options]\n\nTurn an Android WhatsApp crypt15 backup into self-contained archives that open by\ndouble-clicking index.html: no server, no wexport and no network on the reading\nmachine. Point it at the folder you copied; it does the rest.\n\noptions:\n  -h, --help            show this help and exit\n  --version             show the version and exit\n  --key HEX|IMAGE|FILE  the 64 hex digits, a screenshot, or encrypted_backup.key;\n                        without it the key is looked up beside the backup (key.png,\n                        key.txt, encrypted_backup.key) and otherwise asked once\n  --chat N|NAME          build these chats without asking: row numbers or part of a\n                        name, comma-separated and repeatable; 'all' for every chat\n  --list                only print the numbered chat list and exit\n  --yes                 build every chat without asking\n  --out DIR             where the archives go (default: <input>/{OUTPUT_DIR})\n\nexit codes:\n  0  success\n  1  unreadable input, or a question that could not be asked without a terminal\n  2  unusable key: not a key, or the key does not open this backup\n  3  wrong backup type: crypt14, password-based or passkey-based backup\n  4  build incomplete: some media could not be processed, the archive is usable\n  5  missing external dependency: ffmpeg, ffprobe, or tesseract for a screenshot key\n\nrequirements: ffmpeg and ffprobe on PATH; tesseract only for a screenshot key.\n\nexamples:\n  wexport ~/Downloads/a\n  wexport ~/Downloads/a --list\n  wexport ~/Downloads/a --chat 2\n  wexport ~/Downloads/a --chat family --chat 3 --yes\n"
    )
}
