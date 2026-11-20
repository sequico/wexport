# wexport 2.0 — plan

Four things, and nothing else:

1. the interface stays the **command line**, with five new flags (§6);
2. **iPhone** backups join Android's;
3. the tool ships for **macOS, Linux and Windows**;
4. **`ffmpeg` and `ffprobe` go away**: the media work happens in-process — Rust, one C library
   (`libopus`) and one C++ library (`openh264`) compiled in.

The archive format does not change: an archive built from either phone is the same kind of folder, and
a 1.0 archive opens in 2.0 and the other way round. `docs/implementation-plan.md` remains the contract
for the pipeline and the archive; this document adds a second data source, a third platform and a new
media implementation.

**Nothing is removed for Android.** Every Android behaviour stays as it is — `crypt15`, the
incrementals, the 64-digit key, the same flags, the same archive layout. A user who only has Android
cannot tell 2.0 from 1.0 except by the version number, by `ffmpeg` no longer being required, and by
previews that are the ones the phone itself stored.

---

## 1. Scope

| | 1.0 (shipped) | 2.0 |
|---|---|---|
| Interface | CLI | CLI: the same flags plus `-j`, `-q`, `-d`, `-n`, `-c` (§6) |
| Data sources | Android `crypt15`, 64-digit key | the same, **plus** iPhone local backups |
| Platforms | Linux x86-64, Windows x86-64 | Linux x86-64, Windows x86-64, **macOS (universal)** |
| Media | external `ffmpeg` + `ffprobe` on `PATH` | **in-process**: image crates, `openh264`, `libopus` |
| Packaging | `.tar.gz` (Linux), `.zip` (Windows) | the same two, plus `.tar.gz` (macOS); each with a `.sha256` |
| Archive format | one folder per chat, `file://` viewer | unchanged |

The graphical application, the AppImage and the signed `.dmg` are **later work** (§10).

---

## 2. Distribution: one artifact per platform, built where it can be built

| Platform | Artifact | Built on | Notes |
|---|---|---|---|
| Linux x86-64 | `wexport-<v>-linux-x86_64.tar.gz` (holds `wexport`) | `ubuntu-24.04` | as in 1.0 |
| Windows x86-64 | `wexport-<v>-windows-x86_64.zip` (holds `wexport.exe`) | `ubuntu-24.04` + mingw | as in 1.0 |
| macOS universal | `wexport-<v>-macos-universal.tar.gz` (holds `wexport`) | `macos-14` | `aarch64-apple-darwin` + `x86_64-apple-darwin` installed and built, joined with `lipo` |

**macOS needs no Apple Developer account and no notarization, because the artifact is a `.tar.gz`
extracted from a terminal.** The quarantine attribute is set by the browser on the downloaded archive,
and `tar` does not copy a container's extended attributes onto the files it extracts, so the binary is
not quarantined and Gatekeeper does not stop it. (Double-clicking the archive in Finder is the
exception: Archive Utility propagates the flag. The documented install is `curl` + `tar`.) The linker's
ad-hoc signature is what execution requires on Apple Silicon, and nothing else is.

With `ffmpeg` gone, running wexport needs nothing installed; `tesseract` stays optional, and only for a
key handed over as a screenshot.

**How the three artifacts are assembled.** `scripts/release.sh` builds the two Linux-side binaries
locally (the Windows one with mingw) and takes the macOS pair from the workflow run, because macOS
cannot be cross-built. The procedure is two commands:

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
| Local backup **encrypted** with the Apple backup password | not in 2.0: `Manifest.plist` says `IsEncrypted`, and it is refused as a wrong backup type (exit `3`), naming what it found; it needs Apple's keybag (§10.3) |
| **iCloud** backup | never offline. Out. |
| WhatsApp's 64-digit key / passkey | irrelevant here: on iPhone it protects the **cloud** copy |

The public message becomes: *Android needs the 64-digit key; an iPhone needs a local backup that is
not encrypted.*

**How the database is read.** `ChatStorage.sqlite` lives in the backup with its `-wal` and `-shm`
beside it, and the newest messages are in the WAL, not in the main file. All three are copied into the
build's writable workspace — replaying a log needs room for the `-shm` — and the database is opened
read-only there, so SQLite replays the log; a build that read only the main file would silently lose
the last days of every conversation. The fixture has a WAL too, and a test asserts that a message
written only to it is in the archive.

**Backups written before iOS 10** keep their files at `domain-relativePath` instead of a hashed
`fileID`; 2.0 does not read that layout and says so, naming what it found, rather than producing an
empty archive.

**Several device folders.** A path that holds more than one backup (`MobileSync/Backup`, for instance)
is not guessed: each folder is a device UDID, and the `Info.plist` inside names the phone and the date,
so discovery lists them and asks for one instead of picking the alphabetical winner; with no terminal to
ask on, it exits `1` naming them.

---

## 4. Architecture

```
discover  ──►  Source::Android(Inputs)  ──►  msgstore (existing)  ─┐
          └─►  Source::Ios(IosInputs)  ──►  ios      (new)         ├─►  Message ─► archive (unchanged)
                                                                   ┘
```

- **`discover`** detects the source instead of assuming `crypt15`: a copied phone folder, a
  `Manifest.db` backup directory, or a `ChatStorage.sqlite`. The Android path is unchanged behind it;
  the startup check for `ffmpeg`/`ffprobe` disappears with M5.
- **`src/ios.rs`** is the second schema adapter: `ChatStorage.sqlite` read-only, columns resolved by
  probing — names discovered, never guessed, the same rule as Android — producing the existing
  `Chat`/`ChatStat`/`Message`.
- **Media**: relative paths inside the backup; content-addressed placement, thumbnails and waveforms
  apply as they are (§5).
- **Time**: Core Data counts seconds since 2001-01-01; one conversion in `ios.rs`, pinned by tests on a
  winter and a summer date.
- **CLI**: every existing flag keeps its meaning; the five new ones are in §6. `-k` stays the Android
  key and is not asked for on an iPhone backup.

---

## 5. Media without `ffmpeg`

The insight that makes this small: **the phone already stores the poster.** Android `msgstore.db` has
`message_thumbnail(message_row_id, thumbnail BLOB)` — the preview WhatsApp shows in the chat list, kept
even for View Once media. iOS has `ZWAMEDIAITEM.ZTHUMBNAILLOCALPATH`, a path to the thumbnail file.
Copying it needs no decoder at all.

| Work | How | Weight |
|---|---|---|
| image thumbnail (JPEG/PNG/WebP in → 320 px JPEG) | `image` decodes, applies the EXIF orientation, resizes to at most 320 px wide without upscaling, and writes JPEG; WebP input through its pure-Rust `image-webp` feature | ~1.0–1.5 MB |
| video/audio duration and dimensions | an MP4 box reader (duration, dimensions, the `avcC` header) and an EBML reader; OGG for Opus | ~0.2 MB |
| video poster | **the phone's thumbnail** (Android BLOB, iOS file), copied unchanged, its extension taken from its own bytes | 0 |
| video poster when the phone stored none | the first frame decoded with `openh264`: SPS/PPS out of `avcC`, the length-prefixed NAL units of the first sample, one frame — **H.264 only** | ~1.5 MB |
| voice-note waveform | `opus` (libopus, BSD) + an OGG demuxer | ~0.6 MB |

**Total: roughly 2–4 MB per platform**, to be measured at M5, with no subprocess and no `ffmpeg` build.
Two details decide whether it stays that small:

- WebP is read with the pure-Rust `image-webp` decoder, not with `libwebp`.
- `openh264` is C++ and `libopus` is C, so each platform needs both compilers for its target, and the
  build already has them (`g++-mingw-w64` for Windows, the Xcode command-line tools on the macOS
  runner). A cross-build sets `CXX` to the target's C++ compiler, which is what the `openh264-sys2`
  build script reads. This is **not** an `ffmpeg` build for three platforms — it is `cc` doing what it
  already does for `rusqlite`'s bundled SQLite.

**What changes in behaviour, stated plainly**
- Posters come from the phone when it stored one: smaller and softer than the 480 px poster 1.0
  generates, because they are the preview WhatsApp itself shows in the chat list.
- When the phone stored none, the first frame is decoded. That works for H.264; a video in HEVC or
  another codec keeps the placeholder, because `openh264` decodes H.264 only.
- **A video with no poster is not a media failure.** The file is copied and the record is complete; the
  summary counts it ("no poster") and `-d` names each one, so nothing is silent and the exit code stays
  `0`.
- **A media file that is not in the copy can still show its thumbnail.** Android keeps
  `message_thumbnail` and iOS keeps the thumbnail file even for media the phone later deleted or never
  downloaded, so the archive shows the picture with the "not copied" mark the record already carries
  (`mi`), instead of an empty placeholder. With no file there is no source hash to name it by, so such
  a thumbnail is named by the SHA-256 of its own bytes — the same content-addressed rule, applied to
  the only content there is. One test pins it.
- A file `ffmpeg` would have salvaged — a broken container, a truncated frame — now fails on the
  existing per-file failure path, which reports it and lets the archive continue.
- **Image thumbnails become JPEG** (`deriv/<sha>.jpg`) where 1.0 writes WebP (`deriv/<sha>.webp`): one
  encoder instead of two, and smaller than lossless WebP. Video posters stay `.jpg` unless the phone
  stored the preview as PNG. The record shape does not change — `th` is still a path — so **the Android
  golden chunk moves once, at M5, and the diff is read and committed with it**. An archive of either
  version opens in the other, because a viewer only follows `th`.

**`openh264` is in.** A video gets a poster whenever the phone stored one or the codec is H.264: the
thumbnail when it exists, the first frame otherwise. The crate vendors the BSD-licensed library and
compiles it through `cc`, as the `opus` crate does for libopus.

**What else changes in the repository at M5**, because the specification currently says the opposite of
this plan: `docs/implementation-plan.md` §2 and §5 state that `ffmpeg` and `ffprobe` are required and
that the startup gate refuses to run without them. M5 amends those paragraphs, the requirements in the
README and the usage page, the `-h` requirements line, and deletes `discover::require_binaries`; the
only optional external tool left is `tesseract`, for a screenshot key, and it is already checked where
the image is read. Until M5 lands, the 1.0 statement is the true one — no document is edited ahead of
the code that makes it true.

---

## 6. New flags

Short flags only, one letter each, one flag per word — the rule the CLI has always followed.

| Flag | Means | Behaviour |
|---|---|---|
| `-j` | JSON | one JSON object per line on stdout, each carrying the `kind` the reporter emits (`found`, `chat_list`, `progress`, `warning`, `failure`, `finished`), so a consumer reads a stream instead of waiting for a closing bracket; errors stay on stderr and keep their exit codes |
| `-q` | quiet | no progress and no summary; a result still prints (the list from `-l`, the JSON from `-j`), errors still reach stderr and the exit code still carries the result |
| `-d` | diagnostics | what was probed and decided: the files found, the columns recognised, each media file placed, each one missing, each video left without a poster |
| `-n` | dry run | the same work up to the point of writing: what would be built, how many messages and media files, how many missing, the estimated bytes from the source sizes |
| `-c` | check | the source is verified: on Android the `crypt15` header and key type first, without the key, then — with the key, which it therefore needs — `PRAGMA integrity_check` on the decrypted database and the count of media present against media referenced; on an iPhone backup `PRAGMA integrity_check`, every media path a row names resolved through `Manifest.db`, and media present against media referenced, since there is nothing to decrypt. It reports an incremental older than the full backup |

Rules that decide the awkward cases:

- `-n` changes a build, so it needs `-b`: without it it is an error, exactly like `-o`.
- `-c` needs no `-b`: it checks the source and exits. With `-b` it checks first and then builds; with
  `-n` it checks, then reports what it would build, and writes nothing.
- `-c` on an Android backup asks for the key exactly as a build does: the header can be read without
  it, the database cannot. On an iPhone backup it asks for nothing.
- `-c` exits `0` when the source is sound, `4` when the build would be incomplete (a media file
  missing, an incremental behind the full backup), and `1`, `2`, `3` as a build does for an unreadable
  source, a wrong key and a wrong backup type.
- `-j` and `-q` compose: quiet silences the human chatter, JSON still prints. `-j` and `-d` compose too,
  the diagnostics becoming events of the same stream.
- `-q` and `-d` say opposite things: together they are an error rather than one silently winning.
- Every new flag is in `-h`, in the usage page's flag table and in the examples. No exit code is added:
  `-c` and `-n` use `0..5` for the same situations a build does.

---

## 7. Testing

- **A synthetic iPhone fixture in the repository**, generated by a script next to the Android one: a
  `ChatStorage.sqlite` with chats, a group with senders, media rows, a call, a location, a vCard, a
  deleted and an edited message, inside a `Manifest.db` backup. Committed.
- **Unit tests on the adapter**: every mapped field, the 2001 epoch, group authors, the media path,
  missing media, a schema with a renamed column.
- **Unit tests on the new media path**: an image thumbnail at the right size and orientation, a poster
  taken from the stored thumbnail, a video with no stored thumbnail (first frame decoded), an HEVC one
  (placeholder, exit `0`), a waveform from an Opus fixture, a container whose duration cannot be read,
  a corrupt file (per-file failure, no abort).
- **End-to-end**: the iOS fixture builds archives that pass the same contract tests as Android, with
  its own golden chunk; the Android golden chunk moves once, at M5, by the thumbnail extension, and the
  diff is read before it is committed.
- **A real backup, outside the repository**, run by hand on both phones.
- **Per platform, the packaged artifact is checked**: extract it, `wexport -v`, one build on the
  fixture; on `macos-14` also `lipo -info` and `codesign -dv`; on `windows-2022` the `.zip` is
  extracted and run, because a cross-build proves that it links, not that it starts. Plus the point of
  the whole change: the build runs on a machine with **no `ffmpeg` on `PATH`**, asserted in CI by
  removing it before the test run.
- **The gate runs on macOS too**, not only the artifact check: the same `scripts/gate.sh` on the
  `macos-14` runner, at least on every push to `main`. The suite has filesystem assumptions that APFS
  treats differently — `temp_dir()` resolves through `/private`, names are compared case-insensitively,
  and the media tests read inodes — so a green Ubuntu tree is not evidence for a green Mac one. A
  failure there is fixed in the code, not excluded from the run.
- The gates keep their shape — `scripts/gate.sh` on a push, `scripts/gate-release.sh` before a tag —
  and gain the macOS leg.

---

## 8. Milestones

| # | Work | Acceptance |
|---|---|---|
| M1 | **macOS artifact**: `macos-14` job, two targets + `lipo`, `release.sh --stage` gains the third archive | the archive is extracted and run on the macOS runner: `wexport -v`, `lipo -info`, `codesign -dv`, one chat built |
| M2 | Source detection: `Source` enum, discovery for a backup directory and a bare `ChatStorage.sqlite` | every existing test passes; a fake iOS folder is recognised, a real Android one still builds, an encrypted backup is refused by name (exit `3`), two device folders in one path are listed and refused without a terminal (exit `1`) |
| M3 | `src/ios.rs`: chats, messages, senders, the 2001 epoch | unit tests cover every field and a renamed column |
| M4 | iPhone media: paths in the app group, `ZTHUMBNAILLOCALPATH` posters, missing files | the iOS fixture builds an archive that passes the viewer contract |
| M5 | **Media without `ffmpeg`**: image path (JPEG out), container metadata, phone thumbnails on both phones, `openh264` poster when the phone stored none, `libopus` waveform, the startup check removed | the suite passes with no `ffmpeg` on `PATH`; the golden chunk moves once by the thumbnail extension and the diff is read; an H.264 video with and one without a stored thumbnail both get a poster, an HEVC one without keeps the placeholder and still exits `0` |
| M6 | **Structured reporting**: `ui.rs` emits events (`found`, `chat_list`, `progress`, `warning`, `failure`, `finished`) and the CLI prints them, with the current output pinned by tests | the CLI's text and the archives are unchanged; this is also the GUI's prerequisite (§10.1) |
| M7 | **The five flags**: `-j`, `-q`, `-d`, `-n`, `-c`, on top of M6, with the interaction rules of §6 and `-h` | each flag has a test, the JSON is parsed by a test, `-q` with `-d` is refused, `-c` reports a damaged database and a missing media file |
| M8 | Docs (usage page for an unencrypted iPhone backup, README box, `-h`, requirements, the flag table), `CHANGELOG.md` 2.0 entry, validation on real backups, 2.0 tag | download, extract and run works on both phones, on all three platforms |

**The documentation alignment in M8 is every file.** A document that still
describes 1.0 after the 2.0 tag is a defect, and it is fixed in M8, not reported:

- `README.md` — the three assets in the install table, the key box ("Android needs the 64-digit key; an
  iPhone needs an unencrypted local backup"), no `ffmpeg` in the requirements, the feature list.
- `docs/usage.md` — the flag table with `-j -q -d -n -c`, the "what opens" table with the iPhone rows,
  the iPhone steps (unencrypted local backup, where to find it, what to copy), the requirements without
  `ffmpeg`, the troubleshooting table for the new messages.
- `docs/implementation-plan.md` — §2's CLI surface, §5 without the `ffmpeg` requirement, §6/§7 packaging
  for three platforms, the repository tree with `src/ios.rs` and the new fixtures.
- `docs/android-backup-formats.md` — **renamed to `docs/backup-formats.md`**, so the title does not lie:
  it becomes the container reference for both phones, with the iOS `ChatStorage.sqlite` and
  `Manifest.db` layout added, and every reference to the old name followed.
- `CHANGELOG.md` — the 2.0 entry, one line per change a user notices.
- `-h` — the five flags, the requirements line without `ffmpeg`, the iPhone sentence.
- `SECURITY.md` — the scope extended to the iOS parser, the thumbnails read from the database and the
  two compiled libraries (`openh264`, `libopus`), which are parsing surfaces like everything else.
- `.opencode/skills/wexport-implement/SKILL.md` — the module map and the fixture recipe.
- `docs/plan-2.0.md` — **absorbed and deleted at the tag**: what is worth keeping moves into the
  specification and the usage page, the way a TODO file leaves when its steps are done.

Nothing is copied by hand: after the tag the authority is the code and `-h`, and the tables in the docs
are rebuilt from them, so a drift shows up as a difference instead of two prose texts to compare.

Each milestone is committed and pushed on its own; the release gate runs once, before the 2.0 tag.

---

## 9. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| **H.264 decoding with `openh264`** | a video whose first frame cannot be decoded gets no poster | the phone's thumbnail is tried first; the gap is counted in the summary and named by `-d`, and never aborts the build |
| **A video in a codec with no decoder** (HEVC and others) | the placeholder stays, no poster | the phone's thumbnail covers nearly every video; an HEVC decoder is not in 2.0 (§11) |
| Phone thumbnails are small | softer posters than the 480 px ones 1.0 generates | accepted: it is the preview WhatsApp itself shows, at the size the phone keeps |
| Container parsing is not `ffmpeg` | odd files unreadable | per-file failure, never an aborted build; fixtures for MP4, WebM, OGG |
| `libopus` and `openh264` are C and C++ | two more vendored dependencies, and a C++ compiler per target | BSD, built by `cc` in the crate; without them, no waveform and no decoded poster |
| The `-wal` not copied beside the database | the last days of every chat missing | the extractor takes all three files; a test asserts a WAL-only message is in the archive |
| Two backups in the folder handed over | the wrong device built | discovery lists the device folders and asks for one |
| The universal macOS build fails | no macOS artifact | the job runs in CI from M1; `lipo -info` checked before publishing |
| The Windows `.zip` is only cross-built | a package that links and does not start | extracted and run on `windows-2022` before publishing |
| Core Data schema differs between iOS versions | the adapter reads nothing, or the wrong column | probe the schema and fail naming the columns; a fixture for each layout that matters |
| Media in more than one place (`Message/Media`, `Library/Media`) | missing media | resolve both, test both |
| The product's one-line identity ("Android, 64-digit key") | it describes only half of what opens | rewritten once, in M8 |

---

## 10. Later, deliberately not now

1. **The graphical application.** A window over the same pipeline, the CLI shipped inside the same
   artifact. Prerequisite: `ui.rs` emits events instead of printing.
2. **Portable packaging for the GUI**: **AppImage** on Linux (current
   [`type2-runtime`](https://github.com/AppImage/type2-runtime), statically linked, so `libfuse2` is not
   required — proven in a container without it), a **signed, notarized `.dmg`** on macOS (signing is
   what makes a downloaded `.app` open on a double click), a **portable `.exe`** on Windows. Tauri
   generates all of them from one configuration, one runner per system.
3. **Encrypted iPhone backups**: Apple's keybag (PBKDF2, class keys, AES-CBC per file).
4. **HEVC decoding**, for an iPhone video whose thumbnail the phone never stored.

---

## 11. Explicit non-goals for 2.0

No GUI, no AppImage, no `.deb`/`.rpm`/Flatpak/Snap, no MSI, no installer, no iCloud, no password or
passkey chains, no transcoding, no network. **No deprecation of anything Android**: the `crypt15` path,
its incrementals and its key are the product as much as the iPhone path is; 2.0 adds beside them.
