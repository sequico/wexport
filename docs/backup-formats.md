# Backup formats (reference)

What is inside the files a phone leaves behind — Android's encrypted container and an iPhone's Finder
backup — which key opens what, and where the tool's assumptions come from. Facts are from the sources
listed at the end; anything derived rather than observed is marked `unverified`. Tested reference
versions: Android WhatsApp **2.26.34.7** with `wa-crypt-tools` (0.2.0 and its `main` branch, which adds
the OCR of the key screenshot and the `wagui` interface), and the iPhone layout of iOS 10 and later.

---

## 1. Where the files are on the phone

```
/storage/emulated/0/Android/media/com.whatsapp/WhatsApp/
├── Databases/
│   ├── msgstore.db.crypt15                    # the backup, full
│   ├── msgstore-YYYY-MM-DD.1.db.crypt15       # rolling daily copies
│   └── msgstore-increment-*.db.crypt15        # incremental deltas (E2EE backups)
├── Backups/
│   ├── wa.db                                  # contacts — also crypt15
│   ├── chatsettingsbackup.db
│   ├── status_backup.db
│   └── stickers_db.bak
└── Media/
    ├── WhatsApp Images/  WhatsApp Video/  WhatsApp Audio/  WhatsApp Voice Notes/
    ├── WhatsApp Documents/  WhatsApp Animated Gifs/  WhatsApp Stickers/
    └── WhatsApp Profile Photos/  (and Sent/ Received/ Private/ children)
```

Older devices use `/sdcard/WhatsApp/` instead of the `Android/media/` path. Business uses
`com.whatsapp.w4b` and `WhatsApp Business/`. Copying any of this over MTP or `adb pull` needs no
root.

A phone that has had end-to-end encryption turned on recently keeps **both chains** in `Databases/`:
`msgstore.db.crypt15` is the current one and the only usable one; the older
`msgstore-YYYY-MM-DD.1.db.crypt14` and `msgstore-increment-*-….crypt14` files belong to the previous
chain and can never be mixed with it — they need the key file from app-private storage, which needs
root. The listing names them as ignored instead of silently skipping them.

---

## 2. `crypt15` — the container

```
┌───────────────────────────────┐
│ protobuf header               │  IV (12 or 16 bytes), backup version, key_type,
│                               │  app version, JID suffix, feature/migration flags
├───────────────────────────────┤
│ AES-256-GCM ciphertext        │  authenticated: a wrong key fails the tag, not silently
├───────────────────────────────┤
│ payload                       │  zlib/gzip → SQLite (single file)
│                               │  …or a ZIP (multi-file / incremental backups)
└───────────────────────────────┘
```

- The payload is compressed before encryption; level and header vary by app version (WhatsApp moved
  from level 1 to level 9, and current backups carry an extra varint the 2023 schema did not model).
- A backup that is not a `msgstore` (`wa.db`, `chatsettingsbackup.db`, stickers) uses the same
  container with a different payload.
- The header is enough to answer "can this be opened?" before decrypting a byte — this is what the
  discovery step relies on.

### `key_type` — the decisive field

| Value | Meaning | Openable offline |
|---|---|---|
| `3` / `E2EE_ENCRYPTION_KEY` | user chose the 64-digit key | **yes**, if the user has the key |
| `E2EE_PASSWORD` | user chose a password; key wrapped by the HSM | no — password only unlocks key retrieval from WhatsApp's servers |
| `E2EE_PASSKEY` | key wrapped by a passkey; added in the 2.26 schema | no (unverified: no public tooling) |
| `E2EE_DEPRECATED` | pre-E2EE server-held key (some tools print it as `HSM_CONTROLLED`) | no |

---

## 3. Where the key lives

| Backup type | Key material | Without root |
|---|---|---|
| `crypt15`, 64-digit key | `/data/data/com.whatsapp/files/encrypted_backup.key` — the root key is the last 32 bytes (64 hex digits) | the key must have been recorded by the user; wexport reads it from a screenshot (tesseract) or from the typed digits |
| `crypt15`, password | the password is **not** the key; the key is fetched from the HSM after OPAQUE login and cached in the same file | not recoverable |
| `crypt15`, passkey | key wrapped by the passkey | not recoverable |
| `crypt14` / `crypt12` | `/data/data/com.whatsapp/files/key` (Java-serialised: AES key + IV + salt) | not recoverable |

Consequences for wexport:

- Only the first row is a supported input. Everything else is recognised and refused before any work
  starts.
- Enabling E2EE does not re-encrypt existing backups. Only a **full backup performed after** the key
  was created is openable; a stale `crypt14` left in `Databases/` is not.
- A local `crypt14` is *worse* than a `crypt15` for a non-root user: `crypt15` at least lets the key
  live outside the phone.

---

## 4. The database

`msgstore.db` is plain SQLite once decrypted. Tables wexport reads, or replays from an incremental:

| Table | Use |
|---|---|
| `message` | one row per message: timestamp, `from_me`, `sender_jid_row_id` (a `jid` row), `text_data`, type, status, `quoted_row_id`, `edit_version` |
| `message_media` | media rows: `file_path`, mime, size, dimensions, duration, caption |
| `message_add_on` + `message_add_on_reaction` | reactions and other add-ons attached to a message |
| `message_system` | the system notices WhatsApp writes into a conversation |
| `message_vcard` | shared contacts |
| `message_revoked` | deleted-by-sender messages, still present |
| `call_log` | voice/video call rows, rendered as timeline entries |
| `chat` | conversations, group flag, subject |
| `jid` | phone number / `@lid` mapping, and the sender of a group message |
| `receipt_user`, `group_participant_user` | replayed from incrementals; the archive does not show them |

Cautions:

- Column and table names change between WhatsApp versions; 2.26 renamed protobuf fields *inside* the
  header, and the message-type numbering grows with every feature (polls, view-once, albums, events,
  edits, reactions, channels, communities). All of it is confined to one adapter module.
- `message_media.file_path` is relative to the WhatsApp root (`Media/WhatsApp Images/…`), not an
  absolute path: the user's copied folder must preserve the `Media/` subtree.
- Media that WhatsApp has deleted locally is referenced but absent — a normal, expected state.

---

## 5. Incrementals and why the media may look thin

- Full backup: roughly every 5–7 days, or when the user triggers one.
- Between them, `msgstore-increment-*.db.crypt15` carries deltas — a multi-file backup, hence the
  ZIP payload.
- Daily incrementals save message text and metadata only: link previews and media thumbnails can be
  missing from them until the next full backup. Nothing can fetch them offline.

---

## 6. Tools that already implement this

| Project | Does | License |
|---|---|---|
| [ElDavoo/wa-crypt-tools](https://github.com/ElDavoo/wa-crypt-tools) | crypt12/14/15 decrypt + encrypt, header parsing (`wainfo`), key from screenshot (OCR), GUI (`wagui`), tested on real 2.26.34.7 backups | GPL-3.0-or-later |
| [giacomoferretti/whatsapp-backup-downloader-decryptor](https://github.com/giacomoferretti/whatsapp-backup-downloader-decryptor) | Google Drive `mcrypt1` download + decrypt | Apache-2.0 |
| [B16f00t/whapa](https://github.com/B16f00t/whapa) 2.00 (Jul 2026) | Android + iOS parsing, Drive extractor, HTML report | see repo |
| [auanasgheps/whatsapp-backup-tools](https://github.com/auanasgheps/whatsapp-backup-tools) | media archive + Flask viewer, incremental merge | GPL-3.0 |
| [KnugiHK/WhatsApp-Chat-Exporter](https://github.com/KnugiHK/WhatsApp-Chat-Exporter) 0.13.1 | HTML/JSON export, Android + iOS | MIT |

---

## 7. The iPhone: a local backup

A Finder/iTunes backup is a folder of hashed files and one index. Nothing in it is encrypted unless
the user asked for it, and then everything is.

```
<device-id>/
├── Manifest.db          # SQLite: Files(fileID, domain, relativePath, flags, file)
├── Manifest.plist       # binary plist: IsEncrypted, device name, last backup date
├── 00/ ... ff/          # the files, named by fileID
└── Info.plist
```

- **One file, one row.** `Files.fileID` is the SHA-1 of `domain-relativePath` for iOS 10 and later,
  and the file lives at `<root>/<first two hex digits>/<fileID>`. Before iOS 10 the file sat at
  `domain/relativePath` with no hash; wexport reads the hashed layout only and says so.
- **`Manifest.plist` answers the encryption question.** With "Encrypt local backup" on, `IsEncrypted`
  is true and even `Manifest.db` is encrypted; there is no offline way in without Apple's password
  keybag, so wexport refuses the folder as a wrong backup type.
- **WhatsApp's domain is `AppDomainGroup-group.net.whatsapp.WhatsApp.shared`.** Inside it:
  `ChatStorage.sqlite` with its `-wal` and `-shm`, and the media under `Message/Media/…` (a few
  versions keep some under `Library/Media`). A row's media path is relative to that group, so the
  lookup goes through `Manifest.db` to the hashed file.

`ChatStorage.sqlite` is Core Data, not WhatsApp's own schema:

| Table | Use |
|---|---|
| `ZWACHATSESSION` | one row per chat: `ZCONTACTJID`, `ZPARTNERNAME` |
| `ZWAMESSAGE` | `ZCHATSESSION`, `ZMESSAGEDATE`, `ZISFROMME`, `ZTEXT`, `ZFROMJID`, `ZMEDIAITEM`, `ZDELETED`, `ZEDITED`, `ZCALLTYPE` |
| `ZWAMEDIAITEM` | `ZMEDIALOCALPATH`, `ZTHUMBNAILLOCALPATH`, `ZFILESIZE`, `ZMIMETYPE`, `ZLATITUDE`, `ZLONGITUDE`, `ZVCARDNAME` |
| `ZWAGROUPMEMBER` | the roster of a group: `ZCHATSESSION`, `ZMEMBERJID`, `ZCONTACTNAME` |

Cautions:

- **Timestamps are seconds since 2001-01-01**, not since 1970. One conversion in the adapter, pinned
  by a test.
- **The newest messages are in the `-wal`.** A build that copies only `ChatStorage.sqlite` loses the
  last days of every conversation; all three files travel together and are opened so SQLite replays
  the log.
- **The schema moves between iOS releases**, and WhatsApp's column names are not documented. They are
  discovered by probing, and a schema that does not answer fails by naming what it could not find.
- **Media deleted on the phone stays referenced** by its row, with the thumbnail file often still
  there: the archive shows the preview and marks the file as not copied.
- **No key is involved.** The 64-digit key on an iPhone protects the cloud copy, not the local backup.

---

## Sources

- ElDavoo/wa-crypt-tools — README, CHANGELOG (Nov 2026: Python 3.11+, protobuf edition 2026,
  `key_type_new`, OCR of the key screenshot, tested against 2.26.34.7 backups), `constants.py`.
- WhatsApp Security Whitepaper, *End-to-End Encrypted Backups*, v2 (1 May 2026): OPAQUE registration
  and recovery, 256-bit key, password never leaves the client unencrypted.
- Davies et al., *Security Analysis of the WhatsApp End-to-End Encrypted Backup Protocol*
  (eprint 2023/843).
- CEUR Vol-4223 paper 1, *Analysis of WhatsApp data-at-rest security and encryption* (2026):
  crypt5/8/12/14/15 lineage.
- giacomoferretti/wabdd README: key file layout (`[-32:]`), `mcrypt1`, `oauth_token` cookie.
- auanasgheps/whatsapp-backup-tools `docs/archiver/setup-android.md`: paths, incremental naming,
  E2EE setup with the 64-digit option.
- chathoarding.app, *Crypt15 explained* (18 May 2026): header fields, payload compression, table
  list.
- WhatsApp FAQ: end-to-end encrypted backup, turning it off.
- Apple Platform Security, *Backups* — the local backup layout, `Manifest.db`, and what turning on
  the backup password protects.
- mvt-project/mvt — the iOS backup parser (`Manifest.db`, the hashed `fileID`, the app-group
  domains), read as a cross-check of the layout above.

---

wexport is written and maintained by [Sequi Company](https://sequi.company) — `info@sequi.company`.
