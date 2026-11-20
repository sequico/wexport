"""Build the synthetic iPhone backup the tests run on.

Everything here is invented: a Core Data store with WhatsApp's table names, tiny media copied from
the Android fixture, and a fixed set of files. No real WhatsApp data enters the repository.
"""

from __future__ import annotations

import hashlib
import io
import plistlib
import shutil
import sqlite3
from datetime import datetime, timezone
from pathlib import Path

from PIL import Image

HERE = Path(__file__).parent
BACKUP = HERE / "backup"
ANDROID = HERE.parent / "phone" / "Media"
DOMAIN = "AppDomainGroup-group.net.whatsapp.WhatsApp.shared"
EPOCH = datetime(2001, 1, 1, tzinfo=timezone.utc)

SCHEMA = """
CREATE TABLE ZWACHATSESSION (
    Z_PK INTEGER PRIMARY KEY,
    ZCONTACTJID TEXT,
    ZPARTNERNAME TEXT
);
CREATE TABLE ZWAMESSAGE (
    Z_PK INTEGER PRIMARY KEY,
    ZCHATSESSION INTEGER,
    ZMESSAGEDATE INTEGER,
    ZISFROMME INTEGER DEFAULT 0,
    ZTEXT TEXT,
    ZFROMJID TEXT,
    ZMEDIAITEM INTEGER,
    ZDELETED INTEGER DEFAULT 0,
    ZEDITED INTEGER DEFAULT 0,
    ZCALLTYPE INTEGER
);
CREATE TABLE ZWAMEDIAITEM (
    Z_PK INTEGER PRIMARY KEY,
    ZMEDIALOCALPATH TEXT,
    ZTHUMBNAILLOCALPATH TEXT,
    ZFILESIZE INTEGER,
    ZMIMETYPE TEXT,
    ZLATITUDE REAL,
    ZLONGITUDE REAL,
    ZVCARDNAME TEXT
);
CREATE TABLE ZWAGROUPMEMBER (
    Z_PK INTEGER PRIMARY KEY,
    ZCHATSESSION INTEGER,
    ZMEMBERJID TEXT,
    ZCONTACTNAME TEXT
);
"""


def stamp(iso: str) -> int:
    """Core Data counts seconds from 2001-01-01, and the fixture says so in ISO."""
    moment = datetime.fromisoformat(iso.replace("Z", "+00:00"))
    return int((moment - EPOCH).total_seconds())


def chats_and_messages(con: sqlite3.Connection) -> None:
    con.executescript(SCHEMA)
    con.executemany(
        "INSERT INTO ZWACHATSESSION VALUES (?, ?, ?)",
        [
            (1, "393331234567@s.whatsapp.net", "Alice"),
            (2, "120363000000000001@g.us", "Family"),
        ],
    )
    con.executemany(
        "INSERT INTO ZWAMEDIAITEM VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        [
            (
                100,
                "Media/WhatsApp Images/IMG-1.png",
                None,
                96,
                "image/png",
                None,
                None,
                None,
            ),
            (
                101,
                "Media/WhatsApp Video/VID-1.mp4",
                "Media/WhatsApp Video/VID-1.jpg",
                2000,
                "video/mp4",
                None,
                None,
                None,
            ),
            (
                102,
                "Media/Voice Notes/PTT-1.opus",
                None,
                3000,
                "audio/ogg",
                None,
                None,
                None,
            ),
            (
                103,
                "Media/Documents/notes.txt",
                None,
                18,
                "text/plain",
                None,
                None,
                None,
            ),
            (104, None, None, None, None, 45.4642, 9.19, None),
            (105, None, None, None, None, None, None, "Bob"),
            (
                106,
                "Media/WhatsApp Images/gone.png",
                "Media/WhatsApp Images/gone.jpg",
                100,
                "image/png",
                None,
                None,
                None,
            ),
        ],
    )
    con.executemany(
        "INSERT INTO ZWAMESSAGE VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        [
            (
                1,
                1,
                stamp("2026-12-01T09:00:00Z"),
                0,
                "Ciao, come stai?",
                "393331234567@s.whatsapp.net",
                None,
                0,
                0,
                None,
            ),
            (2, 1, stamp("2026-12-01T09:01:00Z"), 1, "Tutto bene", None, None, 0, 0, None),
            (3, 1, stamp("2026-12-01T09:02:00Z"), 0, None, "393331234567@s.whatsapp.net", 100, 0, 0, None),
            (4, 1, stamp("2026-12-01T09:03:00Z"), 0, None, "393331234567@s.whatsapp.net", 101, 0, 0, None),
            (5, 1, stamp("2026-12-01T09:04:00Z"), 1, None, None, 102, 0, 0, None),
            (6, 1, stamp("2026-12-01T09:05:00Z"), 0, "document", "393331234567@s.whatsapp.net", 103, 0, 0, None),
            (7, 1, stamp("2026-12-01T09:06:00Z"), 0, None, "393331234567@s.whatsapp.net", 104, 0, 0, None),
            (8, 1, stamp("2026-12-01T09:07:00Z"), 0, None, "393331234567@s.whatsapp.net", 105, 0, 0, None),
            (9, 1, stamp("2026-12-01T09:08:00Z"), 1, None, None, None, 0, 0, 1),
            (10, 1, stamp("2026-12-01T09:09:00Z"), 0, "deleted on the phone", None, None, 1, 0, None),
            (11, 1, stamp("2026-12-01T09:10:00Z"), 0, "edited text", "393331234567@s.whatsapp.net", None, 0, 1, None),
            (12, 1, stamp("2026-12-01T09:11:00Z"), 0, None, "393331234567@s.whatsapp.net", 106, 0, 0, None),
            (
                13,
                2,
                stamp("2026-12-02T10:00:00Z"),
                0,
                "see you at eight",
                "393339999999@s.whatsapp.net",
                None,
                0,
                0,
                None,
            ),
            (
                14,
                2,
                stamp("2026-12-02T10:01:00Z"),
                1,
                "done",
                None,
                None,
                0,
                0,
                None,
            ),
        ],
    )
    con.executemany(
        "INSERT INTO ZWAGROUPMEMBER VALUES (?, ?, ?, ?)",
        [
            (1, 2, "393339999999@s.whatsapp.net", "Bob"),
            (2, 2, "393331234567@s.whatsapp.net", "Alice"),
        ],
    )


def stamp_media(group: Path) -> None:
    """The files the rows name, under the app group root, plus the previews."""
    images = group / "Message/Media/WhatsApp Images"
    videos = group / "Message/Media/WhatsApp Video"
    notes = group / "Message/Media/Voice Notes"
    documents = group / "Message/Media/Documents"
    for folder in (images, videos, notes, documents):
        folder.mkdir(parents=True, exist_ok=True)
    Image.new("RGB", (24, 16), (60, 120, 200)).save(images / "IMG-1.png", "PNG")
    shutil.copyfile(
        ANDROID / "WhatsApp Video/VID-20261201-WA0003.mp4", videos / "VID-1.mp4"
    )
    shutil.copyfile(
        ANDROID / "WhatsApp Voice Notes/PTT-20261201-WA0002.opus", notes / "PTT-1.opus"
    )
    (documents / "notes.txt").write_text("iphone fixture\n", encoding="utf-8")
    preview = io.BytesIO()
    Image.new("RGB", (24, 16), (200, 120, 60)).save(preview, "JPEG")
    (videos / "VID-1.jpg").write_bytes(preview.getvalue())
    (images / "gone.jpg").write_bytes(preview.getvalue())


def file_id(relative: str) -> str:
    return hashlib.sha1(f"{DOMAIN}-{relative}".encode()).hexdigest()


def add_file(manifest: sqlite3.Connection, root: Path, relative: str) -> None:
    source = root / relative
    identifier = file_id(relative)
    target = BACKUP / identifier[:2] / identifier
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    manifest.execute(
        "INSERT INTO Files (fileID, domain, relativePath, flags, file) VALUES (?, ?, ?, 1, NULL)",
        (identifier, DOMAIN, relative),
    )


def build() -> Path:
    """Create `backup/` as a Finder backup of the WhatsApp app group. Idempotent."""
    if BACKUP.exists():
        shutil.rmtree(BACKUP)
    BACKUP.mkdir(parents=True)
    stage = BACKUP / "stage"
    group = stage / "group.net.whatsapp.WhatsApp.shared"
    group.mkdir(parents=True)
    stamp_media(group)

    database = group / "ChatStorage.sqlite"
    con = sqlite3.connect(database)
    con.execute("PRAGMA journal_mode=WAL")
    con.execute("PRAGMA wal_autocheckpoint=0")
    chats_and_messages(con)
    con.commit()
    con.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    con.commit()
    con.close()

    # The newest message lands in the log, not in the main file: a build that skips the WAL loses it.
    con = sqlite3.connect(database)
    con.execute("PRAGMA journal_mode=WAL")
    con.execute("PRAGMA wal_autocheckpoint=0")
    con.execute(
        "INSERT INTO ZWAMESSAGE VALUES (15, 1, ?, 0, 'last word, only in the log', ?, NULL, 0, 0, NULL)",
        (stamp("2026-12-03T08:00:00Z"), "393331234567@s.whatsapp.net"),
    )
    con.commit()

    manifest = sqlite3.connect(BACKUP / "Manifest.db")
    manifest.execute(
        "CREATE TABLE Files (fileID TEXT PRIMARY KEY, domain TEXT, relativePath TEXT, "
        "flags INTEGER, file BLOB)"
    )
    relative_files = [
        "ChatStorage.sqlite",
        "ChatStorage.sqlite-wal",
        "ChatStorage.sqlite-shm",
        "Message/Media/WhatsApp Images/IMG-1.png",
        "Message/Media/WhatsApp Images/gone.jpg",
        "Message/Media/WhatsApp Video/VID-1.mp4",
        "Message/Media/WhatsApp Video/VID-1.jpg",
        "Message/Media/Voice Notes/PTT-1.opus",
        "Message/Media/Documents/notes.txt",
    ]
    # Copied while the connection is open, so the log travels with the database.
    for relative in relative_files:
        add_file(manifest, group, relative)
    manifest.commit()
    manifest.close()
    con.close()

    (BACKUP / "Manifest.plist").write_bytes(
        plistlib.dumps(
            {
                "IsEncrypted": False,
                "Version": "10.0",
                "Device Name": "fixture",
                "Last Backup Date": datetime(2026, 12, 3),
            },
            fmt=plistlib.FMT_BINARY,
        )
    )
    shutil.rmtree(stage)
    return BACKUP


if __name__ == "__main__":
    print(f"iphone backup at {build()}")
