"""chat.db → the data scripts the archive loads.

One classic script per month, chronological, newest chunk last. Strings are escaped so no message
text can close a script tag or break a literal (plan §6, and the offline-html-archive skill).
"""

from __future__ import annotations

import json
import sqlite3
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Chunk:
    name: str  # "2026-11"
    messages: int
    first_ts: int
    last_ts: int


def _js(value: object) -> str:
    text = json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
    return text.replace("<", "\\u003c").replace("\u2028", "\\u2028").replace("\u2029", "\\u2029")


def emit(chat_db: str | Path, destination: str | Path, manifest: dict) -> list[Chunk]:
    """Write `data/manifest.js` and `data/<YYYY-MM>.js`; returns the chunk list in order."""
    chat_db, destination = Path(chat_db), Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    con = sqlite3.connect(f"file:{chat_db}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    chunks: list[Chunk] = []
    try:
        reactions = _reactions(con)
        current: str | None = None
        buffer: list[dict] = []
        for row in con.execute(_MESSAGE_QUERY):
            month = (row["ts_local"] or "")[:7] or "0000-00"
            if current is not None and month != current:
                chunks.append(_write_chunk(destination, current, buffer))
                buffer = []
            current = month
            buffer.append(_record(row, reactions.get(row["id"], [])))
        if current is not None:
            chunks.append(_write_chunk(destination, current, buffer))
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


def _write_chunk(destination: Path, name: str, records: list[dict]) -> Chunk:
    (destination / f"{name}.js").write_text(
        f'WEX.chunk("{name}",{_js(records)});\n', encoding="utf-8"
    )
    return Chunk(name=name, messages=len(records), first_ts=records[0]["t"], last_ts=records[-1]["t"])


_MESSAGE_QUERY = """
SELECT m.id, m.ts, m.ts_local, m.from_me, m.sender_jid, m.kind, m.text, m.quoted_id,
       m.edited_ts, m.revoked, m.status, m.payload,
       d.path AS media_path, d.mime AS media_mime, d.bytes AS media_bytes,
       d.width AS media_width, d.height AS media_height, d.duration_ms AS media_duration,
       d.thumb AS media_thumb, d.peaks AS media_peaks, d.missing AS media_missing,
       d.alt AS media_alt, d.name AS media_name
  FROM message m
  LEFT JOIN media d ON d.message_id = m.id
 ORDER BY m.ts, m.id
"""


def _record(row: sqlite3.Row, reactions: list[str]) -> dict:
    record: dict = {"i": row["id"], "t": row["ts"], "s": row["ts_local"], "k": row["kind"]}
    if row["from_me"]:
        record["f"] = 1
    if row["sender_jid"]:
        record["a"] = row["sender_jid"].split("@")[0]
    if row["text"]:
        record["x"] = row["text"]
    if row["quoted_id"]:
        record["q"] = row["quoted_id"]
    if row["edited_ts"]:
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
    for message_id, emoji in con.execute("SELECT message_id, emoji FROM reaction ORDER BY message_id"):
        if emoji:
            out.setdefault(message_id, []).append(emoji)
    return out
