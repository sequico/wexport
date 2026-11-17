# User flow (Android)

What the person with the phone does, in order, before touching wexport. Written to be read by that
person, not by the developer.

Time: 20–40 minutes, most of it waiting for the backup and the file copy.
Needed: the phone, a USB cable, a computer with enough free disk (backup + `Media/` can be tens of
GB).

---

## 1. Choose the key (once, irreversibly)

WhatsApp → **Settings → Chats → Chat Backup → End-to-end encrypted backup → Turn on**.

When asked how to protect it, choose **"Use 64-digit encryption key instead"**. Never the password,
never a passkey: with those, the key lives on WhatsApp's servers and the backup cannot be opened by
anyone except WhatsApp itself.

WhatsApp shows the key **once**: 64 hex digits, four rows of four groups. Take a screenshot and store
it outside the phone (photo library synced elsewhere, password manager, printed sheet).

> ⚠️ If the key is lost, the backups become unreadable — including the ones already on the phone.
> This is the whole point of end-to-end encryption, and it cannot be undone by anyone.

Already enabled with a password? Turn it off and on again choosing the 64-digit key. Backups made
before that stay closed; the new full backup is the one we will use.

## 2. Make a full backup

Same screen → **Back up now**. Wait for it to finish. Do not start step 3 until WhatsApp says the
backup is complete.

If the last backup was days ago and only incrementals exist, this step is what puts a complete
`msgstore.db.crypt15` in the folder.

## 3. Copy the folder to the computer

Connect the phone by USB, allow file transfer (MTP), and copy this folder preserving its structure:

```
Android/media/com.whatsapp/WhatsApp/          (WhatsApp Business: …/com.whatsapp.w4b/WhatsApp Business/)
├── Databases/     ← always
├── Backups/       ← recommended: contact names
└── Media/         ← the attachments themselves
```

On macOS, use **OpenMTP** or **Android File Transfer** — "Android/media" is hidden in the Finder
view. On Linux, MTP through your file manager, or `adb pull`:

```
adb pull /storage/emulated/0/Android/media/com.whatsapp/WhatsApp ./phone-copy
```

Place it wherever you like, for example `~/whatsapp/phone-copy`. Three checks before continuing:

- `Databases/` contains a file named exactly `msgstore.db.crypt15`;
- `Media/` is not empty and contains subfolders like `WhatsApp Images`, `WhatsApp Voice Notes`;
- the total size looks like your real archive (gigabytes, not megabytes).

If `Databases/` holds only `msgstore.db.crypt14`, end-to-end encryption is not enabled (or the
backup was never redone after enabling it): go back to step 1 and 2. wexport cannot open it.

## 4. Run wexport

```
wexport check ~/whatsapp/phone-copy --key key.png
wexport build ~/whatsapp/phone-copy --key key.png --out ~/whatsapp/site
wexport serve ~/whatsapp/site
```

Then open the printed `http://127.0.0.1:8765` address.

The key can be given three ways: the screenshot (`--key key.png`), the 64 digits copied from it
(`--key 0123…`), or the `encrypted_backup.key` file if the phone was rooted (`--key encrypted_backup.key`).

## 5. What can go wrong

| Message | Meaning | What to do |
|---|---|---|
| `backup is crypt14` | E2EE was off when the backup was made | enable E2EE with the 64-digit key, force a full backup, copy again |
| `backup needs the password / passkey` | the key is on WhatsApp's servers | re-enable E2EE with the 64-digit key and make a new backup; old backups stay closed |
| `key does not match this backup` | wrong key, or a new key was generated after this backup | use the key from the screenshot; if the key was regenerated, use a backup made after that |
| `media missing` in the build summary | the file is not in the copied `Media/` (deleted on the phone, or never downloaded) | nothing to do; the site shows a placeholder |
| contact names show as numbers | `Backups/wa.db` was not copied | copy it and rebuild |

## 6. Afterwards

`site/` has no external dependency: no network, no other files, no `node_modules`. It does need
`wexport serve` to open (a browser opening the HTML file directly cannot read the data file or seek in
a video), and it never phones home.

Keep the screenshot of the key and `site/` together but separately stored: the key alone is useless
without the backup, and the site is the readable copy that does not need the key at all.
