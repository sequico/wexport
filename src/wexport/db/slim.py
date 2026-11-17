"""`chat.db`: the build-time intermediate the archive is emitted from.

It never ships. Timestamps are frozen here, once, so no viewer and no future machine has to
derive a local time from an epoch (plan §4).
"""

from __future__ import annotations

import sqlite3
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from ..errors import UnreadableInput

SCHEMA = """
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE contact (id INTEGER PRIMARY KEY, jid TEXT UNIQUE, name TEXT, is_me INTEGER DEFAULT 0);
CREATE TABLE chat (id INTEGER PRIMARY KEY, jid TEXT UNIQUE, name TEXT, is_group INTEGER,
                   first_ts INTEGER, last_ts INTEGER, message_count INTEGER);
CREATE TABLE message (id INTEGER PRIMARY KEY, chat_id INTEGER, ts INTEGER, ts_local TEXT,
                      from_me INTEGER, sender_jid TEXT, kind TEXT, text TEXT,
                      quoted_id INTEGER, edited_ts INTEGER, revoked INTEGER DEFAULT 0,
                      status INTEGER, payload TEXT);
CREATE TABLE media (id INTEGER PRIMARY KEY, message_id INTEGER, kind TEXT, path TEXT,
                    mime TEXT, bytes INTEGER, width INTEGER, height INTEGER, duration_ms INTEGER,
                    thumb TEXT, peaks TEXT, sha256 TEXT, alt TEXT, name TEXT,
                    missing INTEGER DEFAULT 0);
CREATE TABLE reaction (message_id INTEGER, sender_jid TEXT, emoji TEXT, ts INTEGER);
CREATE TABLE receipt (message_id INTEGER, delivered_ts INTEGER, read_ts INTEGER, played_ts INTEGER);
CREATE INDEX message_chat_ts ON message(chat_id, ts);
CREATE INDEX media_message ON media(message_id);
"""


def tzdata_version() -> str:
    """The installed tz database version, best effort — recorded so a reader knows the basis."""
    for candidate in ("/usr/share/zoneinfo/tzdata.zi", "/usr/share/zoneinfo/tzdata.zi.gz"):
        path = Path(candidate)
        if path.is_file():
            try:
                first = path.read_text(encoding="utf-8", errors="replace").splitlines()[0]
            except OSError:
                return ""
            return first.strip()
    return ""


class FrozenTime:
    """Renders a wall clock once, with its explicit offset."""

    def __init__(self, tz_name: str):
        try:
            self.zone = ZoneInfo(tz_name)
        except (ZoneInfoNotFoundError, ValueError) as exc:
            raise UnreadableInput(f"unknown timezone {tz_name!r}: {exc}") from exc
        self.tz_name = tz_name

    def local(self, epoch_ms: int) -> str:
        moment = datetime.fromtimestamp(epoch_ms / 1000, tz=self.zone)
        text = moment.isoformat(sep=" ", timespec="seconds")
        if text[-6] in "+-":  # "2026-11-17 14:32:05+01:00" reads better with the space
            text = f"{text[:-6]} {text[-6:]}"
        return text


class ChatDB:
    """A write-once `chat.db`."""

    def __init__(self, path: str | Path, time: FrozenTime, app_version: str):
        self.path = Path(path)
        self.time = time
        self.con = sqlite3.connect(self.path)
        self.con.executescript(SCHEMA)
        self.con.executemany(
            "INSERT INTO meta(key, value) VALUES (?, ?)",
            [
                ("schema", "1"),
                ("source_app_version", app_version),
                ("tz", time.tz_name),
                ("tzdata", tzdata_version()),
                ("builder", "wexport"),
            ],
        )
        self.chat_id: int | None = None

    def add_chat(self, jid: str, name: str, is_group: bool) -> int:
        cursor = self.con.execute(
            "INSERT INTO chat(jid, name, is_group) VALUES (?, ?, ?)", (jid, name, int(is_group))
        )
        self.chat_id = int(cursor.lastrowid)
        return self.chat_id

    def add_message(self, message) -> int:
        assert self.chat_id is not None
        cursor = self.con.execute(
            "INSERT INTO message(id, chat_id, ts, ts_local, from_me, sender_jid, kind, text,"
            " quoted_id, edited_ts, revoked, status, payload)"
            " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                message.id,
                self.chat_id,
                message.ts,
                self.time.local(message.ts),
                int(message.from_me),
                message.sender_jid,
                message.kind,
                message.text,
                message.quoted_id,
                message.ts if message.edited else None,
                int(message.revoked),
                message.status,
                None if not message.payload else _json(message.payload),
            ),
        )
        return int(cursor.lastrowid)

    def add_media(self, message_id: int, row: dict) -> None:
        self.con.execute(
            "INSERT INTO media(message_id, kind, path, mime, bytes, width, height, duration_ms,"
            " thumb, peaks, sha256, alt, name, missing) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                message_id,
                row.get("kind"),
                row.get("path"),
                row.get("mime"),
                row.get("bytes"),
                row.get("width"),
                row.get("height"),
                row.get("duration_ms"),
                row.get("thumb"),
                row.get("peaks"),
                row.get("sha256"),
                row.get("alt"),
                row.get("name"),
                int(bool(row.get("missing"))),
            ),
        )

    def add_reaction(self, message_id: int, sender_jid: str | None, emoji: str, ts: int | None) -> None:
        self.con.execute(
            "INSERT INTO reaction(message_id, sender_jid, emoji, ts) VALUES (?,?,?,?)",
            (message_id, sender_jid, emoji, ts),
        )

    def set_chat_range(self, first_ts: int, last_ts: int, count: int) -> None:
        self.con.execute(
            "UPDATE chat SET first_ts = ?, last_ts = ?, message_count = ? WHERE id = ?",
            (first_ts, last_ts, count, self.chat_id),
        )

    def commit(self) -> None:
        self.con.commit()

    def close(self) -> None:
        self.con.commit()
        self.con.close()

    def __enter__(self) -> ChatDB:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()


def _json(value: dict) -> str:
    import json

    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
