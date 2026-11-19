# wexport 2.0 — plan

Three things, and nothing else:

1. the interface stays the **command line**;
2. **iPhone** backups join Android's;
3. the tool ships for **macOS, Linux and Windows**.

The archive format does not change: an archive built from either phone is the same kind of folder, and
a 1.0 archive opens in 2.0 and the other way round. `docs/implementation-plan.md` remains the contract
for the pipeline and the archive; this document adds a second data source and a third platform.

**iPhone is added, never substituted.** Every Android behaviour stays exactly as it is — `crypt15`, the
incrementals, the 64-digit key, the same flags, the same archives. 2.0 removes nothing and changes
nothing on that path; a user who only has Android cannot tell 2.0 from 1.0 except by the version number.

---

## 1. Scope

| | 1.0 (shipped) | 2.0 |
|---|---|---|
| Interface | CLI | CLI, same flags |
| Data sources | Android `crypt15`, 64-digit key | the same, **plus iPhone local backups** |
| Platforms | Linux x86-64, Windows x86-64 | Linux x86-64, Windows x86-64, **macOS (universal)** |
| Packaging | `.tar.gz` (Linux), `.zip` (Windows) | `.tar.gz` / `.zip` / `.tar.gz`, each with a `.sha256` |
| Archive format | one folder per chat, `file://` viewer | unchanged |

The graphical application, the AppImage and the signed `.dmg` are **later work** (§7). Nothing here
touches Tauri, signing or the desktop.

---

## 2. Distribution: one artifact per platform, built where it can be built

| Platform | Artifact | Built on | Notes |
|---|---|---|---|
| Linux x86-64 | `wexport-<v>-linux-x86_64.tar.gz` (holds `wexport`) | `ubuntu-24.04` | unchanged from 1.0 |
| Windows x86-64 | `wexport-<v>-windows-x86_64.zip` (holds `wexport.exe`) | `ubuntu-24.04` with the mingw linker | unchanged from 1.0 |
| macOS universal | `wexport-<v>-macos-universal.tar.gz` (holds `wexport`) | `macos-14` | **new**: `aarch64-apple-darwin` + `x86_64-apple-darwin`, joined with `lipo` |

**macOS needs no Apple account.** The reasoning, which is worth writing down because it is the whole
cost of this platform:

- The artifact is a `.tar.gz` extracted from a terminal. The quarantine attribute is set by the
  browser on the *downloaded archive*, and `tar` does not copy a container's extended attributes onto
  the files it extracts, so the binary is not quarantined and Gatekeeper does not stop it. The one
  exception is double-clicking the archive in Finder (Archive Utility *does* propagate the flag), and
  the install line in the docs is a `curl` + `tar` anyway.
- On Apple Silicon a Mach-O must carry at least an ad-hoc signature to execute; the linker adds one
  when we build, so `codesign -dv ./wexport` shows it. That is not notarization, and notarization is
  only needed to survive a quarantined file — which the tarball path avoids.
- `ffmpeg` and `ffprobe` still come from the machine: `brew install ffmpeg`. The missing-binary
  message grows the macOS line (`brew install ffmpeg`), like it already has apt/dnf/pacman/winget.

**Where the build runs.** macOS cannot be cross-compiled from Linux, so the release job grows a second
runner:

```
ubuntu-24.04   →  wexport (ELF) + wexport.exe (mingw)      →  dist/
macos-14       →  wexport (arm64) + wexport (x86_64)       →  lipo  →  dist/
```

`scripts/release.sh` stops being the only producer: it stages the three artifacts (from the workflow
run when cutting a release), checks that each holds the binary it claims and that the inner binary
hashes match what was built, and writes the `.sha256` files. **A release refuses to proceed without
the macOS archive**, the same way it already refuses without the Windows one.

---

## 3. iPhone: what can be opened

A local backup made by Finder or iTunes is a folder of hashed files plus a `Manifest.db` that maps each
file to a domain and a path. WhatsApp's data lives in its app group
(`group.net.whatsapp.WhatsApp.shared`): `ChatStorage.sqlite` (Core Data) and the media under
`Message/Media/…`. The message schema is nothing like Android's: `ZWACHATSESSION`, `ZWAMESSAGE`,
`ZWAMEDIAITEM`, `ZWAGROUPMEMBER` and friends.

| How the data is held | 2.0 |
|---|---|
| Local backup, **not encrypted** (Finder/iTunes, "Encrypt local backup" off) | **opens** — no key involved |
| An already-extracted app container or `ChatStorage.sqlite` | **opens** |
| Local backup **encrypted** with the Apple backup password | not in 2.0: it needs Apple's keybag (§7.2) |
| **iCloud** backup | never offline: it needs the Apple account and the network. Out. |
| WhatsApp's 64-digit key / passkey | irrelevant here: on iPhone it protects the **cloud** copy |

The public message becomes: *Android needs the 64-digit key; an iPhone needs a local backup that is
not encrypted.* README, the usage page and `-h` say it in the milestone that ships it.

---

## 4. Architecture

```
discover  ──►  Source::Android(Inputs)  ──►  msgstore (existing)  ─┐
          └─►  Source::Ios(IosInputs)  ──►  ios      (new)         ├─►  Message ─► archive (unchanged)
                                                                   ┘
```

- **`discover`** detects the source instead of assuming `crypt15`: a copied phone folder, a
  `Manifest.db` backup directory, or a `ChatStorage.sqlite`. The Android path is unchanged behind it.
- **`src/ios.rs`** is the second schema adapter: it opens `ChatStorage.sqlite` read-only, resolves the
  columns by probing the schema — names are discovered, never guessed from an app version, the same
  rule as Android — and produces the existing `Chat`/`ChatStat`/`Message`.
- **Media** resolves from the app group: `ZWAMEDIAITEM.ZMEDIALOCALPATH` is a relative path inside the
  backup; content-addressed placement, thumbnails and waveforms apply as they are.
- **Time**: Core Data counts seconds since 2001-01-01, the archive stores Unix milliseconds. The
  conversion lives in `ios.rs` only, pinned by tests on a winter and a summer date.
- **CLI**: the flags do not change. `-k` stays the Android key and is not asked for on an iPhone
  backup. Chat codes, ordering and `-t ZONE` behave identically.

---

## 5. Testing

- **A synthetic iPhone fixture in the repository**, generated by a script next to the Android one: a
  `ChatStorage.sqlite` with chats, a group with senders, media rows, a call, a location, a vCard, a
  deleted and an edited message, inside a `Manifest.db` backup. Committed.
- **Unit tests on the adapter**: every mapped field, the 2001 epoch, group authors, the media path,
  missing media, and a schema with a renamed column.
- **End-to-end**: the iOS fixture builds archives that pass the same contract tests as Android —
  folder names, record shape, frozen stamps, a rebuild that changes nothing — with its own golden
  chunk. The Android golden chunk stays untouched.
- **A real backup, outside the repository**: an iPhone backup run by hand, the way the Android
  real-backup test works.
- **Per platform, the packaged artifact is checked**: extract it, run `wexport -v` and one build on
  the fixture; on macOS also `lipo -info` (both architectures present) and `codesign -dv`.
- The gates do not change: `scripts/gate.sh` on a push, `scripts/gate-release.sh` before a tag.

---

## 6. Milestones

| # | Work | Acceptance |
|---|---|---|
| M1 | **macOS artifact**: `macos-14` job, two targets + `lipo`, `release.sh` stages three artifacts, `install_hints` gains `brew install ffmpeg` | the macOS archive is published, extracted on a Mac, and builds a chat |
| M2 | Source detection: `Source` enum, discovery for a backup directory and a bare `ChatStorage.sqlite` | every existing test passes; a fake iOS folder is recognised, a real Android folder still builds |
| M3 | `src/ios.rs`: chats, messages, senders, the 2001 epoch | unit tests cover every field and a renamed column |
| M4 | Media and attachments from the app group; missing files; thumbnails and waveforms | the iOS fixture builds an archive that passes the viewer contract |
| M5 | Docs (usage page for an unencrypted local backup, README box, `-h`), validation on a real backup, 2.0 tag | the three-command path works on a real iPhone backup, on all three platforms |

Each milestone is committed and pushed on its own; the release gate runs once, before the 2.0 tag.

---

## 7. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| No macОS runner or a broken universal build | no macOS artifact | the job is in CI from M1, so the failure shows itself immediately; `lipo -info` verified before publishing |
| `ffmpeg` absent on a Mac | the tool stops at startup | the message already names the install command per platform; macOS gets `brew install ffmpeg` |
| Core Data schema differs between iOS versions | the adapter reads nothing, or the wrong column | probe the schema and fail naming the columns, as Android does; a fixture per layout we can obtain |
| Media in more than one place (`Message/Media`, `Library/Media`) | missing media in the archive | resolve both, test both in the fixture |
| Core Data timestamps | dates off by 30 years | one conversion function, pinned on two dates |
| Encrypted local backups are common | "it does not open my backup" | the error names the fix ("turn off Encrypt local backup, make a new one") |
| No iPhone among the maintainers | untested code shipped | the real-backup run in M5 is a condition of the release |
| The one-line identity ("Android, 64-digit key") | documentation contradicts itself | changed once, in M5: the usage page's table and the README box |

---

## 8. Later, deliberately not now

- **The graphical application.** A window over the same pipeline, the CLI shipped inside the same
  artifact. Prerequisite: `ui.rs` emits events instead of printing.
- **Portable packaging for the GUI**, when it exists: **AppImage** on Linux (current
  [`type2-runtime`](https://github.com/AppImage/type2-runtime), statically linked, so `libfuse2` is not
  required — proven in a container without it), a **signed, notarized `.dmg`** on macOS (an unsigned
  `.app` double-clicked from a download is quarantined; ~99 USD/year), a **portable `.exe`** on Windows.
  Tauri is the one generator that produces all of them from one configuration, one runner per system.
- **Encrypted iPhone backups**: Apple's keybag (PBKDF2, class keys, AES-CBC per file). Not in 2.0;
  2.0 says what to do instead.

## 9. Explicit non-goals for 2.0

No GUI, no AppImage, no `.deb`/`.rpm`/Flatpak/Snap, no MSI, no installer, no iCloud, no password or
passkey chains, no transcoding, no network.

Also: **no deprecation of anything Android.** The `crypt15` path, its incrementals and its key are the
product as much as the iPhone path is; 2.0 adds beside them.
