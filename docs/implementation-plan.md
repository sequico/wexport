# Implementation plan

Status: v0.1.0 implemented. This document remains the **specification** — what the product must be;
`README.md` says what is done and what is still open. Android only, `crypt15` backups, 64-digit key,
media folder copied by the user.

---

## 1. Scope

**In scope**

| | |
|---|---|
| Platform | Android (WhatsApp and WhatsApp Business) |
| Backup format | `msgstore.db.crypt15` + `msgstore-increment-*.db.crypt15`, plus the rolling `msgstore-YYYY-MM-DD.1.db.crypt15` copies when present |
| Key | 32-byte root key: 64 hex digits typed in, a `.png` screenshot, or an `encrypted_backup.key` file (root only) |
| Input | A folder the user copied from the phone over USB/MTP (`Databases/`, `Backups/`, `Media/`) |
| Output | one **self-contained archive per chat**, `<out>/<date>-<slug>/` — HTML + classic-script data chunks + media, opened by **double-clicking `index.html`**, with no server and no wexport installed |

**Out of scope for v1** (each one is a separate product decision, not a later ticket)

- iPhone / `ChatStorage.sqlite`.
- Google Drive download (`mcrypt1` objects, `oauth_token`).
- Backups made with a password or a passkey: the key is held by WhatsApp's HSM, not recoverable
  offline. The tool recognises them and says so; it cannot open them.
- Legacy `crypt12` / `crypt14` (key file lives in app-private storage, needs root).
- Re-encrypting, writing back to the phone, or merging two accounts.
- A GUI, a mobile app, or a Docker image.

---

## 2. Pipeline

Five stages, each a module with a narrow contract so it can be tested alone.

```
input dir + key
      │
      ▼
[1] preflight   discover files, read the crypt15 header, decide whether this is openable at all
      │         → HeaderInfo{format, app_version, key_type, files[], total_bytes}
      ▼
[2] decrypt     crypt15 → msgstore.db (+ incrementals → msgstore-merged.db)
      │         → one SQLite file, schema untouched
      ▼
[3] read        source schema (WhatsApp 2.26) → slim schema (ours)
      │         → chat.db: messages, chats, contacts, media refs, reactions, receipts
      ▼
[4] media       DB paths → files on disk; derivatives (thumbs, posters, waveform peaks)
      │         → <out>/media/… (the output directory is created here), media rows updated,
      │           missing files flagged
      ▼
[5] build       per chat: query chat.db, emit index.html + app.js + data/*.js chunks + media
      │         → <out>/<date>-<slug>/ , a folder that opens over file://
      │         → optional <out>/index.html linking the chats (never inside a chat folder)
```

Stage contracts:

- **preflight** never decrypts. It reads the protobuf header only, and it is the stage that produces
  every "your backup cannot be opened, and here is why" message. It never guesses.
- **decrypt** writes exactly the decompressed payload to `msgstore.db` — no rewriting, no schema
  change — or raises a clear `IntegrityError`. It does not interpret content.
- **read** is the only module that knows WhatsApp's column names. Everything downstream depends on
  our own schema, so a WhatsApp rename breaks exactly one file.
- **media** is idempotent: a second run skips what exists, reports what is missing, never
  overwrites.
- **build** never mutates the archive; it only reads. Each chat folder is a complete archive: it works
  when copied alone, zipped, or sent to someone who has a browser and nothing else. There is no server
  anywhere in the product: `file://` is the only way an archive is opened.

### CLI surface

```
wexport [folder] [--key <hex|png|keyfile>] [--chat <n|name>]... [--list] [--yes] [--out <dir>]
```

One command, one argument. It finds the backup wherever the copy put it (`Databases/`, `WhatsApp/`,
`Android/media/com.whatsapp/WhatsApp`, a folder holding `msgstore.db.crypt15`, or the file itself),
looks for the key beside the backup (`key.png`, `key.txt`, `encrypted_backup.key`), and otherwise asks
for it once. Then it prints the chats numbered, by newest event first:

```
  #  last event                     msgs  media      size  chat        jid
  1  2026-12-02 12:00:00 +01:00        7      1     100 B  Alice       393331234567@s.whatsapp.net
  2  2026-12-02 11:04:00 +01:00        9      6    7.1 kB  Family      120363000000000001@g.us
```

and asks `which?` — row numbers, part of a name, `all`, or an empty answer for the ten most recent.
`--chat` answers the same question without a terminal, `--yes` builds everything, `--list` only looks.
Archives go to `<folder>/archives` unless `--out` says otherwise, and `<out>/index.html` is always
written: it stays outside every chat folder, because a chat archive must never reveal what else exists.

Media that the user did not copy is **not** an error: it is the normal state of a partial archive,
reported in the build summary and visible in the archive as a placeholder. Only a derivative that was
attempted and failed (a corrupt image, an unreadable video) produces exit `4`.

Exit codes: `0` ok, `1` unreadable input or a question that could not be asked without a terminal,
`2` unusable key, `3` wrong backup type (crypt14 / password / passkey), `4` build incomplete (the
archive is written and usable), `5` missing external dependency.

### Dependency gate

Before any work, the command checks its external binaries; a missing one is **fatal, not a
downgrade**: exit `5`, with the name of the binary and the command that installs it, in English:

```
wexport: ffprobe not found on PATH.
  Debian/Ubuntu   sudo apt install ffmpeg
  Fedora/RHEL     sudo dnf install ffmpeg
  Arch            sudo pacman -S ffmpeg
  Windows         winget install Gyan.FFmpeg
Then re-run the same command.
```

`ffmpeg` and `ffprobe` are required always. `tesseract` is required only when the key is a screenshot:
it is checked at that moment, with the same message shape, so a user who types the 64 digits never
needs it.

---

## 3. Repository layout

Target tree. Directories appear with their first real file, never as empty placeholders.

```
wexport/
├── README.md                    # the product, both trees, how to run either
├── LICENSE                      # GPL-3.0-or-later full text
├── .gitignore
├── docs/                        # shared: one spec, one format reference, one user flow
│   ├── implementation-plan.md   # this file
│   ├── android-backup-formats.md
│   └── user-flow.md
├── .opencode/skills/            # agent skills for working in this repo
│   ├── wexport-implement/       # procedure + invariants + fixtures recipe (Python tree)
│   └── offline-html-archive/    # file:// archive technique
├── py/                          # the original implementation
│   ├── pyproject.toml           # uv, src layout, entry point `wexport`
│   ├── uv.lock
│   ├── README.md                # what this tree is, and how it relates to rust/
│   ├── src/wexport/
│   │   ├── __init__.py
│   │   ├── __main__.py          # `python -m wexport`, and the packager's entry point
│   │   ├── cli.py               # argparse, one command, the numbered picker, exit codes
│   │   ├── errors.py            # exceptions that carry a user-facing message + exit code
│   │   ├── preflight.py         # discovery + header read + key-type gate + binary gate
│   │   ├── keys.py              # hex | png (OCR) | key file → 32 bytes
│   │   ├── backup/              # decrypt.py, merge.py
│   │   ├── db/                  # __init__.py (ro_connect), msgstore.py, slim.py
│   │   ├── media/               # resolve.py, derive.py
│   │   └── site/                # build.py, chunks.py, assets/{index.html,app.js,app.css}
│   └── tests/                   # unit/, integration/, fixtures/synthetic/ (shared with rust/)
├── rust/                        # the rewrite: same product, one static binary
│   ├── Cargo.toml
│   ├── README.md                # module map and the measured parity
│   ├── src/                     # crypt15, keys, discover, msgstore, media, chunks, archive,
│   │                            # progress, ui, assets/
│   └── tests/                   # integration tests against py/tests/fixtures
└── dist/                        # packaged Python binary, gitignored: built, not stored
```

The fixture lives in `py/tests/fixtures/synthetic/phone` and is the one both suites run on: it is
built by `py/tests/fixtures/synthetic/build.py` and contains no real WhatsApp data.

Rules that follow from the layout:

- **Two trees, one contract.** The archive format and the CLI surface are the product; a change to
  either lands in both trees, or the trees diverge. Everything else is an implementation detail.
- No file at `py/src/wexport/` root except `__init__.py`, `cli.py`, `preflight.py`, `keys.py`,
  `errors.py`: the four modules every other module imports, plus the package marker.
- `db/msgstore.py` (Python) and `msgstore.rs` (Rust) are the **only** places a WhatsApp column name or
  table name appears, one per tree.
- Tests exercise module functions, never a subprocess (except the one test that runs the packaged entry
  point). The CLI is covered through its argument-parsing entry for exit codes.
- `site/assets/` (Python) and `src/assets/` (Rust) are the same viewer, copied: plain **classic**
  scripts — no modules (blocked over `file://`), no bundler, no node toolchain.

---

## 4. Data model (build-time `chat.db`)

Built fresh per run; it is derived data and can always be regenerated. It is an **intermediate**: the
build reads it and emits the data chunks the site ships. It is never part of the archive, and deleting
it breaks nothing. Each build carries exactly one row in `chat` — the chat that site belongs to.
`chat_id` stays on `message` anyway, so a future "everything in one archive" mode is a build option,
not a schema migration.

```sql
meta(key TEXT PRIMARY KEY, value TEXT)          -- schema version, source app version, tz, tzdata version,
                                               -- build date
contact(id INTEGER PRIMARY KEY, jid TEXT UNIQUE, name TEXT, is_me INTEGER DEFAULT 0)
chat(id INTEGER PRIMARY KEY, jid TEXT UNIQUE, name TEXT, is_group INTEGER,
     first_ts INTEGER, last_ts INTEGER, message_count INTEGER)
message(id INTEGER PRIMARY KEY, chat_id INTEGER, ts INTEGER, ts_local TEXT, from_me INTEGER,
        sender TEXT, kind TEXT, text TEXT,
        quoted_id INTEGER, edited INTEGER DEFAULT 0, revoked INTEGER DEFAULT 0,
        status INTEGER, payload TEXT)            -- JSON for locations, vcards, calls
media(id INTEGER PRIMARY KEY, message_id INTEGER, kind TEXT, path TEXT,
      mime TEXT, bytes INTEGER, width INTEGER, height INTEGER, duration_ms INTEGER,
      thumb TEXT, peaks TEXT, sha256 TEXT, alt TEXT, name TEXT, missing INTEGER DEFAULT 0)
reaction(message_id INTEGER, sender_jid TEXT, emoji TEXT, ts INTEGER)   -- sender stays NULL for now
receipt(message_id INTEGER, delivered_ts INTEGER, read_ts INTEGER, played_ts INTEGER)
```

Decisions worth stating:

- `media.path`, `media.thumb` and `media.peaks` are paths **relative to the archive root**, so the
  folder can be moved or copied to another disk without a rebuild. The absolute source path is never
  stored: the archive is not a permanent fixture of one machine.
- `kind` is a small closed vocabulary (`text`, `image`, `video`, `audio`, `document`, `sticker`,
  `gif`, `location`, `contact`, `call`, `system`, `deleted`), not WhatsApp's numeric types: the
  mapping lives in `db/msgstore.py` and nowhere else. A poll is currently a `text` message with its
  question — distinguishing it needs the add-on payload, which is the same row a link preview uses,
  so guessing there mislabels ordinary messages.
- Exotic types go in `payload` as JSON instead of one table each. A new WhatsApp feature then costs
  a renderer, not a migration.
- `missing` on `media` is a first-class state: the user did not copy `Media/`, or the file was
  deleted. The archive shows a labelled placeholder, never a broken image.
- `alt` is reserved for an optional re-encoded track (an M4A beside an Opus voice note, a 720p MP4
  beside a video). v1 never fills it: transcoding is off and not exposed.
- `name` keeps the original filename for display: the file on disk is content-addressed, but a person
  reading the archive needs to see `IMG-20261101-WA0003.jpg`, not a SHA-256.
- **Two times per message, and they mean different things.** `ts` is the integer millisecond since
  epoch (UTC) — the sortable truth, what the archive orders by. `ts_local` is the rendered wall-clock
  with an explicit offset (`2026-11-17 14:32:05 +01:00`), **frozen at build time**. The viewer never
  recomputes a local time from `ts`: timezone rules change, sometimes retroactively, and an archive
  opened in ten years must show the hour it actually happened, not the hour a future tzdata would
  derive.
- `meta` records the timezone name the strings were rendered in (the machine's zone, e.g.
  `Europe/Rome`) and the tzdata version used, so a later rebuild is comparable and a reader knows what
  the offsets refer to.
- Displayed time is always the full date and time with its offset, plus a day separator carrying the
  full date and weekday; the tooltip adds the epoch and the UTC form. No "today", no "yesterday", no
  locale-dependent formatting, no rewriting of history when the OS locale changes.

---

## 5. Media handling

| Kind | What the build produces | Notes |
|---|---|---|
| image | 320 px WebP thumbnail + original | EXIF rotated; original kept as-is |
| video, gif | poster JPEG + duration + dimensions, original MP4 | `-movflags +faststart` copy when needed; no transcoding |
| audio (voice note) | peak array (~500 ints) as JSON, original OGG/Opus | waveform drawn without decoding audio in the browser; no re-encode |
| sticker | original WebP/PNG + size class | no thumbnail |
| document | mime icon + filename + size | no preview in v1 |

- Source of every byte: the user's `Media/` folder. Files are stored once **per site**,
  content-addressed, at `media/<first2>/<sha256><ext>` inside the chat folder: inside one chat the
  same photo sent five times is one file. Across per-chat archives the same file is duplicated on
  purpose — a chat folder has to stand alone when it is copied or zipped, and dedupe across archives
  would break that promise.
- The chat grouping lives only in `chat.db` (`media.message_id` → `message.chat_id`), never in a path.
  Renaming a chat, or moving a file between chats, never touches the archive on disk.
- The build hardlinks when the filesystem allows it and copies otherwise: same bytes either way.
- No transcoding: the build touches no video and no audio beyond reading metadata.
- `ffmpeg`/`ffprobe` are **required**, not optional: the startup gate (§2) refuses to run without
  them and prints the install command for the detected platform. No silent degradation — a site
  without thumbnails or playable video is a broken site that looks finished.
- When `ffprobe` fails on a single file (corrupt media), that file is marked and the build continues:
  one broken video is not a reason to abandon an archive.

---

## 6. Archive format and viewer

```
<out>/2026-11-17-family-4f2a/
├── index.html          # shell: opens by double-click, no server, no tool
├── app.js              # classic script: loader, virtualised timeline, renderers, search
├── app.css
├── data/
│   ├── manifest.js     # header, chunk index (name → file, first/last message, counts)
│   └── 2026-11.js      # one chunk per month, newest last; a month over 20k messages is split
│                       # into 2026-11, 2026-11-2, …
├── media/<xx>/<sha256>.<ext>
└── deriv/…             # thumbnails, posters, waveform peaks, same content-addressed scheme
```

The archive is opened with `file://`, which is a hostile environment and decides most of the design:

| On `file://` | Available | Consequence |
|---|---|---|
| classic `<script src="…">` | yes | this is how data chunks are loaded |
| `fetch` / XHR | **no** (origin is `null`) | no JSON files, no SQLite-WASM, no `.db` in the archive |
| ES modules | **no** (CORS) | `app.js` is one classic script |
| Web Workers | **no** (Chrome) | parsing happens on the main thread |
| `localStorage` / `sessionStorage` | **no** (opaque origin) | no saved state; the URL hash is the only state |
| `<img>`, `<video>`, `<audio>` | yes | media and video seeking work, read straight by the browser |

- **Data in chunks**: one classic script per month, newest first, each calling
  `WEX.chunk("2026-11", [...])`. Strings are escaped for `<` so message text can never close a
  `<script>` tag. Opening a chat parses the newest chunk only; scrolling loads the previous one by
  injecting a `<script>` element — the one loading mechanism `file://` allows.
- **Virtualised timeline**: only the messages in and around the viewport exist in the DOM
  (target: ≤ 250 nodes regardless of chat length). This is the requirement that decides the
  architecture, so it is built first, not retrofitted.
- **Bounded memory**: chunks far from the viewport are released. Without this, a 300k-message chat
  would hold the whole conversation in memory and the tab would die on an old laptop.
- **Search** is a linear scan over the loaded chunk data, case- and accent-insensitive, with matches
  highlighted and jumped to. A search on a large chat loads every chunk first (tens of MB, one
  progress bar) — `shortcut: no inverted index; add a precomputed index chunk if searches over 500k
  messages get slow`.
- **Every visible time is absolute**: `2026-11-17 14:32:05 +01:00` on the message, a day separator with
  `Monday, 17 November 2026`, and the epoch plus UTC value on hover. Nothing is computed from the
  browser's clock or locale, so the same folder shows the same strings on any machine, in any year.
- **Renderers per `kind`**, each a small function; unknown `payload` shapes fall back to a raw JSON
  disclosure rather than an exception.
- One page per chat: header (name, participants when it is a group, first and last message with their
  offsets, message count), timeline, media gallery. Navigation between chats exists only in
  `<out>/index.html`, which is never inside a chat folder: a shareable archive must not reveal what
  else exists.
- No framework, no CDN, no bundler, no WASM, no telemetry, no network calls at all.

---

## 7. Milestones

Each milestone ends with something runnable and a stated acceptance test.

### M0 — skeleton and preflight
`pyproject.toml`, CLI, `errors.py`, `keys.py`, `preflight.py`, header parsing, and the synthetic
`crypt15` fixture generator in `tests/fixtures/synthetic/` (M0 cannot be tested without one).
**Acceptance:** `wexport <dir> --list` on the synthetic fixture and on a real backup lists the chats
with format-aware discovery, and `--key` verification either opens the backup or names the problem. On
`crypt14`, on a password backup and on a passkey backup it exits `3` with a message that names the case
and the way out. On a screenshot key it either reads it or says the digits must be typed. `pyproject.toml` records the pinned `wa-crypt-tools` ref,
and a test fails if the header fields we parse stop matching the tested version.

### M1 — decrypt, merge, read
`backup/decrypt.py`, `backup/merge.py`, `db/msgstore.py`, `db/slim.py`.
**Acceptance:** `chat.db` row counts match the source tables; contacts resolved from
`Backups/wa.db` when present, numbers otherwise; an incremental set merges to the same message
count the phone shows; re-running is deterministic (same hashes, same order).

### M2 — media
`media/resolve.py`, `media/derive.py`, the dependency gate (§2), parallel media work with a progress
bar, idempotency.
**Acceptance:** every `media` row is either present with a derivative or `missing=1` with a reason;
second run copies nothing; a 20 GB `Media/` folder finishes without loading files into memory; on a
machine without `ffmpeg`, `build` exits `5` with the install command for that platform and without
touching the output directory.

### M3 — viewer MVP
Virtualised timeline, image/video/audio/document renderers, chat header, the index page.
**Acceptance:** a 100k-message chat opens in under a second and scrolls without jank when
**double-clicked from the file manager** in Chrome, Firefox and Edge; a 3 GB video seeks from
`file://`; every message and every day separator shows the full date and time with its offset, and the
strings do not change when the folder is opened on a machine in a different timezone (test: two
machines or a `TZ` change, no rebuild); a built chat folder still works after being zipped and moved;
nothing loads from the network.

### M4 — search, dates, gallery
Chunked search, jump-to-date, gallery, keyboard navigation.
**Acceptance:** a search over a 300k-message chat returns its first results in under a second,
including loading the chunks; a search that needs every chunk shows progress and does not freeze the
tab; jump-to-date lands on the right message including for DST boundaries.

### M5 — error UX, packaging, docs
Every exit code has a test; **Nuitka `--onefile`** builds for **Linux and Windows** only (macOS is not
built); README and `docs/user-flow.md` checked against a fresh account with no Python installed.
**Acceptance:** a non-technical person follows `docs/user-flow.md` end to end on a machine with no
Python, using only the single binary; the Windows build starts under Defender without a warning, and
the Linux build runs on a distro older than the one it was compiled on.

---

## 8. Testing strategy

- **Unit**: pure functions — key parsing, header parsing, path resolution, payload decoding.
- **Synthetic fixtures, committed**: a `crypt15` generated by the M0 generator from a hand-built
  `msgstore.db` with one message per `kind` value, plus a two-file incremental pair. No personal data
  in git, ever.
- **Integration**: real backup supplied out of band via `WEXPORT_REAL_BACKUP`; skipped when unset.
  The reason is stated in the test docstring so nobody "fixes" the skip into a fake.
- **Golden data**, not golden HTML: the viewer renders through a virtualised list, so comparing markup
  proves nothing. What the suite pins is the emitted chunk bytes and the manifest: building the same
  fixture twice must produce identical files, with the build time held fixed. The viewer is judged in
  M3 by behaviour.
- **No gate is run per change**; the project's checks run once before a push, whole tree, per the
  owner's rules.

---

## 9. Dependencies and license

| Dependency | Why | License |
|---|---|---|
| `wa-crypt-tools` | reference crypt15 decrypt + key-type parsing + OCR of the key screenshot (OCR is the `[ocr]` extra, and needs the `tesseract` binary) | **GPL-3.0-or-later** |
| `Pillow` | thumbnails, EXIF | HPND (permissive) |
| `Nuitka` (build only) | single-binary packaging at M5 | Apache-2.0 |
| `ffmpeg`, `ffprobe` (external) | metadata, poster frames, optional transcode | LGPL/GPL builds vary |
| Python stdlib | sqlite3 (build-time), argparse, hashlib, zoneinfo | PSF |

**Decision (owner, taken):** wexport is **GPL-3.0-or-later** and reuses `wa-crypt-tools` instead of
reimplementing crypt15. The alternative — our own reader (protobuf header + AES-256-GCM + the
HMAC-based key derivation, ~200 lines with `cryptography`) — would allow MIT/Apache and would need a
test suite trusted more than the reference implementation; not worth a milestone. The full licence
text lands with M0; `README.md` and `pyproject.toml` carry the same SPDX identifier.

One caveat that became a decision: the OCR-of-the-screenshot feature sits upstream under
**"Unreleased"** — the released version is 0.2.0. wexport therefore **pins a git ref** of
`wa-crypt-tools`, recorded in `pyproject.toml`, and a test asserts the header fields we parse are the
ones we tested. The OCR path also needs the `tesseract` binary on the host; the CLI degrades to typed
digits, with a message, when it is missing.

**Decision (owner, taken):** no transcoding in v1. The build reads metadata only, keeps the original
bytes, and leaves `media.alt` for the day a browser forces the issue.

---

## 10. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| WhatsApp renames columns or tables (happened in 2.26) | `db/msgstore.py` breaks, everything else survives | single adapter + synthetic fixture pinned to a known app version + version check in `meta` |
| Incremental merge ordering (edits, revokes, deletions) | duplicated or resurrected messages | merge test with a hand-built delta pair; compare against phone counts |
| User copies an old `crypt14`, or a backup made before enabling the key | nothing to open | preflight refuses with exit `3`; user flow says "force a full backup after enabling E2EE" |
| Users lose the 64-digit key | their local backups become unreadable too | screenshot step is mandatory in the flow, with an explicit confirmation |
| Huge `Media/` (tens of GB) | slow builds, disk pressure | no transcoding, hardlink when possible, per-file streaming, progress bar |
| Opus in OGG on Safari | voice notes do not play | not expected: OGG/Opus plays in Chrome, Firefox and Edge; if Safari needs it, fill `media.alt` with an M4A |
| Screenshot-key OCR lives on `wa-crypt-tools` main, not in 0.2.0 | `--key key.png` missing on the released version | pinned git ref (decided) + header fixture test; revisit when the release lands |
| Per-chat sites duplicate shared media | disk grows with the number of chats built | by design, for shareability; `--chat` builds only what is wanted |
| Poll options and votes are not decoded (a poll renders as its question text) | a poll loses its choices | decode the add-on payload when a real backup with polls is available; the add-on row is already read for reactions |
| Nuitka one-file needs a C toolchain and a long build on each platform | releases become a slow manual step, hard to repeat | fixed build images (Ubuntu + Windows runner), one build per release; the AppImage wrapper, if ever wanted, is a second step on the same binary |
| Single binary still expects `ffmpeg` (and `tesseract` for the screenshot key) | the first run fails on a clean machine | exit `5` at startup, naming the missing binary and the install command for that platform (§2); bundling deferred with the Rust work (§12) |
| The archive is a folder, not one file | sending it means zipping; a chat with video is large | the folder is the unit, and the user flow says zip it; media are duplicated per chat on purpose so a folder stands alone |
| `file://` blocks `fetch`, modules, workers and storage | a "modern" front-end would open blank for the recipient | the whole design is built around the constraint (§6); M3 tests double-click in three browsers, not a dev server |

---

## 11. Decisions

Taken by the owner:

1. **Key from a screenshot**: pin an unreleased `wa-crypt-tools` ref rather than wait for the release
   (§9). Tesseract is a host dependency; typed digits always work.
2. **Repository**: private now, public later. Everything is written as if it were public — English
   docs, GPL headers, no hardcoded personal paths.
3. **One site per chat**; the `<out>/index.html` page is the only place that lists chats together
   (§2, §6).
4. **English** in code, CLI, UI and documentation. If the eventual recipient cannot read English, add
   `docs/user-flow.it.md` as a short quick-start; the main artifact stays English.
5. **The archive is plaintext on disk**, with an explicit warning in the build summary. No encrypted
   archive mode.
6. **GPL-3.0-or-later**, reusing `wa-crypt-tools` (§9).
7. **No transcoding** (§5).
8. **`wexport` never touches the network.** No update check, ever.
9. **Packaging: Nuitka `--onefile`**, one single binary per platform, **Linux and Windows only**
   (macOS not built, AppImage not built — the same binary can be wrapped in one later without changing
   anything). `uv tool install wexport` stays the technical path for development. This is §7 M5.
10. **Missing dependencies abort at startup**, exit `5`, with an English message naming the binary and
    the install command per platform (§2). No degraded mode. `tesseract` is demanded only when the key
    is an image.
11. **The archive opens from the filesystem**, and there is no server in the product: no `serve`
    command, no HTTP fallback. Double-clicking `index.html` must work with no wexport installed and no
    browser flag. That is why the data is emitted as classic scripts and why there is no SQLite-WASM
    and no HTTP-only format (§6).
12. **Timestamps are frozen at build.** Every message carries both the epoch milliseconds and a
    rendered ISO-8601 string with an explicit offset; the viewer never derives a local time from the
    epoch, so reopening the archive in ten years, on a machine in another timezone, shows the time it
    actually happened (§4, §6). The zone is the machine's, recorded in the manifest; there is no flag
    to change it in v1, because a wrong zone is better fixed by building on the right machine.

No decision is open.

## 12. Later, deliberately not now

- **Rust rewrite of the core** (decrypt, `msgstore` adapter, `chat.db` writer, media orchestration,
  chunk emitter) producing a native ~20 MB binary with the viewer assets embedded — decided as the
  eventual direction, explicitly **after v1**, not before. What it would buy: no Python runtime,
  millisecond startup, no interpreter inside the one-file binary, and freedom from the pinned upstream
  ref. What it costs: reimplementing the adapter that breaks whenever WhatsApp renames a column, which
  is the reason v1 reuses `wa-crypt-tools` at all. Nothing in v1 should be written in a way that only
  makes sense in Python — the pipeline stages already have file-based contracts, the viewer is plain
  scripts, and there is no wasm runtime or HTTP-only format to port, which is what makes the rewrite
  possible later.
- ffmpeg/tesseract bundling: deferred with it. Both single-binary builds still expect them on the
  host and degrade with a message when absent.
