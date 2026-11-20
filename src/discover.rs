//! Finding the backup inside the folder the user copied, wherever the copy put it.
//!
//! Two phones, two layouts: Android keeps `Databases/msgstore*.db.crypt15` inside a copied app
//! folder; an iPhone keeps a hashed-file backup with a `Manifest.db`, or an extracted app group with
//! a plain `ChatStorage.sqlite`.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The app group WhatsApp keeps its database and media in.
pub const IOS_GROUP: &str = "group.net.whatsapp.WhatsApp.shared";

/// The domain a Finder backup lists those files under.
pub const IOS_DOMAIN: &str = "AppDomainGroup-group.net.whatsapp.WhatsApp.shared";

#[derive(Debug, Clone)]
pub struct Inputs {
    pub root: PathBuf,
    pub database: PathBuf,
    pub incrementals: Vec<PathBuf>,
    pub contacts: Option<PathBuf>,
    pub media: Option<PathBuf>,
    pub ignored: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct IosInputs {
    /// The folder the user pointed at.
    pub root: PathBuf,
    /// `ChatStorage.sqlite` as it lies in the backup or the extracted container.
    pub database: PathBuf,
    pub wal: Option<PathBuf>,
    pub shm: Option<PathBuf>,
    /// `Manifest.db`, when this is a Finder/iTunes backup.
    pub manifest: Option<PathBuf>,
    /// The folder holding the hashed files, beside `Manifest.db`.
    pub files: Option<PathBuf>,
    /// The extracted app group, when the backup was already taken apart.
    pub app_group: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Source {
    Android(Inputs),
    Ios(IosInputs),
}

/// What the folder turned out to hold: one source, or several phone backups to choose between.
#[derive(Debug)]
pub enum Detected {
    One(Source),
    Several(Vec<PathBuf>),
}

pub fn discover(path: &Path) -> Result<Source> {
    match detect(path)? {
        Detected::One(source) => Ok(source),
        Detected::Several(folders) => Err(ambiguous(&folders)),
    }
}

pub fn detect(path: &Path) -> Result<Detected> {
    let expanded = expand(path);
    let root = if expanded.is_file() {
        expanded.parent().unwrap_or(&expanded).to_path_buf()
    } else {
        expanded
    };
    if !root.is_dir() {
        return Err(Error::input(format!("not a folder: {}", root.display())));
    }

    let ios = ios_in(&root)?;
    let android = find_databases(&root);
    match (ios, android) {
        (Some(_), Some(databases)) => {
            return Err(Error::input(format!(
                "{} holds both an Android copy and an iPhone backup.\nPoint me at one of them:\n  {}\n  {}",
                root.display(),
                databases.display(),
                root.display()
            )))
        }
        (Some(ios), None) => return Ok(Detected::One(Source::Ios(ios))),
        (None, Some(_)) => return Ok(Detected::One(Source::Android(android_in(&root)?))),
        (None, None) => {}
    }

    let backups = backup_folders(&root);
    match backups.len() {
        0 => Err(not_a_backup(&root)),
        1 => {
            let ios = ios_backup(&backups[0])?;
            Ok(Detected::One(Source::Ios(ios)))
        }
        _ => Ok(Detected::Several(backups)),
    }
}

pub fn ambiguous(folders: &[PathBuf]) -> Error {
    let mut message =
        String::from("this folder holds more than one phone backup. Point me at one of these:\n");
    for folder in folders {
        message.push_str(&format!("  {}\n", folder.display()));
    }
    Error::input(message.trim_end())
}

fn not_a_backup(root: &Path) -> Error {
    Error::input(format!(
        "no backup under {}\nPoint me at the folder you copied from the phone.\nAndroid: the folder holding Databases/msgstore.db.crypt15.\niPhone: the device folder of a local backup, or its ChatStorage.sqlite.",
        root.display()
    ))
}

fn android_in(root: &Path) -> Result<Inputs> {
    let databases = find_databases(root).ok_or_else(|| not_a_backup(root))?;

    let crypt15 = files_matching(&databases, ".crypt15");
    let crypt14 = files_matching(&databases, ".crypt14");
    let crypt12 = files_matching(&databases, ".crypt12");
    if crypt15.is_empty() {
        if !crypt14.is_empty() {
            return Err(Error::wrong_type(format!(
                "only crypt14 backups in {} — end-to-end encryption is off, or the backup was never redone after enabling it.\nTurn on end-to-end encrypted backup with the 64-digit key, run a full backup, copy the folder again.",
                databases.display()
            )));
        }
        if !crypt12.is_empty() {
            return Err(Error::wrong_type(format!(
                "only crypt12 backups in {} — that chain needs a password and no end-to-end key.\nTurn on end-to-end encrypted backup with the 64-digit key, run a full backup, copy the folder again.",
                databases.display()
            )));
        }
        return Err(Error::input(format!(
            "no msgstore.db.crypt15 in {}",
            databases.display()
        )));
    }

    let mut full = crypt15
        .iter()
        .find(|path| name_is(path, "msgstore.db.crypt15"))
        .cloned();
    if full.is_none() {
        full = crypt15.iter().max_by_key(|path| modified(path)).cloned();
    }
    let full = full.expect("crypt15 is not empty");
    if name_starts_with(&full, "msgstore-increment") {
        return Err(Error::input(format!(
            "{} has only incremental files and no full backup.\nOpen WhatsApp → Settings → Chats → Chat Backup → Back up now, then copy the folder again.",
            databases.display()
        )));
    }

    let mut incrementals: Vec<PathBuf> = crypt15
        .iter()
        .filter(|path| **path != full && name_starts_with(path, "msgstore-increment"))
        .cloned()
        .collect();
    // Oldest first, by the numbers in the name: "increment-2" comes before "increment-10", and a
    // dated name ("2026-12-03") sorts by its date.
    incrementals.sort_by_key(|path| natural_order(path));
    let ignored = crypt14
        .iter()
        .filter(|path| name_starts_with(path, "msgstore"))
        .cloned()
        .collect();

    let mut contacts = None;
    let mut media = None;
    for candidate_root in [root, databases.parent().unwrap_or(root)] {
        if contacts.is_none() {
            if let Some(backups) = first_dir(
                candidate_root,
                &["Backups", "WhatsApp/Backups", "WhatsApp Business/Backups"],
            ) {
                for name in ["wa.db.crypt15", "wa.db"] {
                    let candidate = backups.join(name);
                    if candidate.is_file() {
                        contacts = Some(candidate);
                        break;
                    }
                }
            }
        }
        if media.is_none() {
            media = first_dir(
                candidate_root,
                &["Media", "WhatsApp/Media", "WhatsApp Business/Media"],
            );
        }
    }

    Ok(Inputs {
        root: root.to_path_buf(),
        database: full,
        incrementals,
        contacts,
        media,
        ignored,
    })
}

/// `~/x` as the user's home plus `x`: a quoted tilde reaches us unexpanded, and a folder literally
/// named `~` in the current directory is never what was meant.
pub fn expand(path: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok();
        if let Some(home) = home {
            return PathBuf::from(home).join(stripped);
        }
    }
    path.to_path_buf()
}

/// A Finder-style backup, a bare database, or an extracted app group, in that order.
fn ios_in(root: &Path) -> Result<Option<IosInputs>> {
    if root.join("Manifest.db").is_file() {
        return Ok(Some(ios_backup(root)?));
    }
    let candidates = [
        root.join("ChatStorage.sqlite"),
        root.join(IOS_GROUP).join("ChatStorage.sqlite"),
        root.join(IOS_DOMAIN).join("ChatStorage.sqlite"),
    ];
    for database in candidates {
        if database.is_file() {
            let app_group = database.parent().unwrap_or(root).to_path_buf();
            return Ok(Some(IosInputs {
                root: root.to_path_buf(),
                database,
                wal: None,
                shm: None,
                manifest: None,
                files: None,
                app_group: Some(app_group),
            }));
        }
    }
    // One level further down: a folder holding the extracted group, not the group itself. The
    // candidates are sorted, so two matching folders always pick the same one.
    let mut children = child_dirs(root);
    children.sort();
    for folder in children {
        for name in [IOS_GROUP, IOS_DOMAIN] {
            let database = folder.join(name).join("ChatStorage.sqlite");
            if database.is_file() {
                let app_group = database.parent().map(Path::to_path_buf);
                return Ok(Some(IosInputs {
                    root: root.to_path_buf(),
                    database,
                    wal: None,
                    shm: None,
                    manifest: None,
                    files: None,
                    app_group,
                }));
            }
        }
    }
    Ok(None)
}

/// A backup folder: refuse it when it is encrypted, then find WhatsApp's database through the
/// manifest.
fn ios_backup(folder: &Path) -> Result<IosInputs> {
    if manifest_is_encrypted(folder) {
        return Err(Error::wrong_type(format!(
            "{} is an iPhone backup encrypted with a password, which this tool cannot open.\nTurn off \"Encrypt local backup\" in Finder, make the backup again, and copy the folder."
            ,
            folder.display()
        )));
    }
    let manifest = folder.join("Manifest.db");
    let (database, wal, shm) = ios_files(folder, &manifest).map_err(|error| {
        Error::input(format!(
            "cannot read {}: {error}\nThe backup may be encrypted, or WhatsApp may not be part of it.",
            manifest.display()
        ))
    })?;
    Ok(IosInputs {
        root: folder.to_path_buf(),
        database,
        wal,
        shm,
        manifest: Some(manifest),
        files: Some(folder.to_path_buf()),
        app_group: None,
    })
}

/// The database, its log and its shared-memory file, as the manifest names them.
fn ios_files(
    folder: &Path,
    manifest: &Path,
) -> std::result::Result<(PathBuf, Option<PathBuf>, Option<PathBuf>), rusqlite::Error> {
    let connection = rusqlite::Connection::open_with_flags(
        manifest,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let path_of = |name: &str| -> Option<PathBuf> {
        manifest_file(&connection, folder, IOS_DOMAIN, name)
            .or_else(|| manifest_file(&connection, folder, IOS_GROUP, name))
            .or_else(|| {
                let file_id: String = connection
                    .query_row(
                        "SELECT fileID FROM Files WHERE relativePath = ?1 AND domain LIKE '%whatsapp%' LIMIT 1",
                        rusqlite::params![name],
                        |row| row.get(0),
                    )
                    .ok()?;
                let path = hashed_file(folder, &file_id)?;
                path.is_file().then_some(path)
            })
    };
    let database = path_of("ChatStorage.sqlite").ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    Ok((
        database,
        path_of("ChatStorage.sqlite-wal"),
        path_of("ChatStorage.sqlite-shm"),
    ))
}

/// The file a backup keeps for `fileID`, or None when the manifest hands over something that is not
/// a file ID: the manifest is untrusted input, and `../../etc/passwd` is not a name a backup uses.
fn hashed_file(folder: &Path, file_id: &str) -> Option<PathBuf> {
    if file_id.len() != 40 || !file_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let prefix = &file_id[..2];
    Some(folder.join(prefix).join(file_id))
}

/// The file a backup keeps for `domain`/`relative`, when the manifest lists it and it is there.
pub fn manifest_file(
    manifest: &rusqlite::Connection,
    files: &Path,
    domain: &str,
    relative: &str,
) -> Option<PathBuf> {
    let file_id: String = manifest
        .query_row(
            "SELECT fileID FROM Files WHERE domain = ?1 AND relativePath = ?2 LIMIT 1",
            rusqlite::params![domain, relative],
            |row| row.get(0),
        )
        .ok()?;
    let path = hashed_file(files, &file_id)?;
    path.is_file().then_some(path)
}

/// The same lookup by the file's name alone, for a path the database and the manifest spell
/// differently.
pub fn manifest_file_like(
    manifest: &rusqlite::Connection,
    files: &Path,
    domain: &str,
    name: &str,
) -> Option<PathBuf> {
    let file_id: String = manifest
        .query_row(
            "SELECT fileID FROM Files WHERE domain = ?1 AND relativePath LIKE ?2 LIMIT 1",
            rusqlite::params![domain, format!("%{name}")],
            |row| row.get(0),
        )
        .ok()?;
    let path = hashed_file(files, &file_id)?;
    path.is_file().then_some(path)
}

/// `Manifest.plist` is readable even when everything beside it is not, and it says whether the
/// files are encrypted.
fn manifest_is_encrypted(folder: &Path) -> bool {
    let Ok(plist) = plist::Value::from_file(folder.join("Manifest.plist")) else {
        return false;
    };
    plist
        .as_dictionary()
        .and_then(|dictionary| dictionary.get("IsEncrypted"))
        .and_then(plist::Value::as_boolean)
        .unwrap_or(false)
}

/// The device folders a folder holds, for a path like `MobileSync/Backup`.
fn backup_folders(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![root.to_path_buf()];
    // `MobileSync/Backup/<device>` puts the device two levels under the folder people point at.
    for _ in 0..2 {
        let mut next = Vec::new();
        for folder in &level {
            for child in child_dirs(folder) {
                if child.join("Manifest.db").is_file() || child.join("Manifest.plist").is_file() {
                    found.push(child);
                } else {
                    next.push(child);
                }
            }
        }
        level = next;
    }
    found.sort();
    found
}

fn child_dirs(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut children: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .filter(|path| !skipped(path))
        .collect();
    children.sort();
    children
}

fn first_dir(root: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|relative| root.join(relative))
        .find(|path| path.is_dir())
}

fn files_matching(folder: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.starts_with("msgstore") && name.ends_with(suffix))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn find_databases(root: &Path) -> Option<PathBuf> {
    let looks_like = |path: &Path| {
        std::fs::read_dir(path)
            .map(|entries| {
                entries.filter_map(|entry| entry.ok()).any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .map(|name| name.starts_with("msgstore") && name.contains(".crypt1"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    };
    if root
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("databases"))
        .unwrap_or(false)
        && looks_like(root)
    {
        return Some(root.to_path_buf());
    }
    if let Some(explicit) = first_dir(
        root,
        &[
            "Databases",
            "WhatsApp/Databases",
            "WhatsApp Business/Databases",
        ],
    ) {
        // A folder named Databases is a hint, not proof: another app's copy can carry the same
        // name, and the backup may sit one level further down.
        if looks_like(&explicit) {
            return Some(explicit);
        }
    }

    // The walk skips the trees that cannot hold a backup and takes the shallowest folder that does,
    // in name order, so two copies of the same tree always pick the same one.
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(folder) = queue.pop_front() {
        if folder.as_path() != root && looks_like(&folder) {
            return Some(folder);
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            // file_type does not follow a link: a symlink loop cannot trap the walk.
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .map(|entry| entry.path())
            .filter(|path| !skipped(path))
            .collect();
        children.sort();
        queue.extend(children);
    }
    None
}

/// Trees a copied phone folder can carry that never hold a backup.
fn skipped(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            // The media tree holds tens of thousands of files and nothing to open.
            name == "node_modules" || name == ".git" || name == "Media"
        })
        .unwrap_or(false)
}

fn name_is(path: &Path, wanted: &str) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some(wanted)
}

fn name_starts_with(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with(prefix))
        .unwrap_or(false)
}

/// The numbers in a file name, in order: the natural sort of `increment-2` before `increment-10`
/// and of `2026-12-03` before `2026-12-10`.
fn natural_order(path: &Path) -> Vec<u64> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.split(|character: char| !character.is_ascii_digit())
                .filter(|part| !part.is_empty())
                .filter_map(|part| part.parse::<u64>().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn modified(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// Build the file set of a Finder-style backup for the tests: a manifest and its hashed files.
#[cfg(test)]
pub fn write_backup(root: &Path, files: &[(&str, &[u8])]) -> std::path::PathBuf {
    use sha1::{Digest, Sha1};
    std::fs::create_dir_all(root).unwrap();
    let connection = rusqlite::Connection::open(root.join("Manifest.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE Files (fileID TEXT PRIMARY KEY, domain TEXT, relativePath TEXT, flags INTEGER, file BLOB);",
        )
        .unwrap();
    for (relative, bytes) in files {
        let mut digest = Sha1::new();
        digest.update(format!("{IOS_DOMAIN}-{relative}").as_bytes());
        let file_id = format!("{:x}", digest.finalize());
        let prefix: String = file_id.chars().take(2).collect();
        std::fs::create_dir_all(root.join(&prefix)).unwrap();
        std::fs::write(root.join(&prefix).join(&file_id), bytes).unwrap();
        connection
            .execute(
                "INSERT INTO Files (fileID, domain, relativePath, flags, file) VALUES (?1, ?2, ?3, 1, NULL)",
                rusqlite::params![file_id, IOS_DOMAIN, relative],
            )
            .unwrap();
    }
    root.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("wexport-discover-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"backup").unwrap();
    }

    fn android(source: Source) -> Inputs {
        match source {
            Source::Android(inputs) => inputs,
            Source::Ios(_) => panic!("expected an Android backup"),
        }
    }

    #[test]
    fn hex_looking_names_are_not_mistaken_for_a_folder() {
        assert!(!name_starts_with(
            Path::new("/tmp/msgstore.db.crypt15"),
            "msgstore-increment"
        ));
    }

    #[test]
    fn a_quoted_tilde_expands_to_the_home_folder() {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .expect("a home folder is set in the environment running the tests");
        assert_eq!(
            expand(Path::new("~/archives")),
            Path::new(&home).join("archives")
        );
        assert_eq!(expand(Path::new("/tmp/plain")), Path::new("/tmp/plain"));
    }

    #[test]
    fn increments_are_ordered_by_their_numbers_not_by_their_spelling() {
        let mut names = vec![
            Path::new("msgstore-increment-10.db.crypt15"),
            Path::new("msgstore-increment-2.db.crypt15"),
            Path::new("msgstore-increment-1.db.crypt15"),
        ];
        names.sort_by_key(|path| natural_order(path));
        assert_eq!(
            names,
            [
                Path::new("msgstore-increment-1.db.crypt15"),
                Path::new("msgstore-increment-2.db.crypt15"),
                Path::new("msgstore-increment-10.db.crypt15"),
            ]
        );

        let mut dates = [
            Path::new("msgstore-increment-2026-12-10.db.crypt15"),
            Path::new("msgstore-increment-2026-12-03.db.crypt15"),
        ];
        dates.sort_by_key(|path| natural_order(path));
        assert_eq!(
            dates[0],
            Path::new("msgstore-increment-2026-12-03.db.crypt15")
        );
    }

    #[test]
    fn a_crypt12_folder_says_what_it_is() {
        // The folder name carries neither chain: the message quotes the path, and a name with
        // "crypt12" in it would make the assertion below true for any message at all.
        let root = std::env::temp_dir().join(format!("wexport-oldchain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let databases = root.join("Databases");
        std::fs::create_dir_all(&databases).unwrap();
        std::fs::write(databases.join("msgstore.db.crypt12"), b"old chain").unwrap();
        let error = discover(&root).unwrap_err();
        assert_eq!(error.code, crate::error::EXIT_TYPE);
        assert!(
            error.message.starts_with("only crypt12 backups"),
            "{}",
            error.message
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_empty_databases_folder_does_not_hide_the_backup() {
        let root = std::env::temp_dir().join(format!("wexport-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Databases")).unwrap();
        let nested = root.join("WhatsApp/Databases");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("msgstore.db.crypt15"), b"a backup lives here").unwrap();
        let inputs = android(discover(&root).unwrap());
        assert_eq!(inputs.database, nested.join("msgstore.db.crypt15"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_named_full_backup_wins_over_a_newer_rolling_copy() {
        let root = root("rolling");
        let databases = root.join("Databases");
        write(&databases.join("msgstore-2026-01-01.1.db.crypt15"));
        write(&databases.join("msgstore.db.crypt15"));

        let inputs = android(discover(&root).unwrap());
        assert_eq!(inputs.database.file_name().unwrap(), "msgstore.db.crypt15");
        assert!(
            inputs.incrementals.is_empty(),
            "a rolling copy is not a delta"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn only_increment_names_are_replayed() {
        let root = root("deltas");
        let databases = root.join("Databases");
        write(&databases.join("msgstore.db.crypt15"));
        write(&databases.join("msgstore-increment-2026-12-03.db.crypt15"));
        write(&databases.join("msgstore-2026-12-04.1.db.crypt15"));

        let inputs = android(discover(&root).unwrap());
        assert_eq!(inputs.incrementals.len(), 1);
        assert!(inputs.incrementals[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("msgstore-increment"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_named_databases_is_found_by_its_contents() {
        let parent = root("named");
        let databases = parent.join("Databases");
        write(&databases.join("msgstore.db.crypt15"));

        let inputs = android(discover(&databases).unwrap());
        assert_eq!(inputs.database, databases.join("msgstore.db.crypt15"));
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn a_path_that_is_not_a_folder_says_so() {
        let error = discover(Path::new("/wexport-no-such-folder")).unwrap_err();
        assert!(error.message.contains("not a folder"));
    }

    #[test]
    fn a_folder_of_unrelated_files_is_not_a_backup_folder() {
        let root = root("unrelated");
        write(&root.join("Databases/notes.txt"));
        let error = discover(&root).unwrap_err();
        assert!(error.message.contains("no backup under"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_called_databases_without_a_backup_inside_is_not_one() {
        let parent = root("named-empty");
        let databases = parent.join("Databases");
        write(&databases.join("notes.txt"));
        assert_eq!(
            find_databases(&databases),
            None,
            "the name is a hint, not proof"
        );
        assert_eq!(find_databases(&parent), None);
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn a_file_named_after_a_backup_but_with_another_suffix_is_not_a_backup() {
        let root = root("decoy-name");
        write(&root.join("copy/msgstore.db.backup"));
        write(&root.join("copy/chat.crypt15"));
        assert_eq!(find_databases(&root), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_walk_skips_the_trees_that_never_hold_a_backup() {
        for skipped in ["node_modules", ".git", "Media"] {
            let root = root(&format!("skip-{skipped}"));
            write(&root.join(skipped).join("msgstore.db.crypt15"));
            assert_eq!(find_databases(&root), None, "{skipped} must not be walked");
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn a_backup_a_few_folders_down_is_found() {
        let root = root("deep");
        write(&root.join("com.whatsapp/WhatsApp/Databases/msgstore.db.crypt15"));
        assert_eq!(
            find_databases(&root),
            Some(root.join("com.whatsapp/WhatsApp/Databases"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_copies_of_a_backup_always_pick_the_shallower_one() {
        let root = root("copies");
        write(&root.join("b/c/Databases/msgstore.db.crypt15"));
        write(&root.join("a/Databases/msgstore.db.crypt15"));
        assert_eq!(find_databases(&root), Some(root.join("a/Databases")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_link_to_a_backup_folder_is_not_followed() {
        let outside = root("linked-target");
        write(&outside.join("Databases/msgstore.db.crypt15"));
        let root = root("linked");
        std::os::unix::fs::symlink(&outside, root.join("elsewhere")).unwrap();
        assert_eq!(find_databases(&root), None);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn a_manifest_that_answers_with_a_path_instead_of_a_file_id_finds_nothing() {
        let root = root("hostile-manifest");
        let connection = rusqlite::Connection::open(root.join("Manifest.db")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT, flags INTEGER, file BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO Files VALUES ('../../etc/passwd', ?1, 'x.jpg', 1, NULL)",
                rusqlite::params![IOS_DOMAIN],
            )
            .unwrap();
        assert!(manifest_file(&connection, &root, IOS_DOMAIN, "x.jpg").is_none());
        assert!(manifest_file_like(&connection, &root, IOS_DOMAIN, "x.jpg").is_none());
        assert!(hashed_file(&root, "../../etc/passwd").is_none());
        drop(connection);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_holding_both_phones_says_so_instead_of_picking_one() {
        let root = root("both");
        write_backup(&root, &[("ChatStorage.sqlite", b"a")]);
        write(&root.join("Databases/msgstore.db.crypt15"));
        let error = discover(&root).unwrap_err();
        assert!(
            error
                .message
                .contains("both an Android copy and an iPhone backup"),
            "{}",
            error.message
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_finder_backup_is_found_through_its_manifest() {
        let root = root("ios-backup");
        write_backup(
            &root,
            &[
                ("ChatStorage.sqlite", b"the database"),
                ("Message/Media/x.jpg", b"a picture"),
            ],
        );
        let Source::Ios(inputs) = discover(&root).unwrap() else {
            panic!("expected an iPhone backup");
        };
        assert!(inputs.database.is_file());
        assert_eq!(inputs.manifest, Some(root.join("Manifest.db")));
        assert_eq!(inputs.files.as_deref(), Some(root.as_path()));
        assert!(inputs.wal.is_none() && inputs.shm.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_backup_with_its_log_is_found_whole() {
        let root = root("ios-wal");
        write_backup(
            &root,
            &[
                ("ChatStorage.sqlite", b"the database"),
                ("ChatStorage.sqlite-wal", b"the log"),
                ("ChatStorage.sqlite-shm", b"the index"),
            ],
        );
        let Source::Ios(inputs) = discover(&root).unwrap() else {
            panic!("expected an iPhone backup");
        };
        assert!(inputs.wal.is_some(), "the WAL is part of the database");
        assert!(inputs.shm.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_encrypted_backup_is_refused_by_naming_it() {
        let root = root("ios-encrypted");
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert("IsEncrypted".into(), plist::Value::Boolean(true));
        plist::Value::Dictionary(dictionary)
            .to_file_binary(root.join("Manifest.plist"))
            .unwrap();
        std::fs::write(root.join("Manifest.db"), b"encrypted").unwrap();
        let error = discover(&root).unwrap_err();
        assert_eq!(error.code, crate::error::EXIT_TYPE);
        assert!(error.message.contains("encrypted with a password"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_device_folders_are_listed_not_chosen() {
        let root = root("ios-two");
        write_backup(&root.join("aaa"), &[("ChatStorage.sqlite", b"a")]);
        write_backup(&root.join("bbb"), &[("ChatStorage.sqlite", b"b")]);
        match detect(&root).unwrap() {
            Detected::Several(folders) => {
                assert_eq!(folders.len(), 2);
                assert!(folders[0].ends_with("aaa"));
            }
            Detected::One(_) => panic!("two backups are not one"),
        }
        let error = discover(&root).unwrap_err();
        assert!(error.message.contains("more than one phone backup"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn one_device_folder_inside_a_backup_root_is_found() {
        let root = root("ios-nested");
        write_backup(&root.join("udid"), &[("ChatStorage.sqlite", b"a")]);
        let Source::Ios(inputs) = discover(&root).unwrap() else {
            panic!("expected an iPhone backup");
        };
        assert!(inputs.database.ends_with(
            inputs
                .database
                .file_name()
                .expect("a database file")
                .to_str()
                .unwrap()
        ));
        assert_eq!(inputs.manifest, Some(root.join("udid/Manifest.db")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_extracted_app_group_opens_without_a_manifest() {
        let root = root("ios-container");
        let group = root.join(IOS_GROUP);
        std::fs::create_dir_all(group.join("Message/Media")).unwrap();
        std::fs::write(group.join("ChatStorage.sqlite"), b"a database").unwrap();
        let Source::Ios(inputs) = discover(&root).unwrap() else {
            panic!("expected an iPhone backup");
        };
        assert_eq!(inputs.app_group.as_deref(), Some(group.as_path()));
        assert!(inputs.manifest.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_bare_database_opens_and_has_no_media_root() {
        let root = root("ios-bare");
        std::fs::write(root.join("ChatStorage.sqlite"), b"a database").unwrap();
        let Source::Ios(inputs) = discover(&root).unwrap() else {
            panic!("expected an iPhone backup");
        };
        assert!(inputs.app_group.is_some());
        assert!(inputs.manifest.is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
