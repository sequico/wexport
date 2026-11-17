# wexport

Turn an Android WhatsApp backup into a local, scrollable archive of your chats — every message,
every image, video and voice note, with every timestamp frozen at build time so the archive still
says exactly when something happened in ten years.

Each chat becomes one folder that opens by double-clicking `index.html`: no server, no wexport, no
network. A folder can be zipped and given to someone without exposing the other chats.

## What it does

```
phone ──► folder you copy by USB ──► wexport ──► one folder per chat ──► double-click index.html
                                     + 64-digit key
```

`wexport` reads `msgstore.db.crypt15` (and the daily incrementals), decrypts it with the 64-digit key
WhatsApp showed you when you enabled end-to-end encrypted backups, joins the messages with the
`Media/` folder you copied, and writes the archive. It runs entirely offline and uploads nothing.

## Usage

```sh
uv sync
uv run wexport check ~/whatsapp/phone-copy --key <64 hex digits> --chats --sort last
uv run wexport build ~/whatsapp/phone-copy --key <64 hex digits> --out ~/whatsapp/archives \
  --chat Family --index
xdg-open ~/whatsapp/archives/2026-11-17-family-4f2a/index.html
```

`check --chats` lists every chat with its last event, message count and media weight, ordered by
`--sort last|messages|size|name` (and `--reverse`), so you pick from real names instead of guessing.
`--match <text>` narrows by name or JID and `--limit N` caps the output — the default shows 50 of
them, `--limit 0` shows all, which matters on a phone with a thousand conversations.
Without `--chat`, `build` writes one archive per chat.

The key can be 64 hex digits (spaces and a 4×4 grid are fine), a screenshot of them, or the phone's
`encrypted_backup.key` on a rooted device. The screenshot path needs the OCR extra and the `tesseract`
binary: `uv sync --extra ocr` plus `apt install tesseract-ocr` (or the equivalent). `ffmpeg` and
`ffprobe` are required and missing ones abort at startup with the install command for the platform.
See [the user flow](docs/user-flow.md).

## Scope

- **Android only**, backups in `crypt15` format (end-to-end encrypted, 64-digit key).
- The user copies the phone folder themselves; the tool never touches the device.
- Not supported, deliberately: iPhone, Google Drive download, password-based or passkey-based
  backups, legacy `crypt12`/`crypt14`, re-encrypting anything back.

## Status

v0.1.0, working end to end:

- implemented — `check` and `build`; crypt15 decrypt and incremental merge; msgstore adapter for the
  tables in `docs/android-backup-formats.md`; content-addressed media with thumbnails, posters and
  waveform peaks; month chunks; the `file://` viewer with virtualised rendering, search and
  jump-to-date; the plaintext warning and the `README.txt` in each archive.
- checks — 81 tests collected, all green except the one that skips by design without a real backup;
  `ruff check` clean.
  The viewer was verified in Chromium over `http` with Playwright; **double-clicking from `file://`
  in Chrome, Firefox and Edge still needs a manual pass** (the plan's M3 criterion), because the
  automation available here refuses `file://` URLs.
- packaging — `uv tool install wexport` works today; the Nuitka one-file Linux binary is built from
  `src/wexport/__main__.py` into `dist/wexport` (≈20 MB, no Python needed) and builds the fixture
  end to end. The Windows build is the same command on a Windows machine, still to be run.
- known limits — a poll renders as its question text, without options or votes; incremental merge is
  row-level, so a message deleted only on the phone stays in the archive.

## Packaging

```sh
uv sync --group packaging
.venv/bin/nuitka --onefile --assume-yes-for-downloads --output-dir=dist --output-filename=wexport \
  --include-package=wa_crypt_tools --include-package-data=wexport --include-package=PIL \
  src/wexport/__main__.py
```

`__main__.py`, not `cli.py`: the packaged entry point has no parent package, so relative imports
would fail. `patchelf` is required on Linux; the viewer assets are pulled in by
`--include-package-data=wexport`.

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

**GPL-3.0-or-later** (`SPDX-License-Identifier: GPL-3.0-or-later`), see [LICENSE](LICENSE). wexport
reuses [`wa-crypt-tools`](https://github.com/ElDavoo/wa-crypt-tools) for `crypt15` decryption, which is
GPL-3.0.
