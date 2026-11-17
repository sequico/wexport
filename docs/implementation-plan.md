# Implementation plan

Status: planning. Android only, `crypt15` backups, 64-digit key, media folder copied by the user.
Everything below is the v1 target; nothing is implemented yet.

---

## 1. Scope

**In scope**

| | |
|---|---|
| Platform | Android (WhatsApp and WhatsApp Business) |
| Backup format | `msgstore.db.crypt15` + `msgstore-increment-*.db.crypt15`, plus the rolling `msgstore-YYYY-MM-DD.1.db.crypt15` copies when present |
| Key | 32-byte root key: 64 hex digits typed in, a `.png` screenshot, or an `encrypted_backup.key` file (root only) |
| Input | A folder the user copied from the phone over USB/MTP (`Databases/`, `Backups/`, `Media/`) |
| Output | one **self-contained site per chat**, `<out>/<date>-<slug>/` — HTML + JS + media + `chat.db`, opened through `wexport serve` |

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

Six stages, each a module with a narrow contract so it can be tested alone.

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
[5] build       per chat: chat.db + assets + manifest → <out>/<date>-<slug>/
      │         → one folder that stands alone; optional <out>/index.html linking them (never
      │           inside a chat folder)
      ▼
      │
      ▼
[6] serve       http.server with Range support, no caching, localhost only
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
- **build** never mutates the archive; it only reads. Each chat folder is a complete site: it works
  when copied alone, zipped, or sent to someone who has wexport and nothing else of yours.
- **serve** is optional (a user with a static-host habit can open the folder any other way).

### CLI surface

```
wexport check <input-dir> [--key <hex|png|keyfile>]
wexport build <input-dir> --key <hex|png|keyfile> --out <out-dir>
              [--all | --chat <jid> ...] [--index] [--jobs N]
              [--transcode {none,audio,video,all}] [--link]
wexport serve <site-dir> [--port 8765]
```

`build` defaults to `--all` (one folder per chat). `--chat` takes one or more JIDs and produces only
those folders, which is the sharing path: hand someone the one folder you chose. `--index` additionally
writes `<out>/index.html` with the list of built chats, and that file is deliberately kept outside
every chat folder, because a chat site must never reveal what else exists.

Three commands, `argparse` only. Exit codes: `0` ok, `1` unreadable input, `2` unusable key,
`3` wrong backup type (crypt14 / password / passkey), `4` build incomplete (some derivatives could
not be produced; the site is written and usable), `5` missing external dependency.

Media that the user did not copy is **not** an error: it is the normal state of a partial archive,
reported in the build summary and visible in the site as a placeholder. Only a derivative that was
attempted and failed (a corrupt image, an unreadable video) produces exit `4`.

### Dependency gate

Before any work, every command except `--help`, `--version` and `serve` checks its external binaries.
A missing one is **fatal, not a downgrade**: exit `5`, with the name of the binary and the command
that installs it, on one screen and in English:

```
wexport: ffprobe not found on PATH.
  Debian/Ubuntu   sudo apt install ffmpeg
  Fedora/RHEL     sudo dnf install ffmpeg
  Arch            sudo pacman -S ffmpeg
  Windows         winget install Gyan.FFmpeg
Then re-run the same command. `wexport check` prints the full environment report.
```

`ffmpeg` and `ffprobe` are required for `check` and `build`. `tesseract` is required only when the key
is given as a screenshot: it is checked at that moment, with the same message shape, so a user who
types the 64 digits never needs it. `serve` requires nothing at all.

---

## 3. Repository layout

Target tree. Directories appear with their first real file, never as empty placeholders.

```
wexport/
├── README.md
├── LICENSE                      # GPL-3.0-or-later full text, added with M0
├── pyproject.toml               # uv, src layout, entry point `wexport`
├── .gitignore
├── docs/
│   ├── implementation-plan.md   # this file
│   ├── android-backup-formats.md
│   └── user-flow.md
├── src/wexport/
│   ├── __init__.py
│   ├── cli.py                   # argparse, 3 subcommands, exit codes
│   ├── errors.py                # exceptions that carry a user-facing message + exit code
│   ├── preflight.py             # discovery + header read + key-type gate
│   ├── keys.py                  # hex | png (OCR) | key file → 32 bytes
│   ├── backup/
│   │   ├── decrypt.py           # crypt15 → msgstore.db
│   │   └── merge.py             # incremental replay, ordering, edits/revokes
│   ├── db/
│   │   ├── msgstore.py          # source-schema adapter (WhatsApp 2.26 names)
│   │   ├── slim.py              # our schema + writer
│   │   └── fts.py               # FTS5 index build and query fixtures
│   ├── media/
│   │   ├── resolve.py           # DB path → file, missing detection, dedupe by hash
│   │   └── derive.py            # thumbnails, posters, waveform peaks, optional transcode
│   ├── site/
│   │   ├── build.py             # per-chat site builder, index page, manifest
│   │   ├── serve.py             # Range-capable handler
│   │   └── assets/
│   │       ├── index.html       # shell
│   │       ├── app.js           # router, chat list, virtualised timeline, renderers
│   │       ├── app.css
│   │       └── vendor/          # sqlite-wasm build, pinned, licence file next to it
└── tests/
    ├── unit/
    ├── integration/
    └── fixtures/
        ├── synthetic/           # crypt15 files we generate ourselves, safe to commit
        └── real/                # gitignored, supplied via WEXPORT_REAL_BACKUP
```

Rules that follow from the layout:

- No file at `src/wexport/` root except `__init__.py`, `cli.py`, `preflight.py`, `keys.py`,
  `errors.py`: the four modules every other module imports, plus the package marker.
- `db/msgstore.py` is the **only** place a WhatsApp column name or table name appears.
- Tests exercise module functions, never a subprocess. The CLI is covered through `main(argv)` for
  argument parsing and exit codes, which is where its logic lives.
- `site/assets/` is plain ES modules — no bundler, no node toolchain. If the viewer ever needs a
  build step it is because a dependency was added that should not have been.

---

## 4. Data model (`chat.db`, our schema)

Built fresh per run; it is derived data and can always be regenerated. Slim on purpose: the viewer
queries it in the browser through SQLite-WASM. Each site carries its own `chat.db` with exactly one
row in `chat` — the chat that site belongs to. `chat_id` stays on `message` anyway, so a future
"everything in one site" mode is a build option, not a schema migration.

```sql
meta(key TEXT PRIMARY KEY, value TEXT)          -- schema version, source app version, build date
contact(id INTEGER PRIMARY KEY, jid TEXT UNIQUE, name TEXT, is_me INTEGER DEFAULT 0)
chat(id INTEGER PRIMARY KEY, jid TEXT UNIQUE, name TEXT, is_group INTEGER,
     first_ts INTEGER, last_ts INTEGER, message_count INTEGER)
message(id INTEGER PRIMARY KEY, chat_id INTEGER, ts INTEGER, from_me INTEGER,
        sender_jid TEXT, kind TEXT, text TEXT,
        quoted_id INTEGER, edited_ts INTEGER, revoked INTEGER DEFAULT 0,
        status INTEGER, payload TEXT)            -- JSON for polls, locations, vcards, calls
media(id INTEGER PRIMARY KEY, message_id INTEGER, kind TEXT, path TEXT,
      mime TEXT, bytes INTEGER, width INTEGER, height INTEGER, duration_ms INTEGER,
      thumb TEXT, peaks TEXT, sha256 TEXT, missing INTEGER DEFAULT 0)
reaction(message_id INTEGER, sender_jid TEXT, emoji TEXT, ts INTEGER)
receipt(message_id INTEGER, delivered_ts INTEGER, read_ts INTEGER, played_ts INTEGER)
message_fts USING fts5(text, content='message', content_rowid='id')
```

Decisions worth stating:

- `media.path`, `media.thumb` and `media.peaks` are paths **relative to `site/`**, so the site can be
  moved or copied to another disk without a rebuild. The absolute source path is never stored: the
  archive is not a permanent fixture of one machine.
- `message_fts` is populated once at build and `chat.db` is write-once afterwards, so no FTS triggers
  and no `rebuild` command are needed.
- `kind` is a small closed vocabulary (`text`, `image`, `video`, `audio`, `document`, `sticker`,
  `gif`, `ptv`, `location`, `contact`, `poll`, `call`, `system`, `deleted`), not WhatsApp's numeric
  types: the mapping lives in `db/msgstore.py` and nowhere else.
- Exotic types go in `payload` as JSON instead of one table each. A new WhatsApp feature then costs
  a renderer, not a migration.
- `missing` on `media` is a first-class state: the user did not copy `Media/`, or the file was
  deleted. The site shows a labelled placeholder, never a broken image.
- Timestamps are integer milliseconds since epoch, UTC. Formatting is the viewer's job.

---

## 5. Media handling

| Kind | What the build produces | Notes |
|---|---|---|
| image | 320 px WebP thumbnail + original | EXIF rotated; original kept as-is |
| video, gif, ptv | poster JPEG + duration + dimensions, original MP4 | `-movflags +faststart` copy when needed; 720p transcode only with `--transcode video` |
| audio (voice note) | peak array (~500 ints) as JSON, original OGG/Opus | waveform drawn without decoding audio in the browser; M4A/AAC track only with `--transcode audio` |
| sticker | original WebP/PNG + size class | no thumbnail |
| document | mime icon + filename + size | no preview in v1 |

- Source of every byte: the user's `Media/` folder. Files are stored once **per site**,
  content-addressed, at `site/media/<first2>/<sha256><ext>`: inside one chat the same photo sent five
  times is one file. Across per-chat sites the same file is duplicated on purpose — a chat folder has
  to stand alone when it is copied or zipped, and dedupe across sites would break that promise.
- The chat grouping lives only in `chat.db` (`media.message_id` → `message.chat_id`), never in a path.
  Renaming a chat, or moving a file between chats, never touches the archive on disk.
- The build copies by default, or hardlinks with `--link` (same filesystem only).
- Transcoding is opt-in. Default build touches no video and no audio beyond metadata read.
- `ffmpeg`/`ffprobe` are **required**, not optional: the startup gate (§2) refuses to run without
  them and prints the install command for the detected platform. No silent degradation — a site
  without thumbnails or playable video is a broken site that looks finished.
- When `ffprobe` fails on a single file (corrupt media), that file is marked and the build continues:
  one broken video is not a reason to abandon an archive.

---

## 6. Viewer

- One page per chat: header (name, participants, date range, message count), timeline, media
  drawer/gallery. Navigation between chats exists only in `<out>/index.html`, outside the site.
- **Virtualised timeline**: only the messages in and around the viewport exist in the DOM
  (target: ≤ 250 nodes regardless of chat length). This is the requirement that decides the
  architecture, so it is built first, not retrofitted.
- Data access through SQLite-WASM over HTTP (Range requests work; `file://` does not — Chrome
  blocks module and `fetch` from `file://`, and video cannot seek without Range).
- Renderers per `kind`, each a small function; unknown `payload` shapes fall back to a raw JSON
  disclosure rather than an exception.
- Search: FTS5 query against the site's own `chat.db`. Global search across chats would need a site
  that knows every chat, which is exactly what a shareable chat site must not be; it is out of v1.
- Jump-to-date, gallery, keyboard navigation (`j`/`k`, `/` for search).
- No framework, no CDN, no bundler, no telemetry, no network calls at all.

---

## 7. Milestones

Each milestone ends with something runnable and a stated acceptance test.

### M0 — skeleton and preflight
`pyproject.toml`, CLI, `errors.py`, `keys.py`, `preflight.py`, header parsing, and the synthetic
`crypt15` fixture generator in `tests/fixtures/synthetic/` (M0 cannot be tested without one).
**Acceptance:** `wexport check <dir>` on the synthetic fixture and on a real backup prints format,
app version, key type, file list, total size. On `crypt14`, on a password backup and on a passkey
backup it exits `3` with a message that names the case and the way out. On a screenshot key it either
reads it or says the digits must be typed. `pyproject.toml` records the pinned `wa-crypt-tools` ref,
and a test fails if the header fields we parse stop matching the tested version.

### M1 — decrypt, merge, read
`backup/decrypt.py`, `backup/merge.py`, `db/msgstore.py`, `db/slim.py`, `db/fts.py`.
**Acceptance:** `chat.db` row counts match the source tables; contacts resolved from
`Backups/wa.db` when present, numbers otherwise; an incremental set merges to the same message
count the phone shows; re-running is deterministic (same hashes, same order).

### M2 — media
`media/resolve.py`, `media/derive.py`, the startup dependency gate (§2), `--jobs` parallelism,
idempotency.
**Acceptance:** every `media` row is either present with a derivative or `missing=1` with a reason;
second run copies nothing; a 20 GB `Media/` folder finishes without loading files into memory; on a
machine without `ffmpeg`, `build` exits `5` with the install command for that platform and without
touching the output directory.

### M3 — viewer MVP
Virtualised timeline, image/video/audio/document renderers, chat header, `--index` page, `serve`.
**Acceptance:** a 100k-message chat opens in under a second and scrolls without jank; a 3 GB video
seeks correctly over `wexport serve`; a built chat folder still works after being zipped, moved to
another machine and opened there; nothing loads from the network.

### M4 — search, dates, gallery
FTS5 wiring, jump-to-date, per-chat gallery, keyboard navigation.
**Acceptance:** a phrase search over a 300k-message chat returns in under 100 ms locally; jump-to-date
lands on the right message including for DST boundaries.

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
  proves nothing. The snapshot is `chat.db` (schema plus rows in a fixed order) and the site manifest;
  the viewer is judged in M3 by behaviour.
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
| SQLite-WASM build (vendored asset) | viewer data access | public domain / Apache-2.0 (confirm at vendoring) |
| Python stdlib | sqlite3, http.server, argparse, hashlib | PSF |

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

**Decision (owner, taken):** transcoding is opt-in. `wexport build` reads metadata only; `--transcode
audio` / `video` / `all` are the explicit paths that re-encode, and the default build never does.

---

## 10. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| WhatsApp renames columns or tables (happened in 2.26) | `db/msgstore.py` breaks, everything else survives | single adapter + synthetic fixture pinned to a known app version + version check in `meta` |
| Incremental merge ordering (edits, revokes, deletions) | duplicated or resurrected messages | merge test with a hand-built delta pair; compare against phone counts |
| User copies an old `crypt14`, or a backup made before enabling the key | nothing to open | preflight refuses with exit `3`; user flow says "force a full backup after enabling E2EE" |
| Users lose the 64-digit key | their local backups become unreadable too | screenshot step is mandatory in the flow, with an explicit confirmation |
| Huge `Media/` (tens of GB) | slow builds, disk pressure | no-transcode default, `--link`, per-file streaming, progress output |
| Opus in OGG on Safari | voice notes do not play | optional M4A track on demand (`--transcode audio`) |
| Screenshot-key OCR lives on `wa-crypt-tools` main, not in 0.2.0 | `--key key.png` missing on the released version | pinned git ref (decided) + header fixture test; revisit when the release lands |
| Per-chat sites duplicate shared media | disk grows with the number of chats built | by design, for shareability; `--chat` builds only what is wanted |
| Nuitka one-file needs a C toolchain and a long build on each platform | releases become a slow manual step, hard to repeat | fixed build images (Ubuntu + Windows runner), one build per release; the AppImage wrapper, if ever wanted, is a second step on the same binary |
| Single binary still expects `ffmpeg` (and `tesseract` for the screenshot key) | the first run fails on a clean machine | exit `5` at startup, naming the missing binary and the install command for that platform (§2); bundling deferred with the Rust work (§12) |
| `site/` is a folder, not a single file | user copies it somewhere else and nothing opens | `site/` opens through `wexport serve`; relative paths only, and the user flow says so |

---

## 11. Decisions

Taken by the owner:

1. **Key from a screenshot**: pin an unreleased `wa-crypt-tools` ref rather than wait for the release
   (§9). Tesseract is a host dependency; typed digits always work.
2. **Repository**: private now, public later. Everything is written as if it were public — English
   docs, GPL headers, no hardcoded personal paths.
3. **One site per chat**, `--index` as the only page that lists chats together (§2, §6).
4. **English** in code, CLI, UI and documentation. If the eventual recipient cannot read English, add
   `docs/user-flow.it.md` as a short quick-start; the main artifact stays English.
5. **The site is plaintext on disk**, with an explicit warning in the build summary. No encrypted
   archive mode.
6. **GPL-3.0-or-later**, reusing `wa-crypt-tools` (§9).
7. **Transcoding opt-in** (§5).
8. **`wexport check` never touches the network.** No update check, ever.
9. **Packaging: Nuitka `--onefile`**, one single binary per platform, **Linux and Windows only**
   (macOS not built, AppImage not built — the same binary can be wrapped in one later without changing
   anything). `uv tool install wexport` stays the technical path for development. This is §7 M5.
10. **Missing dependencies abort at startup**, exit `5`, with an English message naming the binary and
    the install command per platform (§2). No degraded mode. `tesseract` is demanded only when the key
    is an image.

No decision is open.

## 12. Later, deliberately not now

- **Rust rewrite of the core** (decrypt, `msgstore` adapter, `chat.db` writer, media orchestration)
  producing a native ~20 MB binary with the viewer assets embedded — decided as the eventual
  direction, explicitly **after v1**, not before. What it would buy: no Python runtime, millisecond
  startup, no interpreter inside the one-file binary, and freedom from the pinned upstream ref. What
  it costs: reimplementing the adapter that breaks whenever WhatsApp renames a column, which is the
  reason v1 reuses `wa-crypt-tools` at all. Nothing in v1 should be written in a way that only makes
  sense in Python — the pipeline stages already have file-based contracts, which is what makes the
  rewrite possible later.
- ffmpeg/tesseract bundling: deferred with it. Both single-binary builds still expect them on the
  host and degrade with a message when absent.
