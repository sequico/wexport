# Implementation plan

Status: v2.0.0 implemented. This document remains the **specification** — what the product must be;
`README.md` says what is done and what is still open. Two phones (Android `crypt15` with the 64-digit
key, iPhone local backups), three platforms, and media made in-process.

---

## 1. Scope

**In scope**

| | |
|---|---|
| Platform | Android (WhatsApp and WhatsApp Business) and iPhone (WhatsApp) |
| Backup format | Android `msgstore.db.crypt15` + `msgstore-increment-*.db.crypt15`, plus the rolling `msgstore-YYYY-MM-DD.1.db.crypt15` copies when present; iPhone Finder/iTunes backup (unencrypted) or an extracted `ChatStorage.sqlite` |
| Key | Android: 32-byte root key — 64 hex digits typed in, a `.png` screenshot, or an `encrypted_backup.key` file (root only). iPhone: none |
| Input | A folder the user copied: Android (`Databases/`, `Backups/`, `Media/`) or the iPhone device folder |
| Output | one **self-contained archive per chat**, `<out>/<date>-<slug>/` — HTML + classic-script data chunks + media, opened by **double-clicking `index.html`**, with no server and no wexport installed |
| Platforms | Linux x86-64, Windows x86-64, macOS universal (arm64 + x86-64) |

**Out of scope** (each one is a separate product decision, not a later ticket)

- iOS backups encrypted with the Apple backup password: Apple's keybag, not recoverable offline. The
  tool recognises them and says so.
- iCloud backups: never a folder on the computer.
- Google Drive download (`mcrypt1` objects, `oauth_token`).
- Android backups made with a password or a passkey: the key is held by WhatsApp's HSM, not
  recoverable offline. The tool recognises them and says so; it cannot open them.
- Legacy `crypt12` / `crypt14` (key file lives in app-private storage, needs root).
- Re-encrypting, writing back to the phone, or merging two accounts.
- A GUI, a mobile app, or a Docker image.

---

## 2. Pipeline

Five stages, each a module with a narrow contract so it can be tested alone.

```
input dir (+ Android key)
      │
      ▼
[1] discover    find the files, tell the two phones apart, read the crypt15 header when it is Android,
      │         decide whether this is openable at all
      │         → Source::Android(Inputs{…}) | Source::Ios(IosInputs{…})
      ▼
[2] open        Android: crypt15 → msgstore.db in a temporary workspace (+ incrementals merged).
      │         iPhone: ChatStorage.sqlite + -wal + -shm copied to the workspace, log replayed.
      │         → one SQLite file, schema untouched
      ▼
[3] read        the source schema (WhatsApp's Android tables, or Core Data on iOS) → our own model
      │         → Chat / ChatStat / Message{kind, text, media, reactions, payload} in memory
      ▼
[4] media       DB paths → files on disk (through the copied folder, or the backup manifest);
      │         derivatives (thumbnails, posters, waveform peaks) made in-process
      │         → <out>/<chat>/media/…, rows filled, missing files flagged
      ▼
[5] build       per chat: the in-memory records → index.html + app.js + data/*.js chunks + media
      │         → <out>/<date>-<slug>/ , a folder that opens over file://
      │         → <out>/index.html linking the chats (never inside a chat folder)
```

Stage contracts:

- **discover** never decrypts. It reads the protobuf header of an Android container, and the manifest
  of an iPhone backup, and it is the stage that produces every "your backup cannot be opened, and here
  is why" message. It never guesses.
- **open** produces exactly the database the phone wrote — no rewriting, no schema change — or fails
  with a message naming the cause. The workspace is owner-only and removed when the run ends.
- **read** is the only module that knows a phone's column names (`msgstore` for Android, `ios` for
  iPhone). Both produce the same model, so everything downstream is one pipeline.
- **media** is idempotent: a second run skips what exists, reports what is missing, never
  overwrites.
- **build** never mutates the archive; it only reads. Each chat folder is a complete archive: it works
  when copied alone, zipped, or sent to someone who has a browser and nothing else. There is no server
  anywhere in the product: `file://` is the only way an archive is opened.

### CLI surface

```
wexport [FOLDER] [-k KEY] [-l [N] [ORDER]] [-r] [-b CODES] [-o DIR] [-t ZONE]
                 [-j] [-q] [-d] [-n] [-c]
```

One command, one argument. It finds the backup wherever the copy put it (`Databases/`, `WhatsApp/`,
`Android/media/com.whatsapp/WhatsApp`, a folder holding `msgstore.db.crypt15`, the file itself, a
device folder with `Manifest.db`, or a folder holding `ChatStorage.sqlite`), looks for the key beside
an Android backup (`key.png`, `key.txt`, `encrypted_backup.key`), and otherwise asks for it once. An
iPhone backup needs no key, and `-k` on one is an error rather than a value silently ignored. Then it
prints the chats numbered, by newest event first:

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
Every rendered time uses this machine's zone unless `-t ZONE` names one (`Europe/Rome`), and the
manifest records the name either way.

Five flags report instead of changing the output: `-j` (JSON lines: `found`, `chat_list`, `progress`,
`diagnostic`, `warning`, `failure`, `finished`), `-q` (silence, with results still printed), `-d`
(diagnostics: the files found, the columns recognised, each media file placed or missing), `-n` (a
dry run that needs `-b` and writes nothing) and `-c` (check the source and stop, or check and then
build).

Media that the user did not copy is **not** an error: it is the normal state of a partial archive,
reported in the build summary and visible in the archive as a placeholder — with the phone's own
preview when it kept one. Only a derivative that was attempted and failed (a corrupt image, an
unreadable video) produces exit `4`.

Exit codes: `0` ok, `1` unreadable input or a question that could not be asked without a terminal,
`2` unusable key, `3` wrong backup type (crypt14 / password / passkey / encrypted iPhone), `4` build
incomplete or a check that found the source incomplete (the archive is written and usable), `5` a
missing external tool — only `tesseract`, for a key given as a screenshot.

### Dependency gate

No external tool is required to run wexport: the media work is in-process. `tesseract` is the one
optional tool, needed only when the key is a screenshot; it is checked at that moment with the install
command for the platform, so a user who types the 64 digits never needs it.

---

## 3. Repository layout

Target tree. Directories appear with their first real file, never as empty placeholders.

```
wexport/
├── README.md                    # the product: what it does, how to run it
├── LICENSE                      # GPL-3.0-or-later full text
├── THIRD-PARTY.md               # the dependencies and compiled-in libraries, with their notices
├── Cargo.toml                   # one crate, one binary, viewer assets embedded
├── Cargo.lock
├── .gitignore
├── docs/
│   ├── implementation-plan.md   # this file
│   ├── backup-formats.md        # the containers and schemas of both phones
│   ├── usage.md
│   ├── plan-2.0.md              # the 2.0 working plan, absorbed at the tag
│   └── figures/                 # the phone screens usage.md shows
├── .github/
│   ├── workflows/gate.yml       # the push gate on every pull request and push to main
│   └── ISSUE_TEMPLATE/, pull_request_template.md
├── .opencode/skills/            # agent skills for working in this repo
│   ├── wexport-implement/       # procedure + invariants + fixtures recipe
│   └── offline-html-archive/    # file:// archive technique
├── src/
│   ├── main.rs                  # the binary: one command, exit codes
│   ├── lib.rs                   # the module map
│   ├── crypt15.rs               # varint + protobuf header, AES-256-GCM, zlib/ZIP
│   ├── keys.rs                  # hex | png (tesseract) | key file → 32 bytes
│   ├── discover.rs              # which phone, which files, the key-type gate
│   ├── error.rs                 # the message a person reads + the exit code a script reads
│   ├── source.rs                # the one trait both phones implement
│   ├── msgstore.rs              # the Android schema adapter (WhatsApp 2.26 names)
│   ├── ios.rs                   # the iPhone schema adapter (Core Data names, manifest lookup)
│   ├── merge.rs                 # incremental replay into the full backup
│   ├── container.rs             # MP4, Matroska and Ogg readers: duration, size, the first frame
│   ├── media.rs                 # path resolution, content addressing, derivatives
│   ├── chunks.rs                # records → classic-script chunks, escaping
│   ├── archive.rs               # one folder per chat, manifest, README, index, prune
│   ├── progress.rs              # the one-line media progress bar
│   ├── report.rs                # the events, the human rendering and the JSON lines
│   ├── ui.rs                    # flags, the chat codes, the run
│   └── assets/                  # the viewer: index.html, app.js, app.css (classic scripts)
└── tests/
    ├── fixture.rs               # end-to-end runs on the committed Android fixture
    ├── ios.rs                   # end-to-end runs on the committed iPhone backup
    ├── real.rs                  # optional run on a real backup, via WEXPORT_REAL_BACKUP
    └── fixtures/
        ├── synthetic/           # schema, data, generator and the crypt15 files it commits
        │   └── ios/             # the Finder backup generator and the backup it commits
        └── real/                # gitignored, supplied via WEXPORT_REAL_BACKUP
```

The Android fixture lives in `tests/fixtures/synthetic/phone` and is built by
`tests/fixtures/synthetic/build.py`; the iPhone one lives in `tests/fixtures/synthetic/ios/backup`
and is built by `tests/fixtures/synthetic/ios/build.py`. Neither contains real WhatsApp data.

Rules that follow from the layout:

- `src/msgstore.rs` and `src/ios.rs` are the **only** places a phone's column or table name appears.
  Everything downstream works on our own `Chat` / `Message` / `Media` types.
- No module outside `src/ui.rs`, `src/main.rs` and `src/report.rs` reads the command line or prints to
  the terminal; `progress.rs` draws the bar the reporter asked for.
- Tests exercise module functions; only the integration suites run the command end to end. The CLI is
  covered through `ui::run(argv)` for exit codes.
- `src/assets/` holds the viewer as plain **classic** scripts — no modules (blocked over `file://`), no
  bundler, no node toolchain — embedded in the binary with `include_str!`.
- The only C and C++ code is what `openh264` and `opus` vendor; both are built by `cc`/`cmake` as part
  of the crate build, for every target the release ships.

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
  beside a video). The build never fills it: transcoding is off and not exposed.
- `nm` keeps the original filename for display: the file on disk is content-addressed, but a person
  reading the archive needs to see `IMG-20261101-WA0003.jpg`, not a SHA-256.
- **Two times per message, and they mean different things.** `t` is the integer millisecond since
  epoch (UTC) — the sortable truth, what the archive orders by. `s` is the rendered wall-clock with an
  explicit offset (`2026-11-17 14:32:05 +01:00`), **frozen at build time**. The viewer never
  recomputes a local time from `t`: timezone rules change, sometimes retroactively, and an archive
  opened in ten years must show the hour it actually happened, not the hour a future tzdata would
  derive.
- The manifest records the timezone name the strings were rendered in — the machine's zone, e.g.
  `Europe/Rome`, or the one `-t ZONE` names — and the tzdata version used, so a later rebuild is
  comparable and a reader knows what the offsets refer to.
- Displayed time is always the full date and time with its offset, plus a day separator carrying the
  full date and weekday; the tooltip adds the epoch and the UTC form. No "today", no "yesterday", no
  locale-dependent formatting, no rewriting of history when the OS locale changes.

---

## 5. Media handling

| Kind | What the build produces | Notes |
|---|---|---|
| image | 320 px JPEG thumbnail + original | EXIF rotated, never upscaled; original kept as-is |
| video, gif | poster JPEG + duration + dimensions, original MP4 | poster is the preview the phone stored, or the first frame decoded with `openh264`; no transcoding |
| audio (voice note) | peak array (500 ints) as JSON, original OGG/Opus | `libopus` decodes to build the waveform; no re-encode |
| sticker | original WebP/PNG + size class | no thumbnail |
| document | mime icon + filename + size | no preview |

- Source of every byte: the user's `Media/` folder (Android) or the files the backup manifest points at
  (iPhone). Files are stored once **per site**, content-addressed, at `media/<first2>/<sha256><ext>`
  inside the chat folder: inside one chat the same photo sent five times is one file. Across per-chat
  archives the same file is duplicated on purpose — a chat folder has to stand alone when it is copied
  or zipped, and dedupe across archives would break that promise.
- The chat a file belongs to never appears in its path: placement is by content, so renaming a chat,
  or the same file arriving in two chats, never orphans anything.
- The build hardlinks when the filesystem allows it and copies otherwise: same bytes either way.
- No transcoding: the build touches no video and no audio beyond reading it.
- **In-process, no external tool.** Images go through the `image` crate, container metadata through a
  small MP4/Matroska reader, a video poster through `openh264` (H.264 only), a waveform through
  `libopus`. A video whose codec has no decoder keeps the placeholder instead of failing the build;
  a corrupt file fails alone, on the per-file path.
- **The phone's own preview is the poster.** Android keeps `message_thumbnail`, iOS keeps a thumbnail
  file even for media the phone deleted: a media row whose file is gone still shows its picture, with
  the "not copied" mark. The preview is used when its bytes are a picture a browser can show (JPEG,
  PNG or WebP); anything else is left out rather than written under an extension that lies.
- The thumbnail and poster a derivative produces are named by the hash of their own bytes when the
  source file is gone, and by the source's hash otherwise: the content-addressed rule, applied to
  whatever content exists.

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
`src/media.rs`, `src/container.rs`, parallel media work with a progress bar, idempotency.
**Acceptance:** every `media` row is either present with a derivative or `missing=1` with a reason;
second run copies nothing; a 20 GB `Media/` folder finishes without loading files into memory; the
build runs on a machine with **no `ffmpeg` on `PATH`**, which is asserted in CI by removing it.

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
(Linux, Windows and macOS universal). README and `docs/usage.md` checked against a fresh account with
nothing but the binary.
**Acceptance:** a non-technical person follows `docs/usage.md` end to end on a machine with no
toolchain, using only the single binary; the Windows build starts under Defender without a warning, and
the Linux build is statically linked against glibc-free targets or runs on a distro older than the one
it was compiled on.

### 2.0 — two phones, three platforms, media in-process

| Step | Work | Acceptance |
|---|---|---|
| M1 | **macOS artifact**: `macos-15` job, two Apple targets + `lipo`, `release.sh --stage` | the archive is extracted and run on the macOS runner; all three archives hold the binary, `LICENSE` and `THIRD-PARTY.md` |
| M2 | Source detection: `Source` enum, a backup directory, a bare `ChatStorage.sqlite` | an encrypted iPhone backup is refused by name (exit `3`); several device folders are listed, never guessed |
| M3 | `src/ios.rs`: chats, messages, senders, the Core Data epoch, the manifest lookup | unit tests cover every field and a renamed column |
| M4 | iPhone media: paths in the app group, the stored thumbnails, missing files | the iOS fixture builds an archive that passes the same contract, and a thumbnail whose shape differs is fitted, not stretched |
| M5 | **Media in-process**: image path, container metadata, phone thumbnails, `openh264` posters, `libopus` waveforms, no startup check | the suite passes with no `ffmpeg` on `PATH`; a video with and one without a stored thumbnail both get a poster; a media file that is not in the copy shows its thumbnail with the `mi` mark |
| M6 | **Structured reporting**: `ui.rs` emits events, `report.rs` renders them, `progress.rs` draws the bar it is asked for | the CLI's text and the archives are unchanged; no module prints on its own |
| M7 | **The five flags**: `-j`, `-q`, `-d`, `-n`, `-c` | each flag has a test, the JSON is parsed by a test, `-q` with `-d` is refused, `-q` and `-j` leave the bar silent, `-c` reports a damaged database and a missing media file |
| M8 | Documentation, `CHANGELOG.md`, the 2.0 tag | download, extract and run works on both phones, on all three platforms |

---

### Release procedure

A release always ships **three** platforms, and nothing is built by hand:

```sh
gh run download <run-id> -n dist-macos -D dist-macos   # the macOS job's pair, from the commit being tagged
./scripts/gate-release.sh     # the push gate, plus mutations, fuzzing, ThreadSanitizer, every build
./scripts/release.sh --stage dist-macos   # dist/: three archives, each with its .sha256
git tag -a v<version> -m "wexport <version>" && git push origin v<version>
gh release create v<version> dist/* --title "wexport <version>" --notes-file <notes.md> --target <commit>
```

`scripts/release.sh` refuses to run without the Windows target, the mingw C and C++ compilers, the
`tar`/`zip` tools and — with `--stage` — the macOS archive, because a release that ships one platform
is not this product. It builds `--release --locked` for Linux and Windows, checks that each file is
what it claims (`ELF 64-bit`, `PE32+ executable`) and that the Linux binary reports the version in
`Cargo.toml`, then writes `dist/wexport-<version>-linux-x86_64.tar.gz`,
`dist/wexport-<version>-windows-x86_64.zip` and the staged
`dist/wexport-<version>-macos-universal.tar.gz`, each holding the binary with `LICENSE` and
`THIRD-PARTY.md`, each with its `.sha256`. It extracts all three and compares the hash of the binary
inside with the one it built or was given, so a package that holds the wrong file cannot be published.
The version comes from `Cargo.toml`; `CHANGELOG.md` loses its `Unreleased` heading in the same change,
and the release notes name the commit the binaries were built from.

The Windows binary is cross-built on Linux and **run on `windows-2022`** in CI, because a cross-build
proves that it links, not that it starts.

## 8. Testing strategy

- **Unit**: pure functions — key parsing, header parsing, path resolution, payload decoding, chunk
  splitting.
- **Synthetic fixtures, committed**: an Android `crypt15` generated from a hand-built `msgstore.db`
  with one message per `kind` value plus a two-file incremental pair, and an iPhone Finder backup
  generated with its `Manifest.db`, its media and a live `-wal`. No personal data in git, ever.
- **Integration**: real backup supplied out of band via `WEXPORT_REAL_BACKUP`; skipped when unset.
  The test states why it is skipped, so the skip is not mistaken for coverage.
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
| `rusqlite` (bundled SQLite) | read the msgstore and the iPhone store, merge incrementals, read `wa.db` | MIT |
| `serde_json` | records, the manifest and the `-j` events | Apache-2.0 / MIT |
| `jiff` | frozen timestamps with an explicit offset, and the zone database | Unlicense / MIT |
| `plist` | read `Manifest.plist` (the iPhone backup's encryption flag) | MIT / Apache-2.0 |
| `image` (JPEG, PNG, WebP) | decode and resize pictures, write JPEG thumbnails | MIT / Apache-2.0 |
| `openh264` (vendored C++) | decode the first H.264 frame when the phone stored no preview | BSD-2-Clause |
| `opus` (vendored C) | decode voice notes for the waveform | BSD-3-Clause |
| `libc` (Unix only) | the SIGPIPE disposition, so a listing piped into `head` ends quietly | MIT / Apache-2.0 |
| `tesseract` (external, optional) | read the 64 digits from a screenshot | Apache-2.0 |

**Decision (owner, taken):** wexport is **GPL-3.0-or-later**. The crypt15 reader — varint + protobuf
header, AES-256-GCM, the nested-HMAC key derivation, the zlib/ZIP payloads — is a port of
`wa-crypt-tools` (GPL-3.0), and a test pins the derivation to the value that reference implementation
computes, so a silent change cannot slip through.

A trade that comes with porting instead of linking: the reader is ours, which means a change in the
container format is our bug to find, not an upstream release to wait for. The header fixture test and
the real-backup run in `tests/real.rs` are what keep that honest. A screenshot key needs the
`tesseract` binary on the host; the CLI degrades to typed digits, with a message, when it is missing.

**Decision (owner, taken):** no transcoding. The build reads metadata only, keeps the original bytes,
and leaves the optional re-encoded track for the day a browser forces the issue.

**Every archive carries its licences.** `LICENSE` (GPL-3.0-or-later) and `THIRD-PARTY.md`, kept in step
with `Cargo.lock`, travel beside the binary; the viewer that goes inside each chat folder carries the
same notice in a comment at the top of `index.html`.

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
| Poll options and votes are not decoded (a poll renders as its question text) | a poll loses its choices | decode the add-on payload from a backup that contains polls; the add-on row is already read for reactions |
| A video in a codec with no decoder (HEVC and others) and no stored preview | no poster for that video | the phone's preview covers nearly all videos; the gap is counted and named by `-d`, never a failed build |
| The iPhone `-wal` is not copied beside the database | the last days of every chat missing | the extractor takes all three files; a test asserts a WAL-only message is in the archive |
| The Core Data schema differs between iOS releases | the adapter reads nothing, or the wrong column | probe the schema and fail naming the columns; a fixture for each layout that matters |
| The iPhone backup is encrypted with a password | nothing to open | exit `3`, naming what was found |
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
9. **Packaging: one release binary per platform from `cargo build --release`**, **Linux, Windows and
   macOS universal**; no AppImage, no `.dmg` (§7 M5).
10. **`tesseract` is the only external tool**, demanded only when the key is an image; everything else
   the tool needs is compiled in, so a missing binary cannot stop a build (§2).
11. **The archive opens from the filesystem**, and there is no server in the product: no `serve`
    command, no HTTP fallback. Double-clicking `index.html` must work with no wexport installed and no
    browser flag. That is why the data is emitted as classic scripts and why there is no SQLite-WASM
    and no HTTP-only format (§6).
12. **Timestamps are frozen at build.** Every message carries both the epoch milliseconds and a
    rendered ISO-8601 string with an explicit offset; the viewer never derives a local time from the
    epoch, so reopening the archive in ten years, on a machine in another timezone, shows the time it
    actually happened (§4, §6). The zone is the machine's or the one `-t ZONE` names, and the manifest
    records which it was.
13. **iPhone in addition, never instead.** The Android path — `crypt15`, its incrementals and the
    64-digit key — stays exactly as it is; the iPhone is a second source behind the same pipeline, and
    an iPhone backup needs no key, so `-k` on one is an error (§2).
14. **Media in-process, with a codec boundary stated.** Images through `image`, metadata through our
    own container readers, the H.264 poster through `openh264`, the waveform through `libopus`. A
    codec without a decoder leaves the poster placeholder, counted and named, and never fails the
    build (§5).
15. **Five reporting flags, short and composable**: `-j`, `-q`, `-d`, `-n`, `-c` (§2). `-j` is a
    contract: the event names and their fields are pinned by a test and documented, and a field is
    added rather than renamed.

No decision is open.

## 12. Later, deliberately not now

- **The graphical application, the AppImage and the signed `.dmg`**: a window over the same pipeline,
  portable packaging per system. [`plan-2.0.md`](plan-2.0.md) keeps the shape; the CLI is the product
  until then.
- **Encrypted iPhone backups**: Apple's keybag (PBKDF2, class keys, AES-CBC per file).
- **HEVC decoding**, for an iPhone video whose thumbnail the phone never stored.
- **Transcoding**: off and not exposed (§5); the record has room for an alternative track.
- **Polls and other add-on payloads**: a poll currently renders as its question text. Decode the
  add-on row from a backup that contains polls.
- **Windows behaviour** is proven on a Windows machine: CI cross-builds and inspects the executable,
  and the native run happens there as part of a release.

---

wexport is written and maintained by [Sequi Company](https://sequi.company) — `info@sequi.company`.
