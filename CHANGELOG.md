# Changelog

One line per release. The detailed history is `git log`.

## Unreleased

- A usage page: the 64-digit key is the only key that works, and the phone steps — turning the backup
  on, saving the key, making a full backup, copying the folder — are drawn screen by screen.

## 1.0.0

- First release: `crypt15` backups (full and incrementals) into one self-contained folder per chat,
  with a `file://` viewer, frozen timestamps with explicit offsets, content-addressed media and
  thumbnails, and a single static binary.
- The command lists the chats with a code each; `-b` builds by code, `-l` orders and cuts the list,
  `-o` chooses where the archives go.
- Exit codes `0..5` separate unreadable input, a bad key, a wrong backup type, an incomplete build and
  a missing external tool.
