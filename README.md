# wexport

Turn an Android WhatsApp backup into a local, scrollable archive of your chats — every message,
every image, video and voice note, rendered as a site you open in a browser.

Private repository. Planning stage: no code yet.

## What it does

```
phone ──► folder you copy by USB ──► wexport ──► site/ (HTML + media) ──► browser
                                     + 64-digit key
```

`wexport` reads `msgstore.db.crypt15` (and the daily incrementals), decrypts it with the 64-digit
key WhatsApp showed you when you enabled end-to-end encrypted backups, joins the messages with the
`Media/` folder you copied, and writes a self-contained site — a virtualised chat view, thumbnails,
video posters, voice-note waveforms and full-text search — that `wexport serve` opens in your
browser.

It runs entirely offline. Nothing is uploaded anywhere.

## Scope

- **Android only**, backups in `crypt15` format (end-to-end encrypted, 64-digit key).
- The user copies the phone folder themselves; the tool never touches the device.
- The user supplies the key as 64 hex digits or as a screenshot of it.

Not supported, deliberately: iPhone, Google Drive download, password-based or passkey-based
backups, legacy `crypt12`/`crypt14`, re-encrypting anything back.

## Documentation

- [Implementation plan](docs/implementation-plan.md) — architecture, repository layout, milestones.
- [Android backup formats](docs/android-backup-formats.md) — what is inside the files, and which key
  opens which format.
- [User flow](docs/user-flow.md) — what the person with the phone has to do, step by step.

## Status

Planning stage: no code yet. Milestone M0 not started — see the plan for milestones and acceptance
criteria. Owner decisions taken: GPL-3.0-or-later, transcoding opt-in.

## License

**GPL-3.0-or-later** (`SPDX-License-Identifier: GPL-3.0-or-later`). wexport reuses
[`wa-crypt-tools`](https://github.com/ElDavoo/wa-crypt-tools) for `crypt15` decryption, which is
GPL-3.0; the full licence text is added with M0.
