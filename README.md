# wexport

Turn an Android WhatsApp backup into a local, scrollable archive of your chats — every message,
every image, video and voice note, with every timestamp frozen at build time so the archive still
says exactly when something happened in ten years.

Each chat becomes one folder that opens by double-clicking `index.html`: no server, no wexport, no
network. A folder can be zipped and given to someone without exposing the other chats.

One static binary, no runtime dependencies beyond `ffmpeg`/`ffprobe`.

## What it does

```
phone ──► folder you copy by USB ──► wexport ──► one folder per chat ──► double-click index.html
                                     + 64-digit key
```

`wexport` reads `msgstore.db.crypt15` (and the daily incrementals), decrypts it with the 64-digit key
WhatsApp showed you when you enabled end-to-end encrypted backups, joins the messages with the
`Media/` folder you copied, and writes the archive. It runs entirely offline and uploads nothing.

## Build and run

```sh
cargo build --release
./target/release/wexport ~/whatsapp/phone-copy
```

It finds the backup, asks for the key if it cannot find `key.png`/`key.txt` beside it, prints the
chats numbered by most recent event, and asks which to build:

```
  #  last event                     msgs  media      size  chat        jid
  1  2026-12-02 12:00:00 +01:00        7      1     100 B  Alice       393331234567@s.whatsapp.net
  2  2026-12-02 11:04:00 +01:00        9      6    7.1 kB  Family      120363000000000001@g.us

which? numbers or part of a name [the 10 most recent: 1, 2]:
```

Archives land in `<folder>/archives/`, one folder per chat, plus an `index.html`; the media part runs
behind a progress bar. For scripts the same thing without questions:

```sh
wexport ~/whatsapp/phone-copy --list            # only the list
wexport ~/whatsapp/phone-copy --chat 2          # one chat
wexport ~/whatsapp/phone-copy --chat family --yes
wexport ~/whatsapp/phone-copy --yes             # everything
```

`wexport -h` prints the flags, the exit codes, the requirements and worked examples.

The key can be 64 hex digits (spaces and a 4×4 grid are fine), a screenshot of them, or the phone's
`encrypted_backup.key` on a rooted device. The screenshot path needs `tesseract`. `ffmpeg` and
`ffprobe` are required and missing ones abort at startup with the install command for the platform.
See [the user flow](docs/user-flow.md).

## Scope

- **Android only**, backups in `crypt15` format (end-to-end encrypted, 64-digit key).
- The user copies the phone folder themselves; the tool never touches the device.
- Not supported, deliberately: iPhone, Google Drive download, password-based or passkey-based
  backups, legacy `crypt12`/`crypt14`, re-encrypting anything back.

## Archive

Per chat: `index.html`, `app.js`, `app.css`, `README.txt`, `data/` (one script per month,
`manifest.js`), `media/` (content-addressed originals), `deriv/` (thumbnails, posters). The viewer is
plain classic scripts — no modules, no bundler, no `fetch` — so it works from `file://`. Timestamps
are strings with their UTC offset, computed at build time; nothing in the archive needs the machine
that built it.

## Layout

| Path | Job |
|---|---|
| `src/crypt15.rs` | varint + protobuf header, AES-256-GCM, zlib/ZIP payload |
| `src/keys.rs` | 64 hex digits, key file, screenshot via tesseract |
| `src/discover.rs` | find the backup anywhere in the copy, the key-type gate, the older-chain report |
| `src/msgstore.rs` | the only place WhatsApp's table and column names live |
| `src/merge.rs` | incremental replay into the full backup |
| `src/media.rs` | content-addressed placement, thumbnails, posters, waveform peaks |
| `src/chunks.rs` | month chunks as classic scripts, escaping, splitting |
| `src/archive.rs` | one folder per chat, manifest, README, index page, prune |
| `src/ui.rs` | flags, the numbered picker, the run |
| `src/progress.rs` | the one-line progress bar |
| `src/assets/` | the viewer |

## Tests

```sh
cargo test                     # unit tests plus end-to-end runs on the committed fixture
cargo clippy --all-targets     # must be silent
cargo fmt --check
```

The suite runs on `tests/fixtures/synthetic/phone`: a crypt15 backup this repository builds itself,
with one message per kind, a group, a call, a reaction, a missing media file and a delta. No real
WhatsApp data is ever committed. The generator is `tests/fixtures/synthetic/build.py`, and the key it
encrypts with is a fixed test constant.

The unit test that matters most pins the key derivation to the value the reference implementation
computes: a silent change there would make every archive wrong in a way no fixture catches.

## Documentation

- [Implementation plan](docs/implementation-plan.md) — the specification: pipeline, data model,
  archive format, milestones, risks, decisions.
- [Android backup formats](docs/android-backup-formats.md) — what is inside the files, and which key
  opens which format.
- [User flow](docs/user-flow.md) — what the person with the phone has to do, step by step.

## Working on it with an agent

Two skills live in `.opencode/skills/`, and they load only when relevant:

- `wexport-implement` — the procedure and the invariants for editing code here, plus the synthetic
  fixture recipe in `references/fixtures.md`.
- `offline-html-archive` — how to build an archive that opens from `file://` with no server.

They point at the plan rather than restating it: the plan is the spec, the skills are the practice.

## License

**GPL-3.0-or-later** (`SPDX-License-Identifier: GPL-3.0-or-later`), see [LICENSE](LICENSE). The
`crypt15` key derivation and container layout are the same as those of
[`wa-crypt-tools`](https://github.com/ElDavoo/wa-crypt-tools) (GPL-3.0), reimplemented here and pinned
by a test to the value that reference implementation computes.
