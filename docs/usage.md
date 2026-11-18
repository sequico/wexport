# Using wexport

What the person with the phone does, in order. Android on the phone. The release ships a Linux x86-64
binary; the same source builds on macOS and on Windows, and needs `ffmpeg` there too.

**wexport opens one kind of file: an Android WhatsApp backup protected by the 64-digit encryption
key.** Not by a password, not by a passkey, and never an iPhone backup. If your backup is protected
another way, no flag, setting or later step changes that — the key simply is not on your phone.

| The backup you have | Can wexport open it? |
|---|---|
| Android, end-to-end encrypted with the **64-digit key** — `Databases/msgstore.db.crypt15` | **yes** |
| Android, protected by a **password** | no: WhatsApp keeps the key on its servers |
| Android, protected by a **passkey** | no: same, the key never touches the file |
| Android with end-to-end encryption **off** — `msgstore.db.crypt14` | no: the file is not encrypted with a key you hold |
| Android on the old **crypt12** chain | no: that chain needs a password, not a key |
| iPhone / iCloud | no: a different format entirely |

The rest of this page is the path from "I have an Android phone" to "I have my chats as folders". The
first four steps happen on the phone and are the same whatever computer you use.

Time: 20–40 minutes, most of it waiting for the backup and for the file copy.
Needed: the phone, a USB cable, a computer with enough free disk (the copied backup plus the archives
can be tens of gigabytes), and this binary. `ffmpeg` and `ffprobe` must be on the computer's `PATH`;
`tesseract` only if you hand wexport a screenshot of the key instead of its digits.

---

## 1. Turn on end-to-end encrypted backup with the 64-digit key (once)

WhatsApp → **Settings → Chats → Chat backup → End-to-end encrypted backup**.

![The Chat backup screen](figures/chat-backup.svg)

*The Chat backup screen. **1** — end-to-end encrypted backup. **2** — back up now.*

WhatsApp opens its protection screen and offers a **passkey** first:

![The passkey offer and More options](figures/key-choice.svg)

*Turning it on. **1** — `More options`. **2** — the entry that gives you a key instead of a passkey.*

Tap **More options**, then **Use 64-digit encryption key instead**, then **Create**. Never choose
`Use password` and never accept the passkey: with either of those, WhatsApp keeps the key on its own
servers, and the file in your phone's folder is not something anyone — including you — can open
offline.

WhatsApp's own steps, in its own words:
[About end-to-end encrypted backup](https://faq.whatsapp.com/490592613091019?cms_platform=android) ·
[How to back up your chat history](https://faq.whatsapp.com/481135090640375?cms_platform=android).

## 2. Save the key now, because it is shown once

![The 64-digit encryption key screen](figures/key.svg)

*The key screen: 16 groups of 4 hex digits. **3** — the digits, shown once.*

The key is 64 hexadecimal characters: 16 groups of 4, usually on four lines. It is 32 bytes, and it is
the only thing that opens the backup.

- **Write it down or screenshot it before you leave this screen.** WhatsApp shows it once, when the
  key is created, and cannot show it again.
- Keep the copy **outside the phone as well**: a screenshot in a photo library that syncs elsewhere, a
  password manager, a printed sheet, another computer. A screenshot that exists only on the phone is
  lost with the phone.
- If the key is lost, the backups become unreadable — the ones on the phone included. WhatsApp cannot
  reset it, send a copy, or restore the backup for you, and after five wrong entries the screen locks
  for a while
  ([WhatsApp](https://faq.whatsapp.com/639067727894647?cms_platform=android)).

**Already using a password or a passkey?** Turn end-to-end encryption off and on again, choose the
64-digit key this time, and then make a **new full backup** (step 3). Backups made before that stay
closed forever; the new full backup is the one wexport will open.

## 3. Make a full backup

Same screen (**Figure 1**), tap **Back up now**, and wait until WhatsApp says the backup finished.

With end-to-end encryption on, the backup file WhatsApp keeps in the phone's folder is
`msgstore.db.crypt15`, encrypted with that key. **Back up now** is what makes it complete and fresh;
the daily files beside it are only the changes made since.

A cloud copy is not what wexport reads. It reads the folder on the phone, which you copy in the next
step — so the archive can be built with the phone in airplane mode and no Google account involved.

## 4. Copy the folder to the computer

Connect the phone by USB, allow file transfer (MTP), and copy this folder **with its structure**:

```
Android/media/com.whatsapp/WhatsApp/          # WhatsApp Business: Android/media/com.whatsapp.w4b/WhatsApp Business/
├── Databases/     ← always: it holds msgstore.db.crypt15
├── Backups/       ← recommended: wa.db, which carries the contact names
└── Media/         ← the attachments themselves, usually most of the gigabytes
```

On macOS, use **OpenMTP** or **Android File Transfer**: `Android/media` is hidden in the Finder. On
Linux, the file manager over MTP works, or `adb` with the phone in developer mode:

```sh
adb pull /storage/emulated/0/Android/media/com.whatsapp/WhatsApp ./phone-copy
```

Put it anywhere, for example `~/whatsapp/phone-copy`. Three checks before going on:

1. `Databases/` contains a file named exactly `msgstore.db.crypt15`;
2. `Media/` is not empty and holds folders like `WhatsApp Images` and `WhatsApp Voice Notes`;
3. the size looks like your real archive — gigabytes, not megabytes.

If `Databases/` holds only `msgstore.db.crypt14` or `msgstore.db.crypt12`, end-to-end encryption with
the key is not on, or the backup was never redone after turning it on: go back to steps 1 and 3.

## 5. Run wexport

```sh
wexport ~/whatsapp/phone-copy          # list every chat with a code, newest first
wexport ~/whatsapp/phone-copy -b 2     # build the chat with code 2
wexport ~/whatsapp/phone-copy -b all   # build every chat
```

The first command prints the list and stops. Read the code beside the chat you want, then build it;
`-l 10 size` shows the ten heaviest, `-r` reverses the order, `-o` sends the archives somewhere else
than `<folder>/output/`. `wexport -h` prints every flag and the exit codes.

The key reaches wexport as the 64 digits (`-k 0123…`), as a screenshot of them (`-k key.png`, needs
`tesseract`), or as the `encrypted_backup.key` file from a rooted phone (`-k encrypted_backup.key`).
With no `-k`, wexport looks beside the backup (and in the current folder) for `key.png` or another
image, `key.txt`, or `encrypted_backup.key`, and otherwise asks for the key on the terminal.

Archives land in `<folder>/output/`: one folder per chat, plus an `index.html` that lists them. When
the build ends, wexport opens that page in the desktop browser if there is one, and prints how many
messages and media files each archive holds.

## 6. What can go wrong

| Message | Meaning | What to do |
|---|---|---|
| `only crypt14 backups in …` | end-to-end encryption was off when the backup was made | turn it on with the 64-digit key (step 1), back up now, copy again |
| `only crypt12 backups in …` | the old chain, which needs a password and has no end-to-end key | turn on the 64-digit key, back up now, copy again |
| `was made with a password` / `with a passkey` | the key is on WhatsApp's servers, not in the file | re-enable with the 64-digit key and make a new full backup; the old files stay closed |
| `the key does not open this backup` | wrong key, or the backup predates the current key | use the key from the screenshot; if the key was regenerated, build from a backup made after that |
| `ffmpeg not found` / `tesseract not found` | a dependency is missing, and wexport refuses to produce a half-broken archive | run the install command the message prints, then the same command again |
| `media missing` in the build summary | that file is not in the copied `Media/` — deleted on the phone, or never downloaded | nothing; the archive shows a placeholder |
| contact names show as numbers | `Backups/wa.db` was not copied | copy `Backups/` and rebuild |

## 7. Afterwards

Each chat folder opens by double-clicking its `index.html`: no server, no wexport, no network.
Everything it needs is inside it — messages, images, video, voice notes with their waveform. Sharing
one chat means zipping that one folder, which is why a folder with video can be large.

Every message shows its full date and time with the UTC offset — `2026-11-17 14:32:05 +01:00` — with a
day separator above it, so the archive still says exactly when something was said in ten years. Those
strings are written at build time and never recomputed by the browser.

Keep the key and the chat folders together but separately stored: the folders need no key at all to be
read, and the key without the backup opens nothing.

---

WhatsApp pages this flow is taken from, all of them current at the time of writing:

- [About end-to-end encrypted backup](https://faq.whatsapp.com/490592613091019?cms_platform=android) —
  the three protections, and that WhatsApp cannot recover any of them.
- [How to back up your chat history](https://faq.whatsapp.com/481135090640375?cms_platform=android) —
  the menu path and `Back up now`.
- [Can't remember password for encrypted backup](https://faq.whatsapp.com/639067727894647?cms_platform=android) —
  the five attempts, and what "turn it off and on again" costs.

The figures on this page are drawn from those steps, not taken from WhatsApp: the wording of the
buttons is WhatsApp's, the layout is ours.
