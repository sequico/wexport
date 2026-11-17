"""chat.db → the data scripts the archive loads.

One classic script per month, chronological, newest chunk last. Strings are escaped so no message
text can close a script tag or break a literal (plan §6, and the offline-html-archive skill).
"""

from __future__ import annotations

import json
import sqlite3
from dataclasses import dataclass
from pathlib import Path

from ..db import ro_connect


@dataclass(frozen=True)
class Chunk:
    name: str  # "2026-11"
    messages: int
    first_ts: int
    last_ts: int


def _js(value: object) -> str:
    text = json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
    return text.replace("<", "\\u003c").replace("\u2028", "\\u2028").replace("\u2029", "\\u2029")


def emit(
    chat_db: str | Path, destination: str | Path, manifest: dict, max_records: int = 20_000
) -> list[Chunk]:
    """Write `data/manifest.js` and `data/*.js`; returns the chunk list in order.

    A month is the usual bucket; a month holding more than `max_records` messages is split, because
    one 300k-message script is hundreds of megabytes to parse before the first pixel.
    """
    chat_db, destination = Path(chat_db), Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    con = ro_connect(chat_db)
    con.row_factory = sqlite3.Row
    chunks: list[Chunk] = []
    try:
        reactions = _reactions(con)
        current: str | None = None
        buffer: list[dict] = []
        parts: dict[str, int] = {}
        for row in con.execute(_MESSAGE_QUERY):
            month = (row["ts_local"] or "")[:7] or "0000-00"
            if current is not None and (month != current or len(buffer) >= max_records):
                chunks.append(_write_chunk(destination, current, buffer, parts))
                buffer = []
                if month != current:
                    parts = {}
            current = month
            buffer.append(_record(row, reactions.get(row["id"], [])))
        if current is not None:
            chunks.append(_write_chunk(destination, current, buffer, parts))
    finally:
        con.close()

    header = dict(manifest)
    header["chunks"] = [
        {"n": chunk.name, "c": chunk.messages, "f": chunk.first_ts, "l": chunk.last_ts}
        for chunk in chunks
    ]
    header["messages"] = sum(chunk.messages for chunk in chunks)
    (destination / "manifest.js").write_text(
        f"WEX.manifest({_js(header)});\n", encoding="utf-8"
    )
    return chunks


def _write_chunk(destination: Path, name: str, records: list[dict], parts: dict[str, int]) -> Chunk:
    part = parts.get(name, 0)
    parts[name] = part + 1
    file_name = name if part == 0 else f"{name}-{part + 1}"
    (destination / f"{file_name}.js").write_text(
        f'WEX.chunk("{file_name}",{_js(records)});\n', encoding="utf-8"
    )
    return Chunk(
        name=file_name,
        messages=len(records),
        first_ts=records[0]["t"],
        last_ts=records[-1]["t"],
    )


_MESSAGE_QUERY = """
SELECT m.id, m.ts, m.ts_local, m.from_me, m.sender, m.kind, m.text, m.quoted_id,
       m.edited, m.revoked, m.status, m.payload,
       d.path AS media_path, d.mime AS media_mime, d.bytes AS media_bytes,
       d.width AS media_width, d.height AS media_height, d.duration_ms AS media_duration,
       d.thumb AS media_thumb, d.peaks AS media_peaks, d.missing AS media_missing,
       d.alt AS media_alt, d.name AS media_name
  FROM message m
  LEFT JOIN media d ON d.id = (SELECT MIN(id) FROM media WHERE message_id = m.id)
 ORDER BY m.ts, m.id
"""

# shortcut: an album is several media rows on one message; the archive shows the first one.
# Render a gallery when a real backup with albums arrives.


def _record(row: sqlite3.Row, reactions: list[str]) -> dict:
    record: dict = {"i": row["id"], "t": row["ts"], "s": row["ts_local"], "k": row["kind"]}
    if row["from_me"]:
        record["f"] = 1
    if row["sender"]:
        record["a"] = row["sender"]
    if row["text"]:
        record["x"] = row["text"]
    if row["quoted_id"]:
        record["q"] = row["quoted_id"]
    if row["edited"]:
        record["e"] = 1
    if row["revoked"]:
        record["r"] = 1
    if row["status"]:
        record["st"] = row["status"]
    if row["payload"]:
        record["pl"] = json.loads(row["payload"])
    if reactions:
        record["rx"] = reactions
    if row["media_path"] or row["media_missing"]:
        media: dict = {}
        if row["media_path"]:
            media["p"] = row["media_path"]
        if row["media_mime"]:
            media["mt"] = row["media_mime"]
        if row["media_bytes"]:
            media["sz"] = row["media_bytes"]
        if row["media_width"]:
            media["w"] = row["media_width"]
        if row["media_height"]:
            media["h"] = row["media_height"]
        if row["media_duration"]:
            media["d"] = row["media_duration"]
        if row["media_thumb"]:
            media["th"] = row["media_thumb"]
        if row["media_alt"]:
            media["alt"] = row["media_alt"]
        if row["media_name"]:
            media["nm"] = row["media_name"]
        if row["media_peaks"]:
            media["pk"] = json.loads(row["media_peaks"])
        if row["media_missing"]:
            media["mi"] = 1
        record["md"] = media
    return record


def _reactions(con: sqlite3.Connection) -> dict[int, list[str]]:
    tables = {r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    if "reaction" not in tables:
        return {}
    out: dict[int, list[str]] = {}
    for message_id, emoji in con.execute("SELECT message_id, emoji FROM reaction ORDER BY message_id, emoji"):
        if emoji:
            out.setdefault(message_id, []).append(emoji)
    return out
