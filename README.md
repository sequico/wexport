# wexport

Turn an Android WhatsApp backup into a local, scrollable archive of your chats — every message,
every image, video and voice note, rendered as a site you open in a browser.

## What it does

```
phone ──► folder you copy by USB ──► wexport ──► one folder per chat ──► double-click index.html
                                     + 64-digit key
```

`wexport` reads `msgstore.db.crypt15` (and the daily incrementals), decrypts it with the 64-digit
key WhatsApp showed you when you enabled end-to-end encrypted backups, joins the messages with the
`Media/` folder you copied, and writes one self-contained archive per chat — a virtualised timeline,
thumbnails, video posters, voice-note waveforms and search, with every message stamped in full:

```
Monday, 17 November 2026
14:32:05 +01:00   │ the message
```

It opens by double-clicking `index.html`: no server, no wexport, no install on the machine that reads
it, and the timestamps are frozen at build, so the archive still says exactly when something happened
years later. A chat folder can be zipped and given to someone without exposing the others.

It runs entirely offline. Nothing is uploaded anywhere.

## Scope

- **Android only**, backups in `crypt15` format (end-to-end encrypted, 64-digit key).
- The user copies the phone folder themselves; the tool never touches the device.
- The user supplies the key as 64 hex digits, as a screenshot of it, or as the key file on a rooted
  phone.

Not supported, deliberately: iPhone, Google Drive download, password-based or passkey-based
backups, legacy `crypt12`/`crypt14`, re-encrypting anything back.

## Documentation

- [Implementation plan](docs/implementation-plan.md) — architecture, repository layout, milestones.
- [Android backup formats](docs/android-backup-formats.md) — what is inside the files, and which key
  opens which format.
- [User flow](docs/user-flow.md) — what the person with the phone has to do, step by step.

## Working on it with an agent

Two skills live in `.opencode/skills/`, and they load only when relevant:

- `wexport-implement` — the procedure and the invariants for editing code here, plus the synthetic
  fixture recipe in `references/fixtures.md`.
- `offline-html-archive` — how to build an archive that opens from `file://` with no server.

They point at the plan rather than restating it: the plan is the spec, the skills are the practice.

## Status

Planning stage: no code yet. Milestone M0 not started — see the plan for milestones and acceptance
criteria. Owner decisions taken: GPL-3.0-or-later, transcode opt-in, one site per chat, English
throughout, plaintext archive with a warning, Nuitka one-file binaries for Linux and Windows. A Rust
core is planned for after v1, not now.

## License

**GPL-3.0-or-later** (`SPDX-License-Identifier: GPL-3.0-or-later`). wexport reuses
[`wa-crypt-tools`](https://github.com/ElDavoo/wa-crypt-tools) for `crypt15` decryption, which is
GPL-3.0; the full licence text is added with M0.
