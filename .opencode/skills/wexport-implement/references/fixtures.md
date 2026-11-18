# Synthetic fixtures

No real WhatsApp data ever enters this repository. Every test runs on a backup this project builds
itself, from a `msgstore.db` we also build ourselves.

## Shape

```
tests/fixtures/
├── synthetic/
│   ├── schema.sql          # the WhatsApp tables we read, minimal columns
│   ├── data.sql            # one row per message kind, plus reactions, quotes, edits, revokes, calls
│   ├── media/              # tiny real files, a few KB total
│   └── build.py            # sqlite3 → msgstore.db → msgstore.db.crypt15 with a fixed test key
└── real/                   # gitignored; supplied through WEXPORT_REAL_BACKUP
```

The test key is a fixed 64-hex-digit constant in `build.py`. It is not a secret and it is not a
backup of anything: it exists so a test can decrypt what the fixture build encrypted.

## Building a `crypt15`

Use the pinned `wa-crypt-tools` library, not a shelled-out command, so the fixture cannot drift from
the version the product uses:

1. `sqlite3` from the stdlib applies `schema.sql` and `data.sql` to `msgstore.db`.
2. The library's key factory turns the fixed hex key into a key object; the database factory parses a
   header and encrypts the payload.
3. Confirm the exact encrypt call by reading the installed package (site-packages of
   `wa-crypt-tools`). Do not guess an API and do not copy a command line from a blog post: the entry
   points are `KeyFactory` and `DatabaseFactory`, and everything else is internal.

## What the fixture must cover

- one message per `kind` in the §4 vocabulary, including `deleted` and `system`;
- a quoted reply, an edited message, a revoked message, a reaction and a read receipt;
- a media row whose file exists and one whose file does not (the `missing` state is normal data);
- a group chat and a one-to-one chat;
- a `call_log` row;
- two chunks' worth of messages, so the month split is exercised;
- one incremental file, so the merge path has a delta pair to replay.

## Media fixtures

Commit the tiny real files (a 1x1 PNG, a 120-byte WebP sticker, a fraction-of-a-second Opus voice
note, a minimal MP4). A few KB in git buys tests that need no external binary.

`build.py` is a test tool, not part of the product: it is the only Python left in the repository, it
runs only when the fixture is regenerated, and the crate never calls it. Image derivatives are
produced with `ffmpeg`, which the product requires anyway, so the only external tools a test run needs
are the same two the product needs. Unit tests that must not depend on a binary work on bytes (the
waveform peaks from raw PCM, chunk splitting, escaping), and one integration test runs the real
`ffmpeg`/`ffprobe` path on the fixture.

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
