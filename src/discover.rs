//! Finding the backup inside the folder the user copied, wherever the copy put it.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Inputs {
    pub root: PathBuf,
    pub database: PathBuf,
    pub incrementals: Vec<PathBuf>,
    pub contacts: Option<PathBuf>,
    pub media: Option<PathBuf>,
    pub ignored: Vec<PathBuf>,
}

/// Abort with the install command when an external binary is missing (plan §2).
pub fn require_binaries(names: &[&str]) -> Result<()> {
    for name in names {
        let found = std::process::Command::new(name)
            .arg("-version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !found {
            return Err(crate::error::missing_binary(name));
        }
    }
    Ok(())
}

pub fn discover(path: &Path) -> Result<Inputs> {
    let expanded = expand(path);
    let root = if expanded.is_file() {
        expanded.parent().unwrap_or(&expanded).to_path_buf()
    } else {
        expanded
    };
    if !root.is_dir() {
        return Err(Error::input(format!("not a folder: {}", root.display())));
    }

    let databases = find_databases(&root).ok_or_else(|| {
        Error::input(format!(
            "no Databases/ folder with a msgstore backup under {}\nPoint me at the folder you copied from the phone: it must contain Databases/msgstore.db.crypt15.\nCopy Android/media/com.whatsapp/WhatsApp (or WhatsApp Business) and try again.",
            root.display()
        ))
    })?;

    let crypt15 = files_matching(&databases, ".crypt15");
    let crypt14 = files_matching(&databases, ".crypt14");
    if crypt15.is_empty() {
        if !crypt14.is_empty() {
            return Err(Error::wrong_type(format!(
                "only crypt14 backups in {} — end-to-end encryption is off, or the backup was never redone after enabling it.\nTurn on end-to-end encrypted backup with the 64-digit key, run a full backup, copy the folder again.",
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

    let incrementals = crypt15
        .iter()
        .filter(|path| **path != full && name_starts_with(path, "msgstore-increment"))
        .cloned()
        .collect();
    let ignored = crypt14
        .iter()
        .filter(|path| name_starts_with(path, "msgstore"))
        .cloned()
        .collect();

    let mut contacts = None;
    let mut media = None;
    for candidate_root in [&root, databases.parent().unwrap_or(&root)] {
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
        root,
        database: full,
        incrementals,
        contacts,
        media,
        ignored,
    })
}

fn expand(path: &Path) -> PathBuf {
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

    // shortcut: the walk stops six folders down — a copy nested deeper than that is not found, and
    // the user is told to point at the folder that holds Databases/.
    let mut queue = vec![(root.to_path_buf(), 0usize)];
    while let Some((folder, depth)) = queue.pop() {
        if depth > 0 && looks_like(&folder) {
            return Some(folder);
        }
        if depth >= 6 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let skip = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name == "node_modules" || name == ".git")
                .unwrap_or(false);
            if path.is_dir() && !skip {
                queue.push((path, depth + 1));
            }
        }
    }
    None
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

fn modified(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_looking_names_are_not_mistaken_for_a_folder() {
        assert!(!name_starts_with(
            Path::new("/tmp/msgstore.db.crypt15"),
            "msgstore-increment"
        ));
    }

    #[test]
    fn a_missing_binary_is_a_dependency_failure() {
        let error = require_binaries(&["wexport-no-such-binary"]).unwrap_err();
        assert_eq!(error.code, crate::error::EXIT_DEPENDENCY);
        assert!(error.message.contains("sudo apt install ffmpeg"));
    }

    #[test]
    fn an_empty_databases_folder_does_not_hide_the_backup() {
        let root = std::env::temp_dir().join(format!("wexport-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Databases")).unwrap();
        let nested = root.join("WhatsApp/Databases");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("msgstore.db.crypt15"), b"a backup lives here").unwrap();
        let inputs = discover(&root).unwrap();
        assert_eq!(inputs.database, nested.join("msgstore.db.crypt15"));
        let _ = std::fs::remove_dir_all(root);
    }
}
