# Synthetic fixtures

No real WhatsApp data ever enters this repository. Every test runs on a backup this project builds
itself, from a `msgstore.db` we also build ourselves.

## Shape

```
tests/fixtures/
├── synthetic/
│   ├── schema.sql          # the Android tables we read, minimal columns
│   ├── data.sql            # one row per message kind, plus reactions, quotes, edits, revokes, calls
│   ├── phone/              # the committed fixture: databases, media, contacts
│   ├── build.py            # sqlite3 → msgstore.db → msgstore.db.crypt15 with a fixed test key
│   └── ios/
│       ├── build.py        # a Finder backup: Manifest.db, Manifest.plist, hashed files, a live -wal
│       └── backup/         # the committed iPhone fixture
└── real/                   # gitignored; supplied through WEXPORT_REAL_BACKUP
```

The test key is a fixed 64-hex-digit constant in `build.py`. It is not a secret and it is not a
backup of anything: it exists so a test can decrypt what the fixture build encrypted.

## Building a `crypt15`

Use the pinned `wa-crypt-tools` library, not a shelled-out command, so the fixture cannot drift from
the version the product uses:

1. `sqlite3` from the stdlib applies `schema.sql` and `data.sql` to `msgstore.db`.
2. `KeyFactory.from_hex` turns the fixed hex key into a key object; `Database15(iv=…)` encrypts the
   zlib-compressed payload. The IV is **fixed**, because the fixture is committed and its bytes must
   not change from one generation to the next.
3. Install it from its repository (`pip install git+https://github.com/ElDavoo/wa-crypt-tools.git`):
   the PyPI release is an older layout whose exit is not compatible with the code above.

## What the fixture must cover

- one message per `kind` in the §4 vocabulary, including `deleted` and `system`;
- a quoted reply, an edited message, a revoked message, a reaction and a read receipt;
- a media row whose file exists and one whose file does not (the `missing` state is normal data);
- a `message_thumbnail` row for a video and one for a file that is gone, so the stored preview is
  covered on both paths;
- a group chat and a one-to-one chat;
- a `call_log` row;
- two chunks' worth of messages, so the month split is exercised;
- one incremental file, so the merge path has a delta pair to replay.
- on the iPhone side: two chats, one of them a group; a message that exists only in the `-wal`; a
  stored thumbnail; a media file that is not in the backup; a location, a vCard, a call and a deleted
  message.

## Media fixtures

Commit the tiny real files (a 16×16 PNG, a 12×12 WebP sticker, a fraction-of-a-second Opus voice
note, a one-second H.264 MP4). A few KB in git buys tests that run with no external tool at all.

`build.py` is a test tool, not part of the product: it runs only when the fixture is regenerated, and
the crate never calls it. The media it commits are generated once with `ffmpeg` on the machine of
whoever regenerates the fixture; the *test run* needs no binary. The same applies to the iPhone
fixture, which reuses those media files through `Manifest.db`.

## The iPhone fixture

`tests/fixtures/synthetic/ios/build.py` writes a Finder backup: `Manifest.db` with the `Files` table,
a binary `Manifest.plist`, the files under their two-hex-digit `fileID` folders, and a `ChatStorage`
database with a **live WAL** holding the last message. Reuse the Android fixture's media bytes for
the picture, the video and the voice note, so the two fixtures cannot drift apart.

## Golden files

`tests/fixtures/golden/` holds the chunk of one fixture chat, built under a fixed zone
(`Europe/Rome`), so the record format is pinned byte for byte.

To regenerate after a change that is *meant* to alter the format: delete the golden file, run
`cargo test --test fixture`, and the test writes the new file and fails once. Read the diff before
committing it; the commit says why the format moved. A diff you cannot explain is a bug.

## Assertions worth writing

- Decrypting the fixture reproduces the exact bytes of the `msgstore.db` it was built from.
- The adapter maps every kind to the expected value, and a renamed source column fails loudly
  instead of silently returning `NULL`.
- Emitting chunks twice produces byte-identical files (determinism), including row order.
- A message containing `</script>`, `\u2028` and an emoji survives the round trip into the chunk
  script.
- A `crypt14`, a password `crypt15` and a passkey `crypt15` each exit `3` before touching the output
  directory.

## Regenerating

`build.py` is deterministic and committed; run it only when the fixture shape changes, and commit the
regenerated `crypt15` in the same commit as the change that required it. If a fixture and a test
disagree, the fixture is what is wrong — unless the fixture is proving a real app behaviour, in which
case the docs say so.
