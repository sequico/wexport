# wexport

Turn an Android WhatsApp backup into a local, scrollable archive of your chats — every message,
every image, video and voice note, with every timestamp frozen at build time so the archive still
says exactly when something happened in ten years.

Each chat becomes one folder that opens by double-clicking `index.html`: no server, no wexport, no
network. A folder can be zipped and given to someone without exposing the other chats.

## Two implementations, one product

```
py/     the original implementation, Python 3.11+ (uv, hatchling, wa-crypt-tools)
rust/   the rewrite, one static binary, no runtime dependencies
```

They produce the same archives: same folder names, byte-identical data chunks, same manifest apart
from its build timestamp. Thumbnails and posters differ only because the encoders do. The archive
format and the command line are the contract, and they are shared:

## What it does

```
phone ──► folder you copy by USB ──► wexport ──► one folder per chat ──► double-click index.html
                                     + 64-digit key
```

`wexport` reads `msgstore.db.crypt15` (and the daily incrementals), decrypts it with the 64-digit key
WhatsApp showed you when you enabled end-to-end encrypted backups, joins the messages with the
`Media/` folder you copied, and writes the archive. It runs entirely offline and uploads nothing.

## Usage

Any of the two, same flags:

```sh
# Rust: build once, then one file on PATH
cd rust && cargo build --release
./target/release/wexport ~/whatsapp/phone-copy

# Python
cd py && uv sync
uv run wexport ~/whatsapp/phone-copy
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
wexport ~/whatsapp/phone-copy --yes             # everything
```

`wexport -h` prints the flags, the exit codes, the requirements and worked examples.

The key can be 64 hex digits (spaces and a 4×4 grid are fine), a screenshot of them, or the phone's
`encrypted_backup.key` on a rooted device. The screenshot path needs `tesseract`; the Python tree also
needs its OCR extra (`uv sync --extra ocr`). `ffmpeg` and `ffprobe` are required and missing ones abort
at startup with the install command for the platform. See [the user flow](docs/user-flow.md).

## Scope

- **Android only**, backups in `crypt15` format (end-to-end encrypted, 64-digit key).
- The user copies the phone folder themselves; the tool never touches the device.
- Not supported, deliberately: iPhone, Google Drive download, password-based or passkey-based
  backups, legacy `crypt12`/`crypt14`, re-encrypting anything back.

## Status

v0.1.0, working end to end in both trees:

- **Python** — 94 tests collected (one skips by design without a real backup), `ruff check` clean; a
  Nuitka one-file Linux binary (≈20 MB) built from `py/src/wexport/__main__.py`; already delivered.
- **Rust** — 16 tests, `clippy --all-targets` and `fmt` clean; a 3.8 MB static binary; parity measured
  against the Python build on the committed fixture.
- Both — one command with the numbered picker and a media progress bar; crypt15 decrypt and
  incremental merge; msgstore adapter for the tables in `docs/android-backup-formats.md`;
  content-addressed media with thumbnails, posters and waveform peaks; month chunks; the `file://`
  viewer with virtualised rendering, search and jump-to-date; the plaintext warning and the
  `README.txt` in each archive.
- The viewer was verified in Chromium over `http` with Playwright; **double-clicking from `file://` in
  Chrome, Firefox and Edge still needs a manual pass** (the plan's M3 criterion), because the
  automation available here refuses `file://` URLs.
- Known limits, shared: a poll renders as its question text, without options or votes; incremental
  merge is row-level, so a message deleted only on the phone stays in the archive.

## Packaging the Python binary

From `py/`:

```sh
uv sync --group packaging
.venv/bin/nuitka --onefile --assume-yes-for-downloads --output-dir=dist --output-filename=wexport \
  --include-package=wa_crypt_tools --include-package-data=wexport --include-package=PIL \
  src/wexport/__main__.py
```

`__main__.py`, not `cli.py`: the packaged entry point has no parent package, so relative imports
would fail. `patchelf` is required on Linux; the viewer assets are pulled in by
`--include-package-data=wexport`. The Rust binary needs none of this: `cargo build --release`.

## Tests

```sh
cd py && uv run pytest          # 94 tests
cd rust && cargo test           # 16 tests
```

Both suites run on the same committed fixture, `py/tests/fixtures/synthetic/phone`: a crypt15 backup
this repository builds itself, with one message per kind, a group, a call, a reaction, a missing
media file and a delta. No real WhatsApp data is ever committed.

## Documentation

- [Implementation plan](docs/implementation-plan.md) — the specification: pipeline, data model,
  archive format, milestones, risks, decisions.
- [Android backup formats](docs/android-backup-formats.md) — what is inside the files, and which key
  opens which format.
- [User flow](docs/user-flow.md) — what the person with the phone has to do, step by step.
- [rust/README.md](rust/README.md) — the Rust tree, its module map and the measured parity.

## Working on it with an agent

Two skills live in `.opencode/skills/`, and they load only when relevant:

- `wexport-implement` — the procedure and the invariants for editing code here, plus the synthetic
  fixture recipe in `references/fixtures.md`.
- `offline-html-archive` — how to build an archive that opens from `file://` with no server.

They point at the plan rather than restating it: the plan is the spec, the skills are the practice.

## License

**GPL-3.0-or-later** (`SPDX-License-Identifier: GPL-3.0-or-later`), see [LICENSE](LICENSE). The Python
tree reuses [`wa-crypt-tools`](https://github.com/ElDavoo/wa-crypt-tools) for `crypt15` decryption,
which is GPL-3.0; the Rust tree implements the same format itself, with the derivation pinned by a test
to the value that reference implementation computes.
