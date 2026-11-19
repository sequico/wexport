# wexport 2.0 — plan

Four things, and nothing else:

1. the interface stays the **command line**;
2. **iPhone** backups join Android's;
3. the tool ships for **macOS, Linux and Windows**;
4. **`ffmpeg` and `ffprobe` go away**: the media work happens in-process, in Rust and in two small
   C libraries linked in.

The archive format does not change: an archive built from either phone is the same kind of folder, and
a 1.0 archive opens in 2.0 and the other way round. `docs/implementation-plan.md` remains the contract
for the pipeline and the archive; this document adds a second data source, a third platform and a new
media implementation.

**Nothing is removed for Android.** Every Android behaviour stays exactly as it is — `crypt15`, the
incrementals, the 64-digit key, the same flags, the same archives. A user who only has Android cannot
tell 2.0 from 1.0 except by the version number and the fact that the tool no longer asks for `ffmpeg`.

---

## 1. Scope

| | 1.0 (shipped) | 2.0 |
|---|---|---|
| Interface | CLI | CLI: the same flags plus `-j`, `-q`, `-d`, `-n`, `-c` (§6) |
| Data sources | Android `crypt15`, 64-digit key | the same, **plus** iPhone local backups |
| Platforms | Linux x86-64, Windows x86-64 | Linux x86-64, Windows x86-64, **macOS (universal)** |
| Media | external `ffmpeg` + `ffprobe` on `PATH` | **in-process**: image crates, `openh264`, `libopus` |
| Packaging | `.tar.gz`, `.zip` | `.tar.gz`, `.zip`, `.tar.gz` (macOS), each with a `.sha256` |
| Archive format | one folder per chat, `file://` viewer | unchanged |

The graphical application, the AppImage and the signed `.dmg` are **later work** (§10).

---

## 2. Distribution: one artifact per platform, built where it can be built

| Platform | Artifact | Built on | Notes |
|---|---|---|---|
| Linux x86-64 | `wexport-<v>-linux-x86_64.tar.gz` (holds `wexport`) | `ubuntu-24.04` | as in 1.0 |
| Windows x86-64 | `wexport-<v>-windows-x86_64.zip` (holds `wexport.exe`) | `ubuntu-24.04` + mingw | as in 1.0 |
| macOS universal | `wexport-<v>-macos-universal.tar.gz` (holds `wexport`) | `macos-14` | `aarch64-apple-darwin` + `x86_64-apple-darwin`, joined with `lipo` |

**macOS needs no Apple account, because the artifact is a `.tar.gz` extracted from a terminal.** The
quarantine attribute is set by the browser on the downloaded archive, and `tar` does not copy a
container's extended attributes onto the files it extracts; the binary is therefore not quarantined
and Gatekeeper does not stop it. (Double-clicking the archive in Finder is the exception: Archive
Utility propagates the flag. The documented install is `curl` + `tar`.) On Apple Silicon the linker
signs ad-hoc, which is what execution requires; `codesign -dv` shows it. Notarization is only needed
to survive a quarantined file, and this path has none.

With `ffmpeg` gone there is **no external dependency at all**: nothing to install on the machine that
builds an archive.

**How the three artifacts are assembled.** `scripts/release.sh` builds the two Linux-side binaries
locally (the Windows one with mingw) and takes the macOS pair from the workflow run, because macOS
cannot be cross-built. The mechanism is named here so it is not improvised on the day:

```sh
gh run download <run-id> -n dist -D dist-macos   # the macOS job uploads dist/ as the artifact "dist"
./scripts/release.sh --stage dist-macos          # stages all three, verifies, writes the .sha256 files
```

The script checks that each archive holds the binary it claims and that the inner binary hashes match
what was built, and **refuses to produce a release without the macOS archive**, the way it already
refuses without the Windows one. The run it stages from must be the one on the commit being tagged
(`gh run list --commit <sha>`), not merely a green run.

---

## 3. iPhone: what can be opened

A local backup made by Finder or iTunes is a folder of hashed files plus a `Manifest.db` mapping each
file to a domain and a path. WhatsApp's data lives in its app group
(`group.net.whatsapp.WhatsApp.shared`): `ChatStorage.sqlite` (Core Data) and the media under
`Message/Media/…`. The schema has nothing in common with Android's: `ZWACHATSESSION`, `ZWAMESSAGE`,
`ZWAMEDIAITEM`, `ZWAGROUPMEMBER`.

| How the data is held | 2.0 |
|---|---|
| Local backup, **not encrypted** (Finder/iTunes, "Encrypt local backup" off) | **opens** — no key involved |
| An already-extracted app container or `ChatStorage.sqlite` | **opens** |
| Local backup **encrypted** with the Apple backup password | not in 2.0: needs Apple's keybag (§10.3) |
| **iCloud** backup | never offline. Out. |
| WhatsApp's 64-digit key / passkey | irrelevant here: on iPhone it protects the **cloud** copy |

The public message becomes: *Android needs the 64-digit key; an iPhone needs a local backup that is
not encrypted.*

**How the database is read.** `ChatStorage.sqlite` lives in the backup with its `-wal` and `-shm`
beside it, and the newest messages are in the WAL, not in the main file. The backup is extracted with
all three files together and opened read-only, so SQLite replays the log; a build that read only the
main file would silently lose the last days of every conversation. The fixture has a WAL too, and a
test asserts that a message written only to it is in the archive.

**Backups written before iOS 10** keep their files at `domain-relativePath` instead of a hashed
`fileID`; 2.0 does not read that layout and says so, naming what it found, rather than producing an
empty archive.

---

## 4. Architecture

```
discover  ──►  Source::Android(Inputs)  ──►  msgstore (existing)  ─┐
          └─►  Source::Ios(IosInputs)  ──►  ios      (new)         ├─►  Message ─► archive (unchanged)
                                                                   ┘
```

- **`discover`** detects the source instead of assuming `crypt15`: a copied phone folder, a
  `Manifest.db` backup directory, or a `ChatStorage.sqlite`. The Android path is unchanged behind it,
  and the startup check for `ffmpeg`/`ffprobe` disappears with them.
- **`src/ios.rs`** is the second schema adapter: `ChatStorage.sqlite` read-only, columns resolved by
  probing — names discovered, never guessed, the same rule as Android — producing the existing
  `Chat`/`ChatStat`/`Message`.
- **Media**: relative paths inside the backup; content-addressed placement, thumbnails and waveforms
  apply as they are (§5).
- **Time**: Core Data counts seconds since 2001-01-01; one conversion in `ios.rs`, pinned by tests on a
  winter and a summer date.
- **CLI**: flags unchanged. `-k` stays the Android key and is not asked for on an iPhone backup.

---

## 5. Media without `ffmpeg`

The insight that makes this small: **the phone already stores the poster.** Android `msgstore.db` has
`message_thumbnail(message_row_id, thumbnail BLOB)` — the preview WhatsApp shows in the chat list, kept
even for View Once media. iOS has `ZWAMEDIAITEM.ZTHUMBNAILLOCALPATH`, a path to the thumbnail file.
Copying it needs no decoder at all.

| Work | How | What it costs |
|---|---|---|
| image thumbnail (JPEG/PNG/WebP in → 320 px) | `image` for decode, its resize, `jpeg-encoder` (or WebP lossless) for the output | ~1.0–1.5 MB |
| video/audio duration and dimensions | an MP4 box parser and a small EBML reader, a few hundred lines; OGG for Opus | ~0.2 MB |
| video poster | **the phone's thumbnail** (Android BLOB, iOS file); nothing when absent, the archive shows the placeholder it already has | 0 |
| video poster when the phone stored none | `openh264` on the first keyframe — **H.264 only** (decided, §9) | ~1.5 MB |
| voice-note waveform | `opus` (libopus, BSD, royalty-free) + an OGG demuxer | ~0.6 MB |

**Total: roughly 2–4 MB per platform** — a figure to be measured at M5, not a promise — with no
subprocess and no `ffmpeg` build. Two details decide whether it stays that small:

- WebP is read with the pure-Rust `image-webp` decoder, not with `libwebp`; the archive's own
  thumbnails are written as JPEG, so an encoder is one small dependency, not a C one.
- `libopus` and `openh264` are C: each platform needs a C compiler for its target, which the build
  already has (`gcc-mingw-w64` for Windows, the Xcode command-line tools on the macOS runner). It is
  **not** an `ffmpeg` build for three platforms — it is `cc` doing what it already does for
  `rusqlite`'s bundled SQLite.

**What changes in behaviour, stated plainly**
- Posters come from the phone when it stored one: smaller and softer than the 480 px we generate
  today. When it stored none (older media, some forwarded videos), the first frame is decoded with
  `openh264`, so a video gets a poster either way.
- **A media file that is not in the copy can still show its thumbnail.** Android keeps
  `message_thumbnail` and iOS keeps the thumbnail file even for media the phone later deleted or never
  downloaded, so the archive shows the picture with the "not copied" mark the record already carries
  (`mi`), instead of an empty placeholder. With no file there is no source hash to name it by, so such
  a thumbnail is named by the SHA-256 of its own bytes — the same content-addressed rule, applied to
  the only content there is. One test pins it.
- A file `ffmpeg` would have salvaged (a broken container, an exotic codec) now fails: the existing
  per-file failure path reports it and the archive continues.
- WebP output disappears in favour of JPEG unless the `image` encoder is used; the viewer shows either.
- The derived files' **bytes** change; their paths do not (they are named by the source's SHA-256), so
  the chunk records and the golden chunk stay identical. The byte-identical rebuild test keeps its
  meaning, and 1.0 archives remain readable.

**`openh264` is in.** A video always gets a poster: the phone's thumbnail when that exists, the first
frame decoded with `openh264` when it does not. The licensing note is recorded here so nobody has to
rediscover it — most AVC patents have lapsed, what remains runs to about 2027 in the United States and
2030 for a single Brazilian patent, and Cisco's royalty payment covers Cisco's own prebuilt binaries,
not a decoder compiled into ours. That is a small exposure, the tool is GPL-3.0-or-later and free, and
the decision is taken: decode H.264.

**What else changes in the repository at M5**, because the specification currently says the opposite of
this plan: `docs/implementation-plan.md` §2 and §5 state that `ffmpeg` and `ffprobe` are required and
that the startup gate refuses to run without them. M5 amends those paragraphs, the README's
requirements, the `-h` requirements line, and deletes `discover::require_binaries`; the only optional
external tool left is `tesseract`, for a screenshot key, and it is already checked where the image is
read. Until M5 lands, the 1.0 statement is the true one — no document is edited ahead of the code that
makes it true.

---

## 6. New flags

Short flags only, one letter each, one flag per word — the rule the CLI has always followed.

| Flag | Means | Behaviour |
|---|---|---|
| `-j` | JSON | the chat list and the build summary go to stdout as JSON; errors stay on stderr and keep their exit codes |
| `-q` | quiet | nothing on success; errors still reach stderr and the exit code still carries the result |
| `-d` | diagnostics | what was probed and decided: the files found, the columns recognised, each media file placed, each one missing |
| `-n` | dry run | the same work up to the point of writing: what would be built, how many messages and media files, how many missing, the estimated bytes from the source sizes |
| `-c` | check | the source is verified: on Android the `crypt15` header and key type first, without the key, then — with the key, which it therefore needs — `PRAGMA integrity_check` on the decrypted database and the count of media present against media referenced; on an iPhone backup everything, since there is nothing to decrypt. It warns when an incremental is older than the full backup |

Rules that decide the awkward cases:

- `-n` changes a build, so it needs `-b`: without it it is an error, exactly like `-o`.
- `-c` needs no `-b`: it checks the source and exits. With `-b` it checks first and then builds.
- `-c` on an Android backup asks for the key exactly as a build does: the header can be read without
  it, the database cannot. On an iPhone backup it asks for nothing.
- `-j` and `-q` compose: quiet silences the human chatter, JSON still prints. `-j` and `-d` compose too.
- `-q` and `-d` say opposite things: together they are an error rather than one silently winning.
- Every new flag is in `-h`, in the usage page's flag table and in the examples, and the exit codes do
  not change (`-c` and `-n` return `0..5` as the same situations do today).

## 7. Testing

- **A synthetic iPhone fixture in the repository**, generated by a script next to the Android one: a
  `ChatStorage.sqlite` with chats, a group with senders, media rows, a call, a location, a vCard, a
  deleted and an edited message, inside a `Manifest.db` backup. Committed.
- **Unit tests on the adapter**: every mapped field, the 2001 epoch, group authors, the media path,
  missing media, a schema with a renamed column.
- **Unit tests on the new media path**: an image thumbnail at the right size, a poster taken from the
  stored thumbnail, a video with no thumbnail (placeholder), a waveform from an Opus fixture, a
  container whose duration cannot be read, a corrupt file (per-file failure, no abort).
- **End-to-end**: the iOS fixture builds archives that pass the same contract tests as Android, with
  its own golden chunk; the Android golden chunk stays untouched.
- **A real backup, outside the repository**, run by hand on both phones.
- **Per platform, the packaged artifact is checked**: extract it, `wexport -v`, one build on the
  fixture; on macOS also `lipo -info` and `codesign -dv`. Plus the point of the whole change: the
  build runs on a machine with **no `ffmpeg` on `PATH`**, asserted in CI by removing it before the
  test run.
- **The gate runs on macOS too**, not only the artifact check: the same `scripts/gate.sh` on the
  `macos-14` runner, at least on every push to `main`. The suite has filesystem assumptions that APFS
  treats differently — `temp_dir()` resolves through `/private`, names are compared case-insensitively,
  and the media tests read inodes — so a green Ubuntu tree is not evidence for a green Mac one. A
  failure there is fixed in the code, not excluded from the run.
- The gates do not change: `scripts/gate.sh` on a push, `scripts/gate-release.sh` before a tag, with
  the macOS leg added to both.

---

## 8. Milestones

| # | Work | Acceptance |
|---|---|---|
| M1 | **macOS artifact**: `macos-14` job, two targets + `lipo`, `release.sh` stages three artifacts | the macOS archive is extracted on a Mac and builds a chat |
| M2 | Source detection: `Source` enum, discovery for a backup directory and a bare `ChatStorage.sqlite` | every existing test passes; a fake iOS folder is recognised, a real Android one still builds |
| M3 | `src/ios.rs`: chats, messages, senders, the 2001 epoch | unit tests cover every field and a renamed column |
| M4 | iPhone media: paths in the app group, `ZTHUMBNAILLOCALPATH` posters, missing files | the iOS fixture builds an archive that passes the viewer contract |
| M5 | **Media without `ffmpeg`**: image path, container metadata, phone thumbnails on Android, `openh264` poster when the phone stored none, `libopus` waveform, the startup check removed | the whole suite passes on a machine with no `ffmpeg`; the Android golden chunk is unchanged; a video with and one without a stored thumbnail both get a poster |
| M6 | **Structured reporting**: `ui.rs` emits events (`found`, `chat_list`, `progress`, `warning`, `failure`, `finished`) and the CLI prints them, with the current output pinned by tests | the CLI's text and the archives are unchanged; this is also the GUI's prerequisite (§10.1) |
| M7 | **The five flags**: `-j`, `-q`, `-d`, `-n`, `-c`, on top of M6, with the interaction rules of §6 and `-h` | each flag has a test, the JSON is parsed by a test, `-c` reports a damaged database and a missing media file |
| M8 | Docs (usage page for an unencrypted iPhone backup, README box, `-h`, requirements, the flag table), `CHANGELOG.md` 2.0 entry, validation on real backups, 2.0 tag | the three-command path works on both phones, on all three platforms |

**After the tag, the shipped docs are the authority.** `-h`, `docs/usage.md` (its flag table, the
"what opens" table, the iPhone steps) and `README.md` describe 2.0; this plan stops being read, the way
the 1.0 specification stopped being the description of what happens on a command line. Nothing in this
document is copied into the shipped docs by hand: the tables are rebuilt there from the code and the
help output, so a drift shows as a difference, not as two prose texts to compare.

Each milestone is committed and pushed on its own; the release gate runs once, before the 2.0 tag.

---

## 9. Risks and open decisions

| Risk | Effect | Mitigation |
|---|---|---|
| **H.264 patent licensing for `openh264`** | most AVC patents have lapsed (US ~2027, one Brazilian to 2030); the pool still licences, and we would be the distributor | **decided: in.** The note stays in §5 so the reasoning is not lost; no further decision needed |
| Phone thumbnails are small or missing | softer posters where they exist; the decoder covers the rest | measured on a real backup in M5; `openh264` is the fallback |
| Container parsing is not `ffmpeg` | odd files unreadable | per-file failure, never an aborted build; fixtures for MP4, WebM, OGG |
| `libopus` is a C library | one more vendored dependency | BSD, royalty-free, built by `cc` in the crate; alternative is no waveform |
| No macOS runner or a broken universal build | no macOS artifact | the CI job exists from M1; `lipo -info` verified before publishing |
| Core Data schema differs between iOS versions | the adapter reads nothing, or the wrong column | probe the schema and fail naming the columns; a fixture per layout we can obtain |
| Media in more than one place (`Message/Media`, `Library/Media`) | missing media | resolve both, test both |
| No iPhone among the maintainers | untested code shipped | the real-backup run is a condition of the release |
| The one-line identity ("Android, 64-digit key") | documentation contradicts itself | changed once, in M8 |

---

## 10. Later, deliberately not now

1. **The graphical application.** A window over the same pipeline, the CLI shipped inside the same
   artifact. Prerequisite: `ui.rs` emits events instead of printing.
2. **Portable packaging for the GUI**: **AppImage** on Linux (current
   [`type2-runtime`](https://github.com/AppImage/type2-runtime), statically linked, so `libfuse2` is not
   required — proven in a container without it), a **signed, notarized `.dmg`** on macOS (an unsigned
   `.app` double-clicked from a download is quarantined; ~99 USD/year), a **portable `.exe`** on
   Windows. Tauri generates all of them from one configuration, one runner per system.
3. **Encrypted iPhone backups**: Apple's keybag (PBKDF2, class keys, AES-CBC per file).

## 11. Explicit non-goals for 2.0

No GUI, no AppImage, no `.deb`/`.rpm`/Flatpak/Snap, no MSI, no installer, no iCloud, no password or
passkey chains, no transcoding, no network. **No deprecation of anything Android**: the `crypt15` path,
its incrementals and its key are the product as much as the iPhone path is; 2.0 adds beside them.
