# Changelog

One line per release. The detailed history is `git log`.

## 1.0.0

- First release: `crypt15` backups (full and incrementals) into one self-contained folder per chat,
  with a `file://` viewer, frozen timestamps with explicit offsets, content-addressed media and
  thumbnails, and one binary per platform (Linux x86-64, Windows x86-64).
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
