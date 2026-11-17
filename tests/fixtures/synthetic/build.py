"""Build the synthetic phone folder the tests run on.

Everything here is invented: a schemaless-of-real-data msgstore, tiny media generated locally, and
a fixed test key. No real WhatsApp data enters the repository.
"""

from __future__ import annotations

import sqlite3
import subprocess
import zlib
from pathlib import Path

KEY_HEX = "0123456789abcdef" * 4  # 64 digits, not a secret: it only opens this fixture
HERE = Path(__file__).parent
PHONE = HERE / "phone"

CONTACTS_SCHEMA = """
CREATE TABLE wa_contacts (_id INTEGER PRIMARY KEY, jid TEXT, display_name TEXT, wa_name TEXT);
INSERT INTO wa_contacts (_id, jid, display_name, wa_name) VALUES
    (1, '393331234567@s.whatsapp.net', 'Alice', 'Alice'),
    (2, '393339999999@s.whatsapp.net', 'Bob', 'Bob');
"""


def build(dest: Path = PHONE) -> Path:
    """Create `dest` as a copy of a phone's WhatsApp folder. Idempotent."""
    media = dest / "Media"
    (media / "WhatsApp Images").mkdir(parents=True, exist_ok=True)
    (media / "WhatsApp Voice Notes").mkdir(parents=True, exist_ok=True)
    (media / "WhatsApp Video").mkdir(parents=True, exist_ok=True)
    (media / "WhatsApp Documents").mkdir(parents=True, exist_ok=True)
    (media / "WhatsApp Stickers").mkdir(parents=True, exist_ok=True)
    (media / "WhatsApp Animated Gifs").mkdir(parents=True, exist_ok=True)

    _png(media / "WhatsApp Images/IMG-20261201-WA0001.png", (16, 16))
    _webp(media / "WhatsApp Stickers/sticker.webp")
    _write(media / "WhatsApp Documents/notes.txt", b"fixture document\n")
    if not (media / "WhatsApp Voice Notes/PTT-20261201-WA0002.opus").exists():
        _ffmpeg(
            "-f", "lavfi", "-i", "sine=frequency=440:duration=0.3", "-c:a", "libopus", "-b:a", "8k",
            str(media / "WhatsApp Voice Notes/PTT-20261201-WA0002.opus"),
        )
    if not (media / "WhatsApp Video/VID-20261201-WA0003.mp4").exists():
        _ffmpeg(
            "-f", "lavfi", "-i", "color=c=gray:s=16x16:d=1", "-pix_fmt", "yuv420p",
            "-movflags", "+faststart",
            str(media / "WhatsApp Video/VID-20261201-WA0003.mp4"),
        )
    if not (media / "WhatsApp Animated Gifs/GIF-20261201-WA0004.mp4").exists():
        _ffmpeg(
            "-f", "lavfi", "-i", "color=c=red:s=16x16:d=1", "-pix_fmt", "yuv420p",
            str(media / "WhatsApp Animated Gifs/GIF-20261201-WA0004.mp4"),
        )

    (dest / "Databases").mkdir(exist_ok=True)
    (dest / "Backups").mkdir(exist_ok=True)

    msgstore = dest / "Databases" / "msgstore.db"
    with sqlite3.connect(msgstore) as con:
        con.executescript((HERE / "schema.sql").read_text(encoding="utf-8"))
        con.executescript((HERE / "data.sql").read_text(encoding="utf-8"))
    (dest / "Databases" / "msgstore.db.crypt15").write_bytes(_encrypt(msgstore.read_bytes()))
    msgstore.unlink()

    contacts = dest / "Backups" / "wa.db"
    with sqlite3.connect(contacts) as con:
        con.executescript(CONTACTS_SCHEMA)
    (dest / "Backups" / "wa.db.crypt15").write_bytes(_encrypt(contacts.read_bytes()))
    contacts.unlink()
    return dest


def _encrypt(data: bytes) -> bytes:
    from wa_crypt_tools import Database15, KeyFactory, Props

    key = KeyFactory.from_hex(KEY_HEX)
    return Database15().encrypt(key, Props(), zlib.compress(data, 9))


def _png(path: Path, size: tuple[int, int]) -> None:
    if path.exists():
        return
    from PIL import Image

    Image.new("RGB", size, (90, 140, 200)).save(path, "PNG")


def _webp(path: Path) -> None:
    if path.exists():
        return
    from PIL import Image

    Image.new("RGBA", (12, 12), (255, 200, 0, 255)).save(path, "WEBP")


def _write(path: Path, data: bytes) -> None:
    path.write_bytes(data)


def _ffmpeg(*args: str) -> None:
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", *args], check=True)


if __name__ == "__main__":
    built = build()
    print(f"fixture at {built}")
