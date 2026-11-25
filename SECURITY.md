# Security

wexport reads decrypted copies of private conversations and writes readable archives, so the
interesting failures are about data exposure and about parsing hostile files.

## Reporting

Write to the maintainers privately first — `info@sequi.company` — and give a reproduction if you can.
Please do not open a public issue for a vulnerability before it is fixed. The same address reaches
[Sequi Company](https://sequi.company), who maintain wexport.

## In scope

- A crafted `msgstore.db.crypt15` (or incremental) that makes the tool panic, hang or write outside
  the destination folder.
- A crafted iPhone backup — `Manifest.db`, `Manifest.plist`, `ChatStorage.sqlite` or a media file —
  that does the same.
- A path in either database (`message_media.file_path`, `ZMEDIALOCALPATH`, a `Manifest.db`
  `relativePath`) that escapes the copied folder or the backup.
- A page in the generated archive that runs script from message text, a filename or a media file.
- A malformed image, H.264 stream or Ogg/Opus note that makes the media decoders (`image`,
  `openh264`, `libopus`) misbehave, since those are parsing surfaces like the rest.
- Key material or conversation content leaving the machine, or ending up anywhere but the archive the
  user asked for.

## Out of scope, by design

- **The archive is plaintext.** Anyone with the folder can read it; that is what it is for, and the
  build summary says so.
- The tool is offline: it has no network code, so there is nothing to attack remotely.
- If the attacker already has the 64-digit key, the backup is theirs to read.
- A backup the user hands over with the phone still unlocked and the app open: the threat model starts
  at a folder on disk.

## What the project does about it

The parsers of untrusted bytes that run in-process — the `crypt15` header, the zipped payload, the
key text — have fuzz targets; the schema adapters, the container readers and the media decoders are
exercised by the fixture suites and their own unit tests; every media path is resolved and confined to
the copied folder or the backup; the iPhone database is copied into a workspace that is owner-only and
removed at the end; message text is escaped so it cannot close a script tag; and `cargo deny` /
`cargo audit` keep an eye on the dependency tree, including the two compiled-in libraries.
