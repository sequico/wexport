# Security

wexport reads decrypted copies of private conversations and writes readable archives, so the
interesting failures are about data exposure and about parsing hostile files.

## Reporting

Write to the maintainers privately first — `info@sequi.company` — and give a reproduction if you can.
Please do not open a public issue for a vulnerability before it is fixed.

## In scope

- A crafted `msgstore.db.crypt15` (or incremental) that makes the tool panic, hang or write outside
  the destination folder.
- A path in the database (`message_media.file_path`) that escapes the copied folder.
- A page in the generated archive that runs script from message text, a filename or a media file.
- Key material or conversation content leaving the machine, or ending up anywhere but the archive the
  user asked for.

## Out of scope, by design

- **The archive is plaintext.** Anyone with the folder can read it; that is what it is for, and the
  build summary says so.
- The tool is offline: it has no network code, so there is nothing to attack remotely.
- If the attacker already has the 64-digit key, the backup is theirs to read.

## What the project does about it

The parsers of untrusted bytes (header, payload, key text) have fuzz targets, the media path is
resolved and confined to the copied folder, message text is escaped so it cannot close a script tag,
and `cargo deny`/`cargo audit` keep an eye on the dependency tree.
