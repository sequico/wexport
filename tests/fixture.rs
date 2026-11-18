//! End to end on the committed fixture.
//!
//! These are the acceptance checks: the archive a build produces must have the folder names, the
//! record shape and the frozen timestamps the plan describes, and a rebuild must be byte-identical.
//! The golden chunk is the record format written down, so a change to it shows up as a diff.

use std::path::{Path, PathBuf};

use wexport::{archive, crypt15, keys, msgstore::Msgstore, ui};

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic/phone")
}

/// The fixture's database, decrypted and named from `wa.db`, for the tests below the command line.
fn decrypted() -> (Msgstore, PathBuf) {
    let path = std::env::temp_dir().join(format!("wexport-it-{}-store.db", unique()));
    decrypt_file(&fixture().join("Databases/msgstore.db.crypt15"), &path);
    let mut store = Msgstore::open(&path).unwrap();
    let contacts = fixture().join("Backups/wa.db.crypt15");
    if contacts.is_file() {
        let contacts_path = path.with_file_name(format!("wexport-it-{}-contacts.db", unique()));
        decrypt_file(&contacts, &contacts_path);
        store.apply_contacts(&contacts_path);
        let _ = std::fs::remove_file(contacts_path);
    }
    (store, path)
}

/// cargo test runs tests as threads in one process, so the pid is not unique enough here.
fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::process::id() as u64 * 1_000_000 + NEXT.fetch_add(1, Ordering::Relaxed)
}

fn decrypt_file(source: &Path, dest: &Path) {
    let mut bytes = std::fs::read(source).unwrap();
    let header = crypt15::parse_header(&bytes).unwrap();
    let key = keys::load(KEY).unwrap();
    crypt15::decrypt_to(
        &mut bytes,
        &header,
        &key,
        &mut std::fs::File::create(dest).unwrap(),
    )
    .unwrap();
}

/// A fixed zone, so the golden chunk does not depend on the machine running the test.
fn rome() -> archive::Clock {
    archive::Clock {
        tz: jiff::tz::TimeZone::get("Europe/Rome").expect("a zone name jiff knows"),
        name: "Europe/Rome".into(),
    }
}

fn build(out: &Path) {
    let args = vec![
        fixture().display().to_string(),
        "-k".into(),
        KEY.into(),
        "-b".into(),
        "all".into(),
        "-o".into(),
        out.display().to_string(),
    ];
    assert_eq!(ui::run(&args).expect("build runs"), 0);
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("wexport-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn archives(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    out.sort();
    out
}

#[test]
fn two_archives_with_the_expected_names() {
    let out = scratch("names");
    build(&out);
    let names: Vec<String> = archives(&out)
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, ["2026-12-02-alice-8c02", "2026-12-02-family-9fd6"]);
    assert!(out.join("index.html").is_file());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn each_archive_is_self_contained() {
    let out = scratch("self");
    build(&out);
    for archive in archives(&out) {
        for name in [
            "index.html",
            "app.js",
            "app.css",
            "README.txt",
            "data/manifest.js",
        ] {
            assert!(
                archive.join(name).is_file(),
                "{name} missing in {}",
                archive.display()
            );
        }
        let readme = std::fs::read_to_string(archive.join("README.txt")).unwrap();
        assert!(readme.contains("No server is needed"));
    }
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_manifest_carries_the_counts_and_the_zone() {
    let out = scratch("manifest");
    build(&out);
    let family = out.join("2026-12-02-family-9fd6/data/manifest.js");
    let text = std::fs::read_to_string(family).unwrap();
    assert!(text.contains("\"messages\":9"));
    assert!(text.contains("\"people\":2"));
    assert!(text.contains("\"chunk"));
    assert!(text.contains("\"source_app_version\":\"2.26.34.7\""));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn timestamps_are_frozen_strings_with_offsets() {
    let out = scratch("stamps");
    build(&out);
    let chunk =
        std::fs::read_to_string(out.join("2026-12-02-family-9fd6/data/2026-12.js")).unwrap();
    assert!(
        chunk.contains("\"s\":\"2026-12-01 10:00:00 +01:00\"")
            || chunk.contains("\"s\":\"2026-12-01 10:00:00 +02:00\"")
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn message_text_cannot_escape_the_script() {
    let out = scratch("escape");
    build(&out);
    let chunk = std::fs::read_to_string(out.join("2026-12-02-alice-8c02/data/2026-11.js")).unwrap();
    assert!(chunk.contains("\\u003c/script"));
    assert!(!chunk.contains("</script>"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_second_build_is_byte_identical() {
    let out = scratch("again");
    build(&out);
    let family = out.join("2026-12-02-family-9fd6/data/2026-12.js");
    let first = std::fs::read(&family).unwrap();
    build(&out);
    assert_eq!(std::fs::read(&family).unwrap(), first);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_rebuild_removes_media_it_no_longer_references() {
    let out = scratch("prune");
    build(&out);
    let orphan = out.join("2026-12-02-family-9fd6/media/zz/orphan.png");
    std::fs::create_dir_all(orphan.parent().unwrap()).unwrap();
    std::fs::write(&orphan, b"gone on the phone").unwrap();
    build(&out);
    assert!(!orphan.exists());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_rebuild_removes_derivatives_it_no_longer_references() {
    let out = scratch("deriv");
    build(&out);
    // Thumbnails and posters sit directly in deriv/, not one folder deeper.
    let orphan = out.join("2026-12-02-family-9fd6/deriv/orphan.webp");
    std::fs::write(&orphan, b"gone on the phone").unwrap();
    build(&out);
    assert!(!orphan.exists());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_rebuild_removes_chunks_of_months_that_are_gone() {
    let out = scratch("chunks");
    build(&out);
    let stale = out.join("2026-12-02-family-9fd6/data/2025-01.js");
    std::fs::write(&stale, "WEX.chunk(\"2025-01\",[]);\n").unwrap();
    build(&out);
    assert!(!stale.exists());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_empty_chat_is_skipped() {
    let out = scratch("empty");
    build(&out);
    let names: Vec<String> = archives(&out)
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(!names.iter().any(|name| name.contains("empty")));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_listing_finds_every_chat() {
    let mut buffer: Vec<u8> = Vec::new();
    let args = vec![
        fixture().display().to_string(),
        "-k".into(),
        KEY.into(),
        "-l".into(),
    ];
    assert_eq!(ui::run(&args).expect("list runs"), 0);
    buffer.clear();
}

#[test]
fn a_media_file_that_was_not_copied_is_counted_as_missing() {
    let (store, database) = decrypted();
    let stats = store.chat_stats().unwrap();
    let alice = stats.iter().find(|stat| stat.chat.name == "Alice").unwrap();
    let out = scratch("missing");
    let root = fixture();
    let media = root.join("Media");
    let clock = rome();
    let context = archive::BuildContext {
        out_root: &out,
        clock: &clock,
        app_version: "2.26.34.7",
        copy_root: &root,
        media_root: Some(&media),
        jobs: 2,
    };
    let summary = archive::build_chat(&store, alice, &context, "Alice").unwrap();
    assert_eq!(summary.media, 1);
    // IMG-deleted-from-phone.png is referenced by the database and absent from the folder.
    assert_eq!(summary.missing, 1);
    let _ = std::fs::remove_dir_all(out);
    let _ = std::fs::remove_file(database);
}

#[test]
fn the_selection_takes_codes_and_skips_the_empty_chat() {
    let out = scratch("selection");
    let args = vec![
        fixture().display().to_string(),
        "-k".into(),
        KEY.into(),
        "-b".into(),
        "2,3".into(),
        "-o".into(),
        out.display().to_string(),
    ];
    assert_eq!(ui::run(&args).expect("selection runs"), 0);
    let names: Vec<String> = archives(&out)
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    // Code 2 is Family, code 3 is the empty chat: one archive, and not the one code 1 would give.
    assert_eq!(names, ["2026-12-02-family-9fd6"]);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_code_out_of_range_is_an_error() {
    let out = scratch("nosuch");
    let args = vec![
        fixture().display().to_string(),
        "-k".into(),
        KEY.into(),
        "-b".into(),
        "99".into(),
        "-o".into(),
        out.display().to_string(),
    ];
    let error = ui::run(&args).expect_err("a code past the end stops the run");
    assert_eq!(error.code, wexport::error::EXIT_INPUT);
    assert!(error.message.contains("not in this backup"));
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn the_listing_puts_the_newest_conversation_first() {
    let (store, database) = decrypted();
    let stats = store.chat_stats().unwrap();
    let names: Vec<&str> = stats.iter().map(|stat| stat.chat.name.as_str()).collect();
    assert_eq!(names, ["Alice", "Family", "Empty chat"]);
    let _ = std::fs::remove_file(database);
}

#[test]
fn the_family_chunk_is_the_committed_golden_file() {
    let (store, database) = decrypted();
    let stats = store.chat_stats().unwrap();
    let family = stats
        .iter()
        .find(|stat| stat.chat.name == "Family")
        .unwrap();
    let out = scratch("golden");
    let root = fixture();
    let media = root.join("Media");
    let clock = rome();
    let context = archive::BuildContext {
        out_root: &out,
        clock: &clock,
        app_version: "2.26.34.7",
        copy_root: &root,
        media_root: Some(&media),
        jobs: 2,
    };
    let built = archive::build_chat(&store, family, &context, "Family").unwrap();
    let chunk = built.archive.join("data/2026-12.js");
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden/family-2026-12.js");
    if let Ok(expected) = std::fs::read(&golden) {
        assert_eq!(
            String::from_utf8_lossy(&std::fs::read(&chunk).unwrap()),
            String::from_utf8_lossy(&expected),
            "the record format changed: review the diff and regenerate {}",
            golden.display()
        );
    } else {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::copy(&chunk, &golden).unwrap();
        panic!(
            "golden written to {}: review it and commit it",
            golden.display()
        );
    }
    let _ = std::fs::remove_dir_all(out);
    let _ = std::fs::remove_file(database);
}

#[test]
fn a_rebuild_after_new_messages_leaves_one_folder() {
    let (store, path) = decrypted();
    let mut stat = store.chat_stats().unwrap().remove(0);
    let root = scratch("rebuild");
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let clock = rome();
    let context = archive::BuildContext {
        out_root: &out,
        clock: &clock,
        app_version: "2.26.34.7",
        copy_root: &fixture(),
        media_root: None,
        jobs: 2,
    };
    let first = archive::build_chat(&store, &stat, &context, "first").unwrap();
    stat.last_ts += 86_400_000;
    let second = archive::build_chat(&store, &stat, &context, "later").unwrap();

    assert_ne!(
        first.archive, second.archive,
        "the new date names a new folder"
    );
    assert_eq!(
        std::fs::read_dir(&out).unwrap().count(),
        1,
        "the older copy of the same chat is gone"
    );
    assert!(second.archive.is_dir());
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_chat_longer_than_a_batch_is_counted_from_end_to_end() {
    // 300 messages is more than one media batch (256) and one month: the summary, the manifest and
    // the chunk must all agree with the source.
    let root = scratch("batch");
    let path = root.join("msgstore.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE message (_id INTEGER PRIMARY KEY, key_remote_jid TEXT, timestamp INTEGER,
                 from_me INTEGER, data TEXT);
             WITH RECURSIVE counter(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM counter WHERE n < 300)
             INSERT INTO message
             SELECT n, '393331234567@s.whatsapp.net', 1765000000000 + n * 60000, n % 2, 'message ' || n
             FROM counter;",
        )
        .unwrap();
    drop(connection);

    let store = Msgstore::open(&path).unwrap();
    let stat = store.chat_stats().unwrap().remove(0);
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let clock = rome();
    let context = archive::BuildContext {
        out_root: &out,
        clock: &clock,
        app_version: "2.26.34.7",
        copy_root: &root,
        media_root: None,
        jobs: 2,
    };
    let summary = archive::build_chat(&store, &stat, &context, "batch").unwrap();

    assert_eq!(summary.messages, 300);
    assert_eq!(summary.first_ts, 1765000000000 + 60_000);
    assert_eq!(summary.last_ts, 1765000000000 + 300 * 60_000);

    let manifest = std::fs::read_to_string(summary.archive.join("data/manifest.js")).unwrap();
    assert!(manifest.contains("\"messages\":300"), "{manifest}");
    let chunks = std::fs::read_dir(summary.archive.join("data"))
        .unwrap()
        .count();
    assert!(chunks >= 2, "the manifest and at least one chunk");

    let whole = std::fs::read_dir(&out).unwrap().count();
    assert_eq!(whole, 1, "one folder for one chat");
    let _ = std::fs::remove_dir_all(root);
}
