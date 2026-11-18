//! End to end on the committed fixture.
//!
//! These are the acceptance checks: the archive a build produces must have the folder names, the
//! record shape and the frozen timestamps the plan describes, and a rebuild must be byte-identical.
//!
//! The names and the chunk bytes were originally compared against a second implementation of the same
//! product; that implementation is gone, and the expectation now lives here as the golden values.

use std::path::{Path, PathBuf};

use wexport::ui;

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic/phone")
}

fn build(out: &Path) {
    let args = vec![
        fixture().display().to_string(),
        "--key".into(),
        KEY.into(),
        "--yes".into(),
        "--out".into(),
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
        "--key".into(),
        KEY.into(),
        "--list".into(),
    ];
    assert_eq!(ui::run(&args).expect("list runs"), 0);
    buffer.clear();
}
