# Changelog

One line per release. The detailed history is `git log`.

## Unreleased

- iPhone local backups (Finder/iTunes, unencrypted) and an extracted `ChatStorage.sqlite` open beside
  Android's `crypt15`, with no key; an encrypted iPhone backup is refused by name.
- macOS universal (arm64 + x86-64) joins Linux and Windows: three archives per release, each holding
  the binary with its licences.
- `ffmpeg` and `ffprobe` are gone: images, posters, container metadata and voice-note waveforms are
  made in-process, so nothing has to be installed to build an archive.
- A video poster is the preview the phone already stored, and the first frame decoded with `openh264`
  when it stored none; a media file missing from the copy still shows its preview, marked as not
  copied.
- Image thumbnails are JPEG, `deriv/<sha>.jpg`.
- Five flags: `-j` (JSON lines), `-q` (quiet), `-d` (diagnostics), `-n` (dry run), `-c` (check).
- `-c` verifies the source — the database and every media file the rows name — before a build or on
  its own.

## 1.0.0

- First release: `crypt15` backups (full and incrementals) into one self-contained folder per chat,
  with a `file://` viewer, frozen timestamps with explicit offsets, content-addressed media and
  thumbnails, and one binary per platform (Linux x86-64, Windows x86-64), published as an archive
  named after the platform: `.tar.gz` for Linux, `.zip` for Windows.
- The command lists the chats with a code each; `-b` builds by code, `-l` orders and cuts the list,
  `-o` chooses where the archives go, `-t ZONE` renders every time in a named zone instead of the
  machine's and records the name.
- Exit codes `0..5` separate unreadable input, a bad key, a wrong backup type, an incomplete build and
  a missing external tool.
- A usage page: the 64-digit key is the only key that works, the phone steps — turning the backup on,
  saving the key, making a full backup, copying the folder — drawn screen by screen, and the flags with
  their syntax, their rules and the exit codes.
- The README opens with why the tool exists: WhatsApp's own export tangles on the long chats worth
  keeping, so wexport takes the whole backup instead, with no ceiling on the number of chats, the
  length of one chat, or the size of the archive.
