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

INCREMENTAL_SQL = """
-- A delta holds only what changed: one new message, and an edit of an existing one.
INSERT INTO message (_id, chat_row_id, from_me, key_id, timestamp, text_data, status,
                     remote_resource, quoted_row_id, edit_version)
VALUES
    (5, 2, 0, 'k5', CAST((julianday('2026-12-01T09:00:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'corrected by the delta', 13, '393339999999@s.whatsapp.net', NULL, 1),
    (17, 2, 1, 'k17', CAST((julianday('2026-12-03T08:00:00Z') - 2440587.5) * 86400000 AS INTEGER),
        'delta message', 13, NULL, NULL, 0);
"""


def _media(dest: Path) -> None:
    media = dest / "Media"
    for folder in (
        "WhatsApp Images",
        "WhatsApp Voice Notes",
        "WhatsApp Video",
        "WhatsApp Documents",
        "WhatsApp Stickers",
        "WhatsApp Animated Gifs",
    ):
        (media / folder).mkdir(parents=True, exist_ok=True)
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


def build(dest: Path = PHONE) -> Path:
    """Create `dest` as a copy of a phone's WhatsApp folder. Idempotent."""
    _media(dest)
    (dest / "Databases").mkdir(exist_ok=True)
    (dest / "Backups").mkdir(exist_ok=True)

    msgstore = dest / "Databases" / "msgstore.db"
    msgstore.unlink(missing_ok=True)
    with sqlite3.connect(msgstore) as con:
        con.executescript((HERE / "schema.sql").read_text(encoding="utf-8"))
        con.executescript((HERE / "data.sql").read_text(encoding="utf-8"))
        _thumbnails(con)
    (dest / "Databases" / "msgstore.db.crypt15").write_bytes(_encrypt(msgstore.read_bytes()))
    msgstore.unlink()

    contacts = dest / "Backups" / "wa.db"
    contacts.unlink(missing_ok=True)
    with sqlite3.connect(contacts) as con:
        con.executescript(CONTACTS_SCHEMA)
    (dest / "Backups" / "wa.db.crypt15").write_bytes(_encrypt(contacts.read_bytes()))
    contacts.unlink()
    return dest


def build_incremental(dest: Path) -> Path:
    """A delta as WhatsApp writes it: only the rows that changed since the full backup."""
    dest.mkdir(parents=True, exist_ok=True)
    delta = dest / "delta.db"
    with sqlite3.connect(delta) as con:
        con.executescript((HERE / "schema.sql").read_text(encoding="utf-8"))
        con.executescript(INCREMENTAL_SQL)
    target = dest / "msgstore-increment-2026-12-03.db.crypt15"
    target.write_bytes(_encrypt(delta.read_bytes()))
    delta.unlink()
    return target


def _encrypt(data: bytes) -> bytes:
    from wa_crypt_tools import Database15, KeyFactory, Props

    key = KeyFactory.from_hex(KEY_HEX)
    # A fixed IV: the fixture is committed, so its bytes must not change from one generation to the
    # next, and the header test pins this value.
    cipher = Database15(iv=bytes.fromhex("bd75ec327fb184c2b0fb38bbf0d26049"))
    return cipher.encrypt(key, Props(), zlib.compress(data, 9))


def _thumbnails(con: sqlite3.Connection) -> None:
    """The previews the phone keeps: the video's poster, and one for a file that is gone."""
    import io

    from PIL import Image

    buffer = io.BytesIO()
    Image.new("RGB", (24, 16), (200, 120, 60)).save(buffer, "JPEG")
    blob = buffer.getvalue()
    con.execute("INSERT INTO message_thumbnail VALUES (8, ?)", (blob,))
    con.execute("INSERT INTO message_thumbnail VALUES (12, ?)", (blob,))


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
