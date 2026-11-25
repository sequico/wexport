//! The build streams, so its peak allocation must not grow with the length of the conversation.
//!
//! A fixed ceiling would only say "this machine has RAM". Two builds, one twice the size of the
//! other, say what actually matters: doubling the messages must not double the peak. The counters
//! below are the only ones in the process, because this file is its own test binary.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};

use wexport::archive::{self, Clock};
use wexport::msgstore::Msgstore;

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() {
            let size = layout.size() as isize;
            let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout);
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = System.realloc(pointer, layout, new_size);
        if !moved.is_null() {
            let change = new_size as isize - layout.size() as isize;
            let live = LIVE.fetch_add(change, Ordering::Relaxed) + change;
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("wexport-memory-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn database(messages: i64, root: &Path) -> PathBuf {
    let path = root.join("msgstore.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(&format!(
            "CREATE TABLE message (_id INTEGER PRIMARY KEY, key_remote_jid TEXT,
                 timestamp INTEGER, from_me INTEGER, data TEXT);
             WITH RECURSIVE counter(n) AS (
                 SELECT 1 UNION ALL SELECT n + 1 FROM counter WHERE n < {messages}
             )
             INSERT INTO message
             SELECT n, '393331234567@s.whatsapp.net', 1700000000000 + n * 1000, n % 2,
                    'message number ' || n
             FROM counter;"
        ))
        .unwrap();
    path
}

/// Build one chat of `messages` and return the peak allocation seen during the build.
fn peak_for(messages: i64, name: &str) -> usize {
    let workspace = scratch(name);
    let path = database(messages, &workspace);
    let store = Msgstore::open(&path, &workspace, None).unwrap();
    let stat = store.chat_stats().unwrap().remove(0);
    let out = workspace.join("archives");
    std::fs::create_dir_all(&out).unwrap();
    let clock = Clock {
        tz: jiff::tz::TimeZone::UTC,
        name: "UTC".into(),
    };
    let report = wexport::report::Report::new(false, false, false);
    let context = archive::BuildContext {
        out_root: &out,
        clock: &clock,
        app_version: "2.26.34.7",
        jobs: 2,
        report: &report,
    };

    // Only the peak is reset: the live counter must stay true across the two builds.
    PEAK.store(0, Ordering::SeqCst);
    let summary = archive::build_chat(&store, &stat, &context, "memory").unwrap();
    assert_eq!(summary.messages as i64, messages);
    let peak = PEAK.load(Ordering::SeqCst).max(0) as usize;
    drop(store);
    let _ = std::fs::remove_dir_all(&workspace);
    peak
}

#[test]
fn the_peak_does_not_follow_the_length_of_the_chat() {
    let small = peak_for(50_000, "small");
    let large = peak_for(100_000, "large");
    println!("peak: {small} bytes at 50k messages, {large} bytes at 100k");

    assert!(
        large < 128 * 1024 * 1024,
        "a 100k-message chat peaked at {large} bytes"
    );
    // Holding the chat would double the peak with the message count; streaming keeps it flat.
    assert!(
        large < small * 3 / 2,
        "doubling the messages doubled the peak: {small} -> {large}"
    );
}
