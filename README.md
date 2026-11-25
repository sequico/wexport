# wexport

Turn a WhatsApp backup into a local, scrollable archive of your chats — every message, every image,
video and voice note, with each timestamp frozen at build time so the archive still says when something
happened in ten years.

Both phones are read: Android's end-to-end encrypted `crypt15` file (with its 64-digit key), and an
iPhone's local backup made by Finder or iTunes.

Each chat becomes one folder that opens by double-clicking `index.html`: no server, no wexport, no
network. Give someone one folder and they see one conversation and nothing else.

**WhatsApp's own *Export chat* is built for forwarding a conversation, not for keeping one.** It
exports a single chat at a time, as a plain-text file with the media flattened beside it — and on the
long conversations people actually want to keep, its text comes out tangled. That is the problem
wexport was written for, and it is why it goes after the backup instead of the export:

- **Every chat in one run, with no practical ceiling.** 7 000 chats and 566 000 messages is the real
  backup this tool was built against, and `-b all` builds them all.
- **No ceiling on how long a chat is.** A conversation is streamed out of the database and written one
  month at a time, so a chat of hundreds of thousands of messages never has to fit in memory.
- **A site that stays usable at gigabytes.** The viewer keeps only the rows on screen, loads one month
  at a time and releases the months far from the viewport; search walks the chunks with a progress
  line instead of freezing the tab. Media is content-addressed, so a photo sent ten times is one file.
- **Offline, always.** The tool never opens a connection; the archive never loads anything from
  outside its own folder.
- **One folder per chat.** Zip it, copy it to a USB stick, send it: it still works.
- **Frozen time.** Every message carries its wall clock with an explicit UTC offset, written once at
  build time. Reopened in ten years, on any machine, it shows the same hour.
- **One file, no runtime.** A single binary per platform — Linux x86-64, Windows x86-64 and macOS
  universal — with no installer, no interpreter, no runtime library of ours and **no `ffmpeg`** to
  install: the thumbnails, posters and waveforms are made by the binary itself.

![The archive viewer](docs/viewer.png)

> **Android needs the 64-digit key; an iPhone needs a local backup that is not encrypted.** Android
> `crypt15` backups are opened with the key WhatsApp shows when end-to-end encrypted backup is turned
> on; a password or a passkey keeps that key on WhatsApp's servers. An iPhone's data is read from a
> Finder/iTunes backup with "Encrypt local backup" off, with no key involved.
> [Turning on the backup and saving the key →](docs/usage.md)

## Install

The [releases](https://github.com/sequico/wexport/releases) carry one archive per platform, with its
SHA-256 beside it:

| Platform | Asset | First run |
|---|---|---|
| Linux x86-64 | `wexport-<version>-linux-x86_64.tar.gz` | `tar xzf wexport-<version>-linux-x86_64.tar.gz && ./wexport ~/Downloads/whatsapp-copy` |
| Windows x86-64 | `wexport-<version>-windows-x86_64.zip` | unzip it, then `wexport.exe C:\Users\you\Downloads\whatsapp-copy` |
| macOS universal | `wexport-<version>-macos-universal.tar.gz` | `curl -LO <url> && tar xzf wexport-<version>-macos-universal.tar.gz && ./wexport ~/Downloads/whatsapp-copy` |

Each archive holds the binary — `wexport` on Linux and macOS, `wexport.exe` on Windows — together with
`LICENSE` and `THIRD-PARTY.md`. The SHA-256 of each archive sits beside it in the release. On macOS the
archive is extracted from a terminal, so Gatekeeper has no reason to stop it; there is nothing to
right-click or approve.

Nothing else is installed. `tesseract` is needed only to read the key from a screenshot.

From source instead: `cargo build --release --locked` (Windows: add
`--target x86_64-pc-windows-gnu`).

## How it works

```
Android:  phone ──► folder you copy by USB ──► wexport ──► one folder per chat ──► double-click index.html
                  + 64-digit key

iPhone:   Finder/iTunes backup ──► wexport ──► the same folders
```

`wexport` reads Android's `msgstore.db.crypt15` (and the daily incrementals), decrypts it with the
64-digit key WhatsApp showed you when you enabled end-to-end encrypted backups, and joins the messages
with the `Media/` folder you copied. For an iPhone it reads the `ChatStorage.sqlite` inside a local
backup, with its log, and follows the manifest to the media files. Either way the output is the same
kind of folder.

## First run

```sh
wexport ~/whatsapp/phone-copy
```

It finds the backup wherever the copy put it, asks for the key when it cannot find `key.png`,
`key.txt` or `encrypted_backup.key` beside it, and lists the chats with a code each:

```
  #  last event                     msgs  media      size  chat
  1  2026-12-02 12:00:00 +01:00        7      1     100 B  Alice
  2  2026-12-02 11:04:00 +01:00        9      6    7.1 kB  Family
```

The code is what `-b` takes. Archives land in `<folder>/output/`, one folder per chat plus an
`index.html`, and the media part runs behind a progress bar.

[Every flag, the orders and the exit codes →](docs/usage.md#8-commands-and-flags)

## The archive

```
output/2026-12-02-family-9fd6/
├── index.html      # opens by double-click; no server, no fetch, no modules
├── app.js, app.css
├── README.txt      # what this folder is, and when it was built
├── data/           # manifest.js plus one classic script per month
├── media/          # originals, content-addressed
└── deriv/          # thumbnails and posters
```

The viewer is plain classic scripts — no bundler, no `fetch`, no workers — because that is what
`file://` allows. It virtualises the timeline (a few hundred nodes whatever the chat length), releases
chunks that scroll out of view, searches across all of them, jumps to a date, and opens images, video
and voice notes straight from the folder. Keyboard: `/` search, `j`/`k` scroll, `g`/`Home` first
message, `G`/`End` last.

Media the user did not copy is a normal state, not an error: the archive shows a labelled placeholder —
and when the phone kept a preview of that file, the picture is there with the label instead of an empty
box.

## The key

For Android, three forms, all read offline:

- the 64 hex digits (spaces and a 4×4 grid are fine);
- a screenshot of them (needs `tesseract`);
- `encrypted_backup.key` from a rooted phone.

Without any of them the tool asks once on the terminal.

For an iPhone there is no key: the backup is copied by Finder with "Encrypt local backup" off.

## Scope

- **Android** backups in `crypt15` format (end-to-end encrypted, 64-digit key), full and incrementals.
- **iPhone** local backups made by Finder or iTunes, unencrypted, and an already-extracted
  `ChatStorage.sqlite`.
- The user copies the phone folder or the backup themselves; the tool never touches the device.
- Not supported, deliberately: encrypted iPhone backups, iCloud, Google Drive download,
  password-based or passkey-based Android backups, `crypt12`/`crypt14`, re-encrypting anything back.

## Privacy

- Nothing leaves the machine: no telemetry, no update check, no network code at all.
- The archive is **plaintext**: anyone holding the folder can read the conversation. The build summary
  says so.
- The test fixtures in this repository are generated by the repository itself. No real WhatsApp data is
  committed.

## Exit codes

`0` success · `1` unreadable input, or a question with no terminal · `2` unusable key · `3` wrong
backup type (including an encrypted iPhone backup) · `4` build incomplete (the archive is usable) ·
`5` a missing tool — only `tesseract`, for a key given as a screenshot.

The same table, with what each one means for the person running it, is in [docs/usage.md](docs/usage.md#9-what-can-go-wrong).

## Status

**2.0.0.** Two phones, three platforms, media made in-process. The suite covers the container, both
schema adapters, the archive bytes (a committed golden chunk per phone), a rebuild that changes
nothing and removes what is gone, the memory ceiling, and the iPhone database with its log. The viewer
is checked from `file://` in headless Chromium, on the same files a desktop browser opens.

## Documentation

- [Implementation plan](docs/implementation-plan.md) — the specification: pipeline, data model,
  archive format, milestones, risks, decisions.
- [Backup formats](docs/backup-formats.md) — what is inside the files, for Android and iPhone, and
  which key opens which format.
- [Usage](docs/usage.md) — the phone steps: turning on the 64-digit key, saving it, making the
  backup, copying the folder, running wexport, and what each error means.

## Development

```sh
cargo test                                   # unit and end-to-end runs on the fixtures
./scripts/gate.sh                            # what a push must pass, about twenty seconds
./scripts/gate-release.sh                    # the same, plus mutation testing, fuzzing and TSan
```

The Android fixture is `tests/fixtures/synthetic/phone`, built by
`tests/fixtures/synthetic/build.py`; the iPhone one is `tests/fixtures/synthetic/ios/backup`, built by
`tests/fixtures/synthetic/ios/build.py`. See [docs/implementation-plan.md](docs/implementation-plan.md)
for the specification, and `AGENTS.md` for the project rules an agent follows here.

## Contributing

Issues and pull requests are welcome, under the [Code of Conduct](CODE_OF_CONDUCT.md). Run
`./scripts/gate.sh` before opening a pull request: it is the check `main` must pass. A change to the archive format, the command line or the viewer also belongs in
`docs/implementation-plan.md`, which is the specification those three follow.

## License

**GPL-3.0-or-later**, see [LICENSE](LICENSE). The dependencies and the two compiled-in libraries are
listed with their notices in [THIRD-PARTY.md](THIRD-PARTY.md). The `crypt15` key derivation and
container layout follow [`wa-crypt-tools`](https://github.com/ElDavoo/wa-crypt-tools) (GPL-3.0),
reimplemented here and pinned by a test to the value that reference computes.

wexport is written and maintained by **[Sequi Company](https://sequi.company)** (Ancona, Italy) —
`info@sequi.company`. Copyright (C) 2026 Sequi Company; see [AUTHORS](AUTHORS).
