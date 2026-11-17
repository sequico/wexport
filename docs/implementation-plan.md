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
| Output | `site/` — static HTML + JS + media + `chat.db`, opened through `wexport serve` |

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
[5] build       chat.db + assets + manifest → <out>/, which becomes the site
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
- **build** never mutates the archive; it only reads.
- **serve** is optional (a user with a static-host habit can open the folder any other way).

### CLI surface

```
wexport check <input-dir> [--key <hex|png|keyfile>]
wexport build <input-dir> --key <hex|png|keyfile> --out <site-dir>
              [--chats <jid,...>] [--jobs N] [--transcode {none,audio,video,all}] [--link]
wexport serve <site-dir> [--port 8765]
```

Three commands, `argparse` only. Exit codes: `0` ok, `1` unreadable input, `2` unusable key,
`3` wrong backup type (crypt14 / password / passkey), `4` build incomplete (some derivatives could
not be produced; the site is written and usable).

Media that the user did not copy is **not** an error: it is the normal state of a partial archive,
reported in the build summary and visible in the site as a placeholder. Only a derivative that was
attempted and failed (a corrupt image, an unreadable video) produces exit `4`.

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
│   │   ├── build.py             # copy assets, write manifest, hardlink/copy media
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
queries it in the browser through SQLite-WASM.

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

- Source of every byte: the user's `Media/` folder. Files are stored once, content-addressed, at
  `site/media/<first2>/<sha256><ext>`: the same photo sent in five chats is one file on disk, and
  dedupe falls out of the layout instead of being a maintenance chore.
- The chat grouping lives only in `chat.db` (`media.message_id` → `message.chat_id`), never in a path.
  Renaming a chat, or moving a file between chats, never touches the archive on disk.
- The build copies by default, or hardlinks with `--link` (same filesystem only).
- Transcoding is opt-in. Default build touches no video and no audio beyond metadata read.
- `ffmpeg`/`ffprobe` are external: probed at startup, and a missing binary downgrades to
  "no derivatives" with a warning instead of failing the build.

---

## 6. Viewer

- One page, three regions: chat list (left), timeline (centre), media drawer/gallery (right).
- **Virtualised timeline**: only the messages in and around the viewport exist in the DOM
  (target: ≤ 250 nodes regardless of chat length). This is the requirement that decides the
  architecture, so it is built first, not retrofitted.
- Data access through SQLite-WASM over HTTP (Range requests work; `file://` does not — Chrome
  blocks module and `fetch` from `file://`, and video cannot seek without Range).
- Renderers per `kind`, each a small function; unknown `payload` shapes fall back to a raw JSON
  disclosure rather than an exception.
- Search: FTS5 query against `chat.db`, results grouped by chat, jump to message.
- Jump-to-date, gallery per chat, keyboard navigation (`j`/`k`, `/` for search).
- No framework, no CDN, no bundler, no telemetry, no network calls at all.

---

## 7. Milestones

Each milestone ends with something runnable and a stated acceptance test.

### M0 — skeleton and preflight
`pyproject.toml`, CLI, `errors.py`, `keys.py`, `preflight.py`, header parsing, and the synthetic
`crypt15` fixture generator in `tests/fixtures/synthetic/` (M0 cannot be tested without one).
**Acceptance:** `wexport check <dir>` on the synthetic fixture and on a real backup prints format,
app version, key type, file list, total size. On `crypt14`, on a password backup and on a passkey
backup it exits `3` with a message that names the case and the way out. On a screenshot key it
either reads it or says the digits must be typed.

### M1 — decrypt, merge, read
`backup/decrypt.py`, `backup/merge.py`, `db/msgstore.py`, `db/slim.py`, `db/fts.py`.
**Acceptance:** `chat.db` row counts match the source tables; contacts resolved from
`Backups/wa.db` when present, numbers otherwise; an incremental set merges to the same message
count the phone shows; re-running is deterministic (same hashes, same order).

### M2 — media
`media/resolve.py`, `media/derive.py`, `--jobs` parallelism, idempotency.
**Acceptance:** every `media` row is either present with a derivative or `missing=1` with a reason;
second run copies nothing; a 20 GB `Media/` folder finishes without loading files into memory.

### M3 — viewer MVP
Virtualised timeline, image/video/audio/document renderers, chat list, `serve`.
**Acceptance:** a 100k-message chat opens in under a second and scrolls without jank; a 3 GB video
seeks correctly over `wexport serve`; nothing loads from the network.

### M4 — search, dates, gallery
FTS5 wiring, jump-to-date, per-chat gallery, keyboard navigation.
**Acceptance:** a phrase search over 300k messages returns in under 100 ms locally; jump-to-date
lands on the right message including for DST boundaries.

### M5 — error UX, packaging, docs
Every exit code has a test; `pyinstaller` one-file builds for Linux/macOS/Windows; README and
`docs/user-flow.md` checked against a fresh account with no Python installed.
**Acceptance:** a non-technical person follows `docs/user-flow.md` end to end on a machine with no
Python, using only the release binary.

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
| `ffmpeg`, `ffprobe` (external) | metadata, poster frames, optional transcode | LGPL/GPL builds vary |
| SQLite-WASM build (vendored asset) | viewer data access | public domain / Apache-2.0 (confirm at vendoring) |
| Python stdlib | sqlite3, http.server, argparse, hashlib | PSF |

**Decision (owner, taken):** wexport is **GPL-3.0-or-later** and reuses `wa-crypt-tools` instead of
reimplementing crypt15. The alternative — our own reader (protobuf header + AES-256-GCM + the
HMAC-based key derivation, ~200 lines with `cryptography`) — would allow MIT/Apache and would need a
test suite trusted more than the reference implementation; not worth a milestone. The full licence
text lands with M0; `README.md` and `pyproject.toml` carry the same SPDX identifier.

One caveat found while reviewing this plan: the OCR-of-the-screenshot feature and the `wagui` front
end sit in the upstream changelog under **"Unreleased"** — the released version is 0.2.0. Until they
ship, wexport pins a git ref or accepts typed digits only, and M0's acceptance test states which.
`--key <png>` is in the v1 CLI surface, so this is an input to M0, not to M4.

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
| Screenshot-key OCR lives on `wa-crypt-tools` main, not in 0.2.0 | `--key key.png` missing on the released version | pin the git ref, or ship typed digits first and add the image path when it ships |
| `site/` is a folder, not a single file | user copies it somewhere else and nothing opens | `site/` opens through `wexport serve`; relative paths only, and the user flow says so |

---

## 11. Open decisions (owner)

1. **One site for all chats (recommended) or one site per chat** for sharing a single conversation.
2. **UI language**: English only, Italian only, or both.
3. **Distribution**: `uvx wexport` for the technical user, `pyinstaller` binaries for everyone else,
   or both.
4. **Whether `wexport check` may talk to the network at all** (for a "new version available" hint).
   Recommendation: no, never.

Decided already, kept here so they are not reopened by accident: GPL-3.0-or-later reusing
`wa-crypt-tools` (§9), transcode opt-in (§5, §9).
