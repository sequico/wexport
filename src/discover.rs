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
        let inputs = discover(&root).unwrap();
        assert_eq!(inputs.database, nested.join("msgstore.db.crypt15"));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod more_tests {
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

    #[test]
    fn the_named_full_backup_wins_over_a_newer_rolling_copy() {
        let root = root("rolling");
        let databases = root.join("Databases");
        write(&databases.join("msgstore-2026-01-01.1.db.crypt15"));
        write(&databases.join("msgstore.db.crypt15"));

        let inputs = discover(&root).unwrap();
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

        let inputs = discover(&root).unwrap();
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

        let inputs = discover(&databases).unwrap();
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
        assert!(error.message.contains("no Databases/ folder"));
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
}
