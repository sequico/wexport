# Implementation plan

Status: v1.0.0 implemented. This document remains the **specification** — what the product must be;
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
[1] discover    find the files, read the crypt15 header, decide whether this is openable at all
      │         → Inputs{root, database, incrementals, contacts, media, ignored} + Header{iv,
      │           app_version, key_type, header_len}
      ▼
[2] decrypt     crypt15 → msgstore.db in a temporary workspace (+ incrementals attached and
      │         INSERT OR REPLACE'd into it)
      │         → one SQLite file, schema untouched
      ▼
[3] read        source schema (WhatsApp 2.26) → our own model
      │         → Chat / ChatStat / Message{kind, text, media, reactions, payload} in memory
      ▼
[4] media       DB paths → files on disk; derivatives (thumbs, posters, waveform peaks)
      │         → <out>/<chat>/media/…, rows filled, missing files flagged
      ▼
[5] build       per chat: the in-memory records → index.html + app.js + data/*.js chunks + media
      │         → <out>/<date>-<slug>/ , a folder that opens over file://
      │         → <out>/index.html linking the chats (never inside a chat folder)
```

Stage contracts:

- **discover** never decrypts. It reads the protobuf header only, and it is the stage that produces
  every "your backup cannot be opened, and here is why" message. It never guesses.
- **decrypt** produces exactly the decompressed payload — no rewriting, no schema change — or fails
  with a message naming the cause. It does not interpret content.
- **read** is the only module that knows WhatsApp's column names. Everything downstream depends on
  our own model, so a WhatsApp rename breaks exactly one file.
- **media** is idempotent: a second run skips what exists, reports what is missing, never
  overwrites.
- **build** never mutates the archive; it only reads. Each chat folder is a complete archive: it works
  when copied alone, zipped, or sent to someone who has a browser and nothing else. There is no server
  anywhere in the product: `file://` is the only way an archive is opened.

### CLI surface

```
wexport [FOLDER] [-k KEY] [-l [N] [ORDER]] [-r] [-b CODES] [-o DIR]
```

One command, one argument. It finds the backup wherever the copy put it (`Databases/`, `WhatsApp/`,
`Android/media/com.whatsapp/WhatsApp`, a folder holding `msgstore.db.crypt15`, or the file itself),
looks for the key beside the backup (`key.png`, `key.txt`, `encrypted_backup.key`), and otherwise asks
for it once. Then it prints the chats numbered, by newest event first:

```
  #  last event                     msgs  media      size  chat
  1  2026-12-02 12:00:00 +01:00        7      1     100 B  Alice
  2  2026-12-02 11:04:00 +01:00        9      6    7.1 kB  Family
```

The code in the first column is what `-b` takes. `-l` lists and exits (it is also the default),
`-r` reverses that order, `-b` builds by the codes the list shows (`-b all` builds everything), and
`-l N ORDER` cuts it (by recency, size or name).
Archives go to `<folder>/output` unless `-o` says otherwise, and `<out>/index.html` is always
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
├── README.md                    # the product: what it does, how to run it
├── LICENSE                      # GPL-3.0-or-later full text
├── Cargo.toml                   # one crate, one binary, viewer assets embedded
├── Cargo.lock
├── .gitignore
├── docs/
│   ├── implementation-plan.md   # this file
│   ├── android-backup-formats.md
│   ├── usage.md
│   └── figures/                 # the phone screens usage.md shows
├── .opencode/skills/            # agent skills for working in this repo
│   ├── wexport-implement/       # procedure + invariants + fixtures recipe
│   └── offline-html-archive/    # file:// archive technique
├── src/
│   ├── main.rs                  # the binary: one command, exit codes
│   ├── lib.rs                   # the module map
│   ├── crypt15.rs               # varint + protobuf header, AES-256-GCM, zlib/ZIP
│   ├── keys.rs                  # hex | png (tesseract) | key file → 32 bytes
│   ├── discover.rs              # discovery, key-type gate, binary gate
│   ├── error.rs                 # the message a person reads + the exit code a script reads
│   ├── msgstore.rs              # the source-schema adapter (WhatsApp 2.26 names)
│   ├── merge.rs                 # incremental replay into the full backup
│   ├── media.rs                 # path resolution, content addressing, derivatives
│   ├── chunks.rs                # records → classic-script chunks, escaping
│   ├── archive.rs               # one folder per chat, manifest, README, index, prune
│   ├── progress.rs              # the one-line media progress bar
│   ├── ui.rs                    # flags, the chat codes, the run
│   └── assets/                  # the viewer: index.html, app.js, app.css (classic scripts)
└── tests/
    ├── fixture.rs               # end-to-end runs on the committed fixture
    ├── real.rs                  # optional run on a real backup, via WEXPORT_REAL_BACKUP
    └── fixtures/
        ├── synthetic/           # schema, data, generator and the crypt15 files it commits
        └── real/                # gitignored, supplied via WEXPORT_REAL_BACKUP
```

The fixture lives in `tests/fixtures/synthetic/phone`: it is built by
`tests/fixtures/synthetic/build.py` and contains no real WhatsApp data.

Rules that follow from the layout:

- `src/msgstore.rs` is the **only** place a WhatsApp column name or table name appears. Everything
  downstream works on our own `Chat` / `Message` / `Media` types.
- No module outside `src/ui.rs` and `src/main.rs` reads the command line or prints to the terminal.
- Tests exercise module functions; only the integration suite runs the command end to end. The CLI is
  covered through `ui::run(argv)` for exit codes.
- `src/assets/` holds the viewer as plain **classic** scripts — no modules (blocked over `file://`), no
  bundler, no node toolchain — embedded in the binary with `include_str!`.

---

## 4. Data model (the chunk record)

The build reads the source database and turns each row into one record, written once into the month
chunk as JSON. There is no intermediate database: the records live in memory for the length of one
chat, which is also why a rebuild can never half-update an archive.

Where a field is absent from the record, it is absent on purpose — an empty string, a `0` status or a
`false` flag is not written. That is what keeps a chunk small, and what makes two builds of the same
data byte-identical.

| Message field | Meaning |
|---|---|
| `i` | message id in the source database (negative for call-log rows, which share no id space) |
| `t` | epoch milliseconds (UTC): the sortable truth |
| `s` | the same moment rendered with its offset, frozen (see below) |
| `k` | `kind`: `text`, `image`, `video`, `audio`, `document`, `sticker`, `gif`, `location`, `contact`, `call`, `system`, `deleted` |
| `f` | `1` when the message is the user's own |
| `a` | group sender, as a name when `wa.db` knows it, as a number otherwise |
| `x` | text body, or the caption when the message carries media with no text |
| `q` | id of the quoted message |
| `e` | `1` when the message was edited |
| `r` | `1` when it was revoked |
| `st` | delivery status, when non-zero |
| `pl` | payload for types with structure: `{lat, lon}`, `{name}`, `{duration_s, video, missed}` |
| `rx` | reactions, emoji only |
| `md` | the media row below, when the message carries a file |

| Media field | Meaning |
|---|---|
| `p` | path **relative to the archive root**: `media/ab/ab12….jpg` |
| `sz` | size in bytes, from the database or from the file |
| `mt`, `nm` | mime type and the original filename, for display |
| `w`, `h`, `d` | dimensions and duration |
| `th` | thumbnail or poster, also relative |
| `pk` | waveform peaks: 500 amplitudes 0–100, inline |
| `mi` | `1` when the row points at a file the user did not copy |

The manifest carries the chat title, `is_group`, the timezone name and tzdata version, the source app
version, the builder version, the build instant, counts, the first and last rendered timestamps, and
the chunk index (name, count, first/last message).

Decisions worth stating:

- Media paths are **relative to the archive root**, so the folder can be moved or copied to another
  disk without a rebuild. The absolute source path is never stored: the archive is not a permanent
  fixture of one machine.
- `kind` is a small closed vocabulary, not WhatsApp's numeric types: the mapping lives in
  `src/msgstore.rs` and nowhere else. A poll is currently a `text` message with its question —
  distinguishing it needs the add-on payload, which is the same row a link preview uses, so guessing
  there mislabels ordinary messages.
- Exotic types go in `pl` as JSON instead of one table each. A new WhatsApp feature then costs a
  renderer, not a schema change.
- A media row whose file is absent is a first-class state, not an error: the user did not copy
  `Media/`, or the file was deleted. The archive shows a labelled placeholder, never a broken image.
- `alt` is reserved for an optional re-encoded track (an M4A beside an Opus voice note, a 720p MP4
  beside a video). v1 never fills it: transcoding is off and not exposed.
- `nm` keeps the original filename for display: the file on disk is content-addressed, but a person
  reading the archive needs to see `IMG-20261101-WA0003.jpg`, not a SHA-256.
- **Two times per message, and they mean different things.** `t` is the integer millisecond since
  epoch (UTC) — the sortable truth, what the archive orders by. `s` is the rendered wall-clock with an
  explicit offset (`2026-11-17 14:32:05 +01:00`), **frozen at build time**. The viewer never
  recomputes a local time from `t`: timezone rules change, sometimes retroactively, and an archive
  opened in ten years must show the hour it actually happened, not the hour a future tzdata would
  derive.
- The manifest records the timezone name the strings were rendered in (the machine's zone, e.g.
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
- The chat a file belongs to never appears in its path: placement is by content, so renaming a chat,
  or the same file arriving in two chats, never orphans anything.
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

The date is the last message in the chat and the slug is the chat name plus four hex digits of its
jid. A rebuild after new messages therefore names a new folder: the build removes the previous folder
of the same chat first, so one chat never leaves two copies behind. A chat renamed on the phone keeps
the folder under its earlier name.

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
  `<script>` tag. Opening a chat parses the chunks the last screen needs — usually the newest one or
  two — and scrolling loads the previous one by injecting a `<script>` element — the one loading
  mechanism `file://` allows.
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
`Cargo.toml`, the CLI in `src/ui.rs`, `src/error.rs`, `src/keys.rs`, `src/discover.rs`, header parsing in
`src/crypt15.rs`, and the synthetic `crypt15` fixture generator in `tests/fixtures/synthetic/`
(M0 cannot be tested without one).
**Acceptance:** `wexport <dir> -l` on the synthetic fixture and on a real backup lists the chats
with format-aware discovery, and `-k` verification either opens the backup or names the problem. On
`crypt14`, on a password backup and on a passkey backup it exits `3` with a message that names the case
and the way out. On a screenshot key it either reads it or says the digits must be typed. The header
parser is pinned by a fixture test, and the key derivation by a test against the value the reference
implementation computes.

### M1 — decrypt, merge, read
`src/crypt15.rs`, `src/merge.rs`, `src/msgstore.rs`.
**Acceptance:** message and media counts match the source tables (a media row with no path is not
media); contacts resolved from `Backups/wa.db` when present, numbers otherwise; an incremental set
merges to the same message
count the phone shows; re-running is deterministic (same hashes, same order).

### M2 — media
`src/media.rs`, the dependency gate (§2), parallel media work with a progress
bar, idempotency.
**Acceptance:** every `media` row is either present with a derivative or `missing=1` with a reason;
second run copies nothing; a 20 GB `Media/` folder finishes without loading files into memory; on a
machine without `ffmpeg`, `-b` exits `5` with the install command for that platform and without
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
Every exit code has a test; **`cargo build --release`** produces one static binary per platform
(**Linux and Windows**; macOS is not built). README and `docs/usage.md` checked against a fresh
account with nothing but the binary.
**Acceptance:** a non-technical person follows `docs/usage.md` end to end on a machine with no
toolchain, using only the single binary; the Windows build starts under Defender without a warning, and
the Linux build is statically linked against glibc-free targets or runs on a distro older than the one
it was compiled on.

---

## 8. Testing strategy

- **Unit**: pure functions — key parsing, header parsing, path resolution, payload decoding, chunk
  splitting.
- **Synthetic fixtures, committed**: a `crypt15` generated by the M0 generator from a hand-built
  `msgstore.db` with one message per `kind` value, plus a two-file incremental pair. No personal data
  in git, ever.
- **Integration**: real backup supplied out of band via `WEXPORT_REAL_BACKUP`; skipped when unset.
  The reason is stated in the test so nobody "fixes" the skip into a fake.
- **Golden data**, not golden HTML: the viewer renders through a virtualised list, so comparing markup
  proves nothing. What the suite pins is the emitted chunk bytes and the manifest: building the same
  fixture twice must produce identical files, with the build time held fixed. The viewer is judged in
  M3 by behaviour.
- **No gate is run per change**; the project's checks run once before a push, whole tree, per the
  owner's rules. The list lives in `scripts/gate.sh` (push, ~20 s) and `scripts/gate-release.sh`
  (release, with mutation testing, fuzzing, ThreadSanitizer and the `--locked` build).

---

## 9. Dependencies and license

| Dependency | Why | License |
|---|---|---|
| `aes-gcm`, `hmac`, `sha2`, `sha1`, `md-5`, `flate2`, `zip` | crypt15 container: AES-256-GCM, the HMAC key derivation, zlib and ZIP payloads | Apache-2.0 / MIT |
| `rusqlite` (bundled SQLite) | read the msgstore, merge incrementals, read `wa.db` | MIT |
| `serde_json` | records and the manifest | Apache-2.0 / MIT |
| `jiff` | frozen timestamps with an explicit offset, and the zone database | Unlicense / MIT |
| `ffmpeg`, `ffprobe` (external) | metadata, thumbnails, poster frames, waveform peaks | LGPL/GPL builds vary |
| `tesseract` (external, optional) | read the 64 digits from a screenshot | Apache-2.0 |

**Decision (owner, taken):** wexport is **GPL-3.0-or-later**. The crypt15 reader — varint + protobuf
header, AES-256-GCM, the nested-HMAC key derivation, the zlib/ZIP payloads — is a port of
`wa-crypt-tools` (GPL-3.0), and a test pins the derivation to the value that reference implementation
computes, so a silent change cannot slip through.

A trade that comes with porting instead of linking: the reader is ours, which means a change in the
container format is our bug to find, not an upstream release to wait for. The header fixture test and
the real-backup run in `tests/real.rs` are what keep that honest. A screenshot key needs the
`tesseract` binary on the host; the CLI degrades to typed digits, with a message, when it is missing.

**Decision (owner, taken):** no transcoding in v1. The build reads metadata only, keeps the original
bytes, and leaves the optional re-encoded track for the day a browser forces the issue.

---

## 10. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| WhatsApp renames columns or tables (happened in 2.26) | `src/msgstore.rs` breaks, everything else survives | single adapter + synthetic fixture pinned to a known app version |
| The crypt15 container changes (field meaning, tail layout) | decryption fails on new phones | header fixture test, key derivation pinned to the reference value, real-backup run in `tests/real.rs` |
| Incremental merge ordering (edits, revokes, deletions) | duplicated or resurrected messages | merge test with a hand-built delta pair; compare against phone counts |
| User copies an old `crypt14`, or a backup made before enabling the key | nothing to open | preflight refuses with exit `3`; user flow says "force a full backup after enabling E2EE" |
| Users lose the 64-digit key | their local backups become unreadable too | screenshot step is mandatory in the flow, with an explicit confirmation |
| Huge `Media/` (tens of GB) | slow builds, disk pressure | no transcoding, hardlink when possible, per-file streaming, progress bar |
| Opus in OGG on Safari | voice notes do not play | not expected: OGG/Opus plays in Chrome, Firefox and Edge; if Safari needs it, fill `media.alt` with an M4A |
| Screenshot-key OCR depends on `tesseract` being installed | `-k key.png` fails on a clean machine | exit `5` naming the binary and the install command; typed digits always work |
| Per-chat sites duplicate shared media | disk grows with the number of chats built | by design, for shareability; `-b` builds only what is wanted |
| Poll options and votes are not decoded (a poll renders as its question text) | a poll loses its choices | decode the add-on payload when a real backup with polls is available; the add-on row is already read for reactions |
| A single static binary still expects `ffmpeg` (and `tesseract` for the screenshot key) | the first run fails on a clean machine | exit `5` at startup, naming the missing binary and the install command for that platform (§2); bundling deferred (§12) |
| The archive is a folder, not one file | sending it means zipping; a chat with video is large | the folder is the unit, and the user flow says zip it; media are duplicated per chat on purpose so a folder stands alone |
| `file://` blocks `fetch`, modules, workers and storage | a "modern" front-end would open blank for the recipient | the whole design is built around the constraint (§6); M3 tests double-click in three browsers, not a dev server |

---

## 11. Decisions

Taken by the owner:

1. **Key from a screenshot**: read it with `tesseract` at the moment the key is needed. It is a host
   dependency; typed digits always work.
2. **Repository**: private now, public later. Everything is written as if it were public — English
   docs, GPL headers, no hardcoded personal paths.
3. **One site per chat**; the `<out>/index.html` page is the only place that lists chats together
   (§2, §6).
4. **English** in code, CLI, UI and documentation. If the eventual recipient cannot read English, add
   `docs/usage.it.md` as a short quick-start; the main artifact stays English.
5. **The archive is plaintext on disk**, with an explicit warning in the build summary. No encrypted
   archive mode.
6. **GPL-3.0-or-later**, with the crypt15 reader ported from `wa-crypt-tools` (§9).
7. **No transcoding** (§5).
8. **`wexport` never touches the network.** No update check, ever.
9. **Packaging: one release binary per platform from `cargo build --release`**, **Linux and Windows
   only** (macOS not built, AppImage not built — the same binary can be wrapped in one later without
   changing anything). This is §7 M5.
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
    to change it, because a wrong zone is better fixed by building on the right machine.

No decision is open.

## 12. Later, deliberately not now

- **ffmpeg/tesseract bundling**: both builds still expect them on the host and degrade with a message
  when absent. Bundling them into the release archive is a packaging change on the same binary.
- **Transcoding**: off and not exposed (§5); the record has room for an alternative track.
- **Polls and other add-on payloads**: a poll currently renders as its question text. Decode the
  add-on row when a real backup with polls is available.
- **Cross-compiled release builds** for Linux (musl) and Windows from one runner, with the interface
  test run under Wine; until then, a release is a build per platform.
