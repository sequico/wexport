# wexport 2.0 — plan

The version that carries the tool to people who do not use a terminal: the same pipeline, a graphical
application, and the command line still there for scripts. One portable artifact per platform, no
installer, no package manager, no runtime to fetch.

This document is the plan, not the specification: `docs/implementation-plan.md` stays the contract for
what the pipeline and the archive are. Everything below is 2.0 work, and nothing here changes the
archive format.

---

## 1. What 2.0 is

| | 1.0 (shipped) | 2.0 |
|---|---|---|
| Interface | CLI only | a window that walks through folder → key → chats → build, **and** the CLI |
| Delivery | `wexport-<v>-linux-x86_64.tar.gz`, `wexport-<v>-windows-x86_64.zip` | one portable artifact per platform, Linux as **AppImage** |
| Archive format | one folder per chat, `file://` viewer | unchanged (a 1.0 archive opens in 2.0 and the other way round) |
| Source of the data | Android `crypt15`, 64-digit key | unchanged in 2.0 (iPhone is a separate decision, §8) |

The GUI is not a rewrite: it is a second front-end on the `wexport` library the CLI already uses.

---

## 2. The one thing to decide before any work: AppImage is Linux only

**An AppImage is an ELF binary carrying a SquashFS image and a FUSE runtime. macOS and Windows cannot
execute it, and no compatibility layer changes that.** "Multiplatform AppImage" cannot be literal, so
the intent has to be preserved instead: *one file, double-click, nothing to install* — which each
platform spells differently.

| Platform | AppImage | The same promise, that platform's format |
|---|---|---|
| Linux | yes | **AppImage** (decided) |
| macOS | impossible | `.app` inside a `.dmg`, signed and notarized; unsigned it is quarantined by Gatekeeper |
| Windows | impossible | portable single `.exe` (NSIS `portable` mode) or `wexport.exe` in a `.zip` |

**Decision (owner): only portable artifacts — no `.deb`, no `.rpm`, no Flatpak, no installer.** On
Linux that is exactly the AppImage. On macOS and Windows it is the portable file above, which is the
closest thing either system has to an AppImage.

**Open decision (owner):** accept those equivalents on macOS and Windows, or ship 2.0 on Linux only.
Recommended: accept them; the promise the user experiences is the same.

Also decided: **the CLI ships with the application**, not as a separate download. Every 2.0 artifact
contains both; §4 says how the CLI is reached from inside an AppImage.

---

## 3. Architecture

```
src/lib.rs            the pipeline (unchanged)
src/ui.rs      ──►    wexport-cli: the same behaviour, printing through a Reporter
src/app/       ──►    wexport-app: the window, drawing through the same Reporter
src/archive, chunks, crypt15, discover, keys, media, merge, msgstore, progress, error   (unchanged)
```

**The one refactor that makes both possible**: `ui.rs` prints and draws a bar with `println!`. It has
to emit events instead — `Reporter` with `found`, `chat_list`, `progress(chat, done, total)`,
`warning`, `failure`, `finished` — and two implementations: one writes the terminal text it writes
today, the other sends messages to the window. The CLI's output must not change at all; the golden
and end-to-end tests already pin the archive, and the CLI tests pin the messages.

**Toolkit: Tauri** (recommended). Reasons: the project is Rust, the archive viewer is already HTML and
JS, the binary is small, and one configuration produces the AppImage, the `.dmg` and the Windows
portable `.exe`. The alternative, a native Rust toolkit (egui or Slint), avoids the webview dependency
and is lighter on old distributions, but the interface is plainer and every screen is more work.
Pick Tauri unless the webview dependency proves unworkable on the distributions we target.

**Screens** (one window, five steps):

1. **Folder** — pick or drop the copied phone folder; the tool reports what it found (backup date,
   size, media present).
2. **Key** — paste the 64 digits, drop the screenshot, or point at `encrypted_backup.key`; the same
   three forms the CLI accepts, with the same errors, in place.
3. **Chats** — the list with names, dates, message and media counts; check the ones to build, or all.
4. **Build** — one progress row per chat, file counts, a running total; cancelling stops cleanly with
   the archives already complete left in place.
5. **Done** — where the archives went, how many media were missing, a button that opens `index.html`.

The pipeline runs on a worker thread and reports through the `Reporter`; the window never blocks.
The archive viewer is untouched: it remains the reading interface, opened by the system browser or by
the button.

**Privacy is a hard constraint, not a feature.** The application opens no socket, ever — no update
check, no crash report, no telemetry. The only network-free guarantee the 1.0 CLI gives is one the GUI
must keep visibly: the window says so, and the dependency list stays reviewable.

---

## 4. Packaging and distribution

**Linux — AppImage.** Built with a pinned `appimagetool` whose runtime is the current
[`type2-runtime`](https://github.com/AppImage/type2-runtime): statically linked against musl and FUSE,
so **`libfuse2` is not needed on the target** (only the kernel's `/dev/fuse`, which every normal system
has). The build must *prove* it, not assume it: `--appimage-version` is recorded, and the artifact is
extracted and run inside a container without `libfuse2` before it is published. File name:
`wexport-<version>-x86_64.AppImage`. Desktop integration (menu entry, icon) is carried inside but
appears only with AppImageLauncher or a manual entry; the README says which.

**macOS — `.dmg`.** The `.app` bundle inside, universal (arm64 + x86_64 via `lipo`), built on a
`macos-14` runner. **Signing and notarization are mandatory** for a double-click experience: an
unsigned downloaded `.app` is quarantined by Gatekeeper. That means an Apple Developer ID
(~99 USD/year) and a CI step with `codesign` + `notarytool` + `stapler`. Without it, the artifact still
works but the user has to clear the quarantine attribute; that is the fallback, documented, not the
plan. File name: `wexport-<version>-universal.dmg`.

**Windows — portable `.exe`.** NSIS in portable mode (one file, no install, extracts to a temporary
folder and runs), or the `.exe` in a `.zip` if NSIS proves fragile. Authenticode signing is optional
and costs money; without it SmartScreen warns once. File name: `wexport-<version>-x86_64.exe`.

**The CLI inside the artifact.** Each artifact carries `wexport` (CLI) and the GUI entry point.
`AppRun` starts the window; `wexport.AppImage --cli …` passes the arguments to the CLI unchanged, and
`APPIMAGE_EXTRACT_AND_RUN=1` extracts it for anyone who wants the bare binary. On macOS the same
arguments go to `wexport --cli` from inside the bundle; on Windows to `wexport.exe --cli`.

**Checksums and the release procedure** stay as they are: `scripts/release.sh` produces the artifacts
and a `.sha256` for each, and refuses to publish an archive whose inner binary does not hash-match the
one it built. The version comes from `Cargo.toml`; `CHANGELOG.md` loses `Unreleased` in the same change.

---

## 5. Testing and gates

The archive stays byte-identical, so the existing tests keep their value: the golden chunk, the
end-to-end fixture, the real-backup run.

Added:

- **Reporter tests**: the CLI's text is asserted per event, so the refactor cannot change what 1.0
  prints.
- **GUI smoke test**: the app starts headless (virtual display), lists a fixture backup, builds one
  chat through the same pipeline, and the archive matches the CLI's. Cancellation is tested too.
- **Packaging smoke test, per platform**: the artifact is extracted and run —
  `--version`, `--cli -h`, and one real build on the fixture. On Linux inside a `libfuse2`-free
  container (that is the AppImage claim, so it is the test). On macOS, `codesign --verify` and
  `spctl --assess`. On Windows, the `.exe` runs under the runner.
- **No network**: a test that resolves nothing — the process runs with a blocked loopback and the
  suite still passes.

The push gate (`scripts/gate.sh`) grows the GUI unit tests; packaging smoke tests belong to the
release gate (`scripts/gate-release.sh`), because they need the three runners. A release is cut only
after that gate is green on the tree being tagged.

---

## 6. Milestones

| # | Work | Acceptance |
|---|---|---|
| M1 | `Reporter` refactor: `ui.rs` emits events, the CLI prints them; no behaviour change | 1.0's CLI output and archives unchanged, gate green |
| M2 | Tauri app: folder, key, chat list, build, cancel, open | builds a real chat from a phone copy on Linux |
| M3 | Packaging: AppImage (runtime proven), `.dmg`, portable `.exe`; `release.sh` produces all three with checksums | each artifact extracted and run on its platform, one build each |
| M4 | macOS signing + notarization; Windows signing decision; CI runners for macos and windows | a downloaded `.dmg` opens without the quarantine dance |
| M5 | Documentation: `docs/usage.md` rewritten around the window, CLI kept as the scriptable path, `-h` unchanged | a person who has never used a terminal builds a chat on each platform |

Every milestone is committed and pushed on its own; the release gate runs once, before the 2.0 tag.

---

## 7. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| **"AppImage on macOS/Windows" is impossible** | the plan as literally written cannot be built | the equivalents in §2; the decision is listed there |
| WebKitGTK inside the AppImage | size, GL driver mismatch on old distributions | bundle or declare, then measure on a minimal distro; fall back to a native toolkit if it breaks |
| AppImage runtime older than the static one | `libfuse2` reappears as a dependency | pin `appimagetool`, record `--appimage-version`, test in a `libfuse2`-free container |
| Apple notarization | 99 USD/year, an account, CI secrets | do it before 2.0 ships on macOS, or document the quarantine step and call the macOS build "unsigned" |
| The GUI widens the attack surface of a tool that reads private conversations | a bug that leaks data is worse than a missing feature | no network code, no telemetry, the same single dependency set, the SECURITY.md scope extended to the app |
| A second front-end drifts from the CLI | two behaviours to test and fix | one pipeline, one `Reporter` contract, one archive; the CLI is not a port of the GUI |

---

## 8. Out of scope for 2.0 (unless decided otherwise)

- **iPhone backups.** A different format, a different adapter, and — for iCloud — credentials and
  network access. It is a product of its own and is not implied by the GUI.
- **Cloud**: Google Drive, iCloud, any download.
- **Password and passkey chains**: the 64-digit key stays the only way in.
- **Transcoding**, an encrypted archive mode, a server, a viewer that needs an install.
- **Installers and package managers**: no `.deb`, `.rpm`, Flatpak, Snap, MSI. One portable file per
  platform is the product.
