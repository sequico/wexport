"""The only module that knows WhatsApp's table and column names.

Everything downstream depends on the dataclasses here, so a WhatsApp rename breaks exactly this
file (plan §3). Column names are resolved by probing the schema, never by guessing a version.
"""

from __future__ import annotations

import sqlite3
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

from ..errors import UnreadableInput

# Numeric media types used by older schemas; the mime type wins when both are present.
_MEDIA_WA_TYPES = {
    1: "image",
    2: "audio",
    3: "video",
    4: "contact",
    5: "location",
    8: "document",
    13: "gif",
    20: "sticker",
}


@dataclass(frozen=True)
class Chat:
    jid: str
    name: str
    is_group: bool


@dataclass
class Media:
    rel_path: str
    kind: str = "document"
    mime: str | None = None
    bytes: int | None = None
    width: int | None = None
    height: int | None = None
    duration_ms: int | None = None


@dataclass
class Message:
    id: int
    ts: int
    from_me: bool
    kind: str
    text: str = ""
    sender_jid: str | None = None
    quoted_id: int | None = None
    edited: bool = False
    revoked: bool = False
    status: int | None = None
    payload: dict = field(default_factory=dict)
    media: Media | None = None


class Msgstore:
    """A decrypted msgstore.db, read-only."""

    def __init__(self, path: str | Path):
        self.path = Path(path)
        try:
            self.con = sqlite3.connect(f"file:{self.path}?mode=ro", uri=True)
        except sqlite3.Error as exc:
            raise UnreadableInput(f"cannot open {self.path.name}: {exc}") from exc
        self.con.row_factory = sqlite3.Row
        self._resolve()

    def close(self) -> None:
        self.con.close()

    def __enter__(self) -> Msgstore:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    # ------------------------------------------------------------- schema probing

    def _tables(self) -> set[str]:
        rows = self.con.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()
        return {r[0] for r in rows}

    def _cols(self, table: str) -> list[str]:
        return [r[1] for r in self.con.execute(f'PRAGMA table_info("{table}")')]

    def _pick(self, table: str, *names: str, required: bool = False) -> str | None:
        cols = self._cols(table) if table in self.has else []
        for name in names:
            if name in cols:
                return name
        if required:
            raise UnreadableInput(
                f"{self.path.name}: table {table} has none of the columns {', '.join(names)}.\n"
                "The schema is newer or older than this build understands; nothing was written."
            )
        return None

    def _resolve(self) -> None:
        self.has = self._tables()
        if "message" not in self.has:
            raise UnreadableInput(f"{self.path.name}: no message table — not a msgstore database")

        self.c_id = self._pick("message", "_id", required=True)
        self.c_ts = self._pick("message", "timestamp", required=True)
        self.c_from_me = self._pick("message", "from_me", "key_from_me")
        self.c_text = self._pick("message", "text_data", "data")
        self.c_status = self._pick("message", "status")
        self.c_remote = self._pick("message", "remote_resource")
        self.c_chat_row = self._pick("message", "chat_row_id")
        self.c_chat_jid = self._pick("message", "key_remote_jid")
        self.c_quoted = self._pick("message", "quoted_row_id")
        self.c_edit = self._pick("message", "edit_version")
        self.c_lat = self._pick("message", "latitude")
        self.c_lon = self._pick("message", "longitude")
        self.c_wa_type = self._pick("message", "media_wa_type", "message_type")

        self.media_cols = {
            "message": self._pick("message_media", "message_row_id", "message_id"),
            "path": self._pick("message_media", "file_path"),
            "mime": self._pick("message_media", "mime_type", "media_mime_type"),
            "bytes": self._pick("message_media", "file_size", "media_size"),
            "width": self._pick("message_media", "width"),
            "height": self._pick("message_media", "height"),
            "duration": self._pick("message_media", "media_duration", "duration"),
            "name": self._pick("message_media", "media_name", "file_name"),
            "caption": self._pick("message_media", "caption", "media_caption"),
            "wa_type": self._pick("message_media", "media_wa_type"),
        }

        self.chat_jid_row: dict[int, int] = {}
        self.contact_names: dict[str, str] = {}
        self.chat_by_row = self._resolve_chats()
        self._system_ids = self._id_set("message_system", "message_row_id")

    def _resolve_chats(self) -> dict[int, Chat]:
        if "chat" not in self.has or "jid" not in self.has:
            return {}
        jid_row = self._pick("chat", "jid_row_id")
        jid_raw = self._pick("jid", "raw_string", "user")
        if not jid_row or not jid_raw:
            return {}
        subject = self._pick("chat", "subject")
        subject_sql = f'c."{subject}"' if subject else "NULL"
        rows = self.con.execute(
            f'SELECT c."_id", j."{jid_raw}", {subject_sql}, c."{jid_row}" '
            f'FROM chat c JOIN jid j ON j."_id" = c."{jid_row}"'
        ).fetchall()
        out: dict[int, Chat] = {}
        self.chat_jid_row: dict[int, int] = {}
        for row in rows:
            jid = (row[1] or "").strip()
            if not jid:
                continue
            out[row[0]] = Chat(jid=jid, name=_display_name(row[2], jid), is_group=jid.endswith("@g.us"))
            self.chat_jid_row[row[0]] = int(row[3])
        return out

    # ------------------------------------------------------------- reading

    def chats(self) -> list[Chat]:
        if self.chat_by_row:
            return sorted(self.chat_by_row.values(), key=lambda c: c.name.lower())
        if not self.c_chat_jid:
            raise UnreadableInput(f"{self.path.name}: cannot tell which chat a message belongs to")
        seen: dict[str, Chat] = {}
        for (raw,) in self.con.execute(f'SELECT DISTINCT "{self.c_chat_jid}" FROM message'):
            jid = (raw or "").strip()
            if jid:
                seen[jid] = Chat(jid=jid, name=_display_name(None, jid), is_group=jid.endswith("@g.us"))
        return sorted(seen.values(), key=lambda c: c.name.lower())

    def last_ts(self, chat: Chat) -> int:
        """Newest message time in one chat, for naming the archive folder. One indexed query."""
        where, params = self._chat_filter(chat)
        row = self.con.execute(f'SELECT MAX(m."{self.c_ts}") FROM message m WHERE {where}', params).fetchone()
        newest = int((row[0] if row else 0) or 0)
        if "call_log" in self.has and self.chat_by_row:
            wanted = [
                self.chat_jid_row[cid] for cid, c in self.chat_by_row.items() if c.jid == chat.jid
            ]
            if wanted:
                marks = ",".join("?" * len(wanted))
                call = self.con.execute(
                    f'SELECT MAX(timestamp) FROM call_log WHERE jid_row_id IN ({marks})', wanted
                ).fetchone()
                newest = max(newest, int((call[0] if call else 0) or 0))
        return newest

    def apply_contacts(self, path: str | Path) -> None:
        """Name the chats from `wa.db`, when the user copied it. Numbers otherwise."""
        path = Path(path)
        try:
            con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
            tables = {r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        except sqlite3.Error:
            return
        try:
            if "wa_contacts" not in tables:
                return
            columns = [r[1] for r in con.execute("PRAGMA table_info(wa_contacts)")]
            name = next((c for c in ("display_name", "wa_name", "given_name") if c in columns), None)
            if not name or "jid" not in columns:
                return
            names = {
                _jid_key(jid): (value or "").strip()
                for jid, value in con.execute(f'SELECT jid, "{name}" FROM wa_contacts')
                if value
            }
        finally:
            con.close()
        if not names:
            return
        self.contact_names = names
        updated: dict[int, Chat] = {}
        for row_id, chat in self.chat_by_row.items():
            renamed = names.get(_jid_key(chat.jid))
            updated[row_id] = Chat(jid=chat.jid, name=renamed or chat.name, is_group=chat.is_group)
        self.chat_by_row = updated

    def messages(self, chat: Chat) -> Iterator[Message]:
        """Every message of one chat in time order, call-log entries included."""
        revoked = self._id_set("message_revoked", "message_row_id")
        polls = self._id_set("message_add_on", "message_row_id")
        media_rows = self._media_rows()
        vcards = self._vcard_rows()

        columns = [
            f'm."{self.c_id}" AS id',
            f'm."{self.c_ts}" AS ts',
            f'm."{self.c_from_me}" AS from_me' if self.c_from_me else "0 AS from_me",
            f'm."{self.c_text}" AS text' if self.c_text else "NULL AS text",
            f'm."{self.c_status}" AS status' if self.c_status else "NULL AS status",
            f'm."{self.c_remote}" AS remote' if self.c_remote else "NULL AS remote",
            f'm."{self.c_quoted}" AS quoted' if self.c_quoted else "NULL AS quoted",
            f'm."{self.c_edit}" AS edit' if self.c_edit else "0 AS edit",
            f'm."{self.c_lat}" AS lat' if self.c_lat else "NULL AS lat",
            f'm."{self.c_lon}" AS lon' if self.c_lon else "NULL AS lon",
            f'm."{self.c_wa_type}" AS wa_type' if self.c_wa_type else "NULL AS wa_type",
            f'm."{self.c_chat_row}" AS chat_row' if self.c_chat_row else "NULL AS chat_row",
        ]
        where, params = self._chat_filter(chat)
        query = (
            f'SELECT {", ".join(columns)} FROM message m '
            f'WHERE {where} ORDER BY m."{self.c_ts}", m."{self.c_id}"'
        )
        for row in self.con.execute(query, params):
            media = self._media_object(media_rows.get(row["id"]), row["wa_type"])
            caption = self._caption(media_rows.get(row["id"])) or ""
            kind, payload = self._kind(row, media, vcards.get(row["id"]), row["id"] in polls)
            yield Message(
                id=row["id"],
                ts=int(row["ts"] or 0),
                from_me=bool(row["from_me"]),
                kind="deleted" if row["id"] in revoked else kind,
                text=(row["text"] or "") or caption,
                sender_jid=(self._person_name(row["remote"]) if not row["from_me"] else None),
                quoted_id=row["quoted"] or None,
                edited=bool(row["edit"]),
                revoked=row["id"] in revoked,
                status=row["status"],
                payload=payload,
                media=media,
            )
        yield from self._calls(chat)

    def _person_name(self, jid: str | None) -> str | None:
        """A group sender as a name when wa.db knows it, otherwise as its number."""
        if not jid:
            return None
        return self.contact_names.get(_jid_key(jid)) or jid.split("@")[0]

    def _chat_filter(self, chat: Chat) -> tuple[str, list]:
        if self.c_chat_row and self.chat_by_row:
            ids = [cid for cid, c in self.chat_by_row.items() if c.jid == chat.jid]
            return ("0", []) if not ids else (f'm."{self.c_chat_row}" = ?', [ids[0]])
        if not self.c_chat_jid:
            return "0", []
        variants = {chat.jid, chat.jid.replace("@s.whatsapp.net", "@c.us")}
        return f'm."{self.c_chat_jid}" IN ({",".join("?" * len(variants))})', list(variants)

    def _id_set(self, table: str, column: str) -> set[int]:
        if table not in self.has or column not in self._cols(table):
            return set()
        return {row[0] for row in self.con.execute(f'SELECT "{column}" FROM "{table}"')}

    def _media_rows(self) -> dict[int, sqlite3.Row]:
        key = self.media_cols["message"]
        if "message_media" not in self.has or not key:
            return {}
        return {row[key]: row for row in self.con.execute("SELECT * FROM message_media")}

    def _caption(self, row: sqlite3.Row | None) -> str | None:
        column = self.media_cols["caption"]
        return (row[column] or "") if row is not None and column else None

    def _media_object(self, row: sqlite3.Row | None, wa_type: object) -> Media | None:
        cols = self.media_cols
        if row is None or not cols["path"]:
            return None
        rel_path = (row[cols["path"]] or "").replace("\\", "/")
        if not rel_path:
            return None
        return Media(
            rel_path=rel_path,
            kind=_media_kind(row, cols, wa_type),
            mime=(row[cols["mime"]] if cols["mime"] else None),
            bytes=(row[cols["bytes"]] if cols["bytes"] else None),
            width=(row[cols["width"]] if cols["width"] else None),
            height=(row[cols["height"]] if cols["height"] else None),
            duration_ms=(row[cols["duration"]] if cols["duration"] else None),
        )

    def _vcard_rows(self) -> dict[int, dict]:
        if "message_vcard" not in self.has:
            return {}
        cols = self._cols("message_vcard")
        key = next((c for c in ("message_row_id", "message_id") if c in cols), None)
        name = next((c for c in ("display_name", "formatted_name", "vcard") if c in cols), None)
        if key is None or name is None:
            return {}
        rows = self.con.execute(f'SELECT "{key}", "{name}" FROM message_vcard')
        return {row[0]: {"name": row[1] or ""} for row in rows}

    def _kind(
        self, row: sqlite3.Row, media: Media | None, vcard: dict | None, is_poll: bool
    ) -> tuple[str, dict]:
        if row["id"] in self._system_ids:
            return "system", {}
        if media is not None:
            return media.kind, {}
        if row["lat"] is not None and row["lon"] is not None:
            return "location", {"lat": row["lat"], "lon": row["lon"]}
        if vcard is not None:
            return "contact", {"name": vcard.get("name", "")}
        if is_poll:
            # shortcut: poll options and votes are not decoded; it renders as a card with its text.
            return "poll", {"type": "poll"}
        return "text", {}

    def _calls(self, chat: Chat) -> Iterator[Message]:
        if "call_log" not in self.has:
            return
        cols = self._cols("call_log")
        jid_col = next((c for c in ("jid_row_id", "key_remote_jid") if c in cols), None)
        if jid_col is None or not self.chat_by_row:
            return
        wanted = {self.chat_jid_row[cid] for cid, c in self.chat_by_row.items() if c.jid == chat.jid}
        if not wanted:
            return
        duration = next((c for c in ("duration", "call_duration") if c in cols), None)
        video = next((c for c in ("video_call", "is_video") if c in cols), None)
        missed = next((c for c in ("missed_call", "is_missed") if c in cols), None)
        from_me = "from_me" if "from_me" in cols else None
        rows = self.con.execute('SELECT * FROM "call_log" ORDER BY timestamp')
        for row in rows:
            if row[jid_col] not in wanted:
                continue
            yield Message(
                id=-int(row["_id"]),
                ts=int(row["timestamp"] or 0),
                from_me=bool(row[from_me]) if from_me else False,
                kind="call",
                payload={
                    "duration_s": row[duration] if duration else 0,
                    "video": bool(row[video]) if video else False,
                    "missed": bool(row[missed]) if missed else False,
                },
            )


def _media_kind(row: sqlite3.Row, cols: dict, wa_type: object) -> str:
    numeric = wa_type
    if isinstance(numeric, int) and numeric in _MEDIA_WA_TYPES:
        return _MEDIA_WA_TYPES[numeric]
    mime = (row[cols["mime"]] or "") if cols["mime"] else ""
    name = ((row[cols["name"]] if cols["name"] else "") or "").lower()
    path = ((row[cols["path"]] or "") if cols["path"] else "").lower()
    if mime.startswith("image/"):
        sticker = mime == "image/webp" and ("sticker" in path or "sticker" in name)
        return "sticker" if sticker else "image"
    if mime.startswith("video/"):
        return "gif" if name.startswith("gif") or "animated" in path else "video"
    if mime.startswith("audio/"):
        return "audio"
    return "document"


def _display_name(name: str | None, jid: str) -> str:
    return name.strip() if name else jid.split("@")[0]


def _jid_key(jid: str) -> str:
    """The same person can arrive as @s.whatsapp.net or @c.us; compare on the number."""
    return jid.split("@")[0].split(":")[0]
