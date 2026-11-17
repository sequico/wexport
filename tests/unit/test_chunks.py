"""Chunk emission: escaping, determinism, month split, manifest."""

from __future__ import annotations

import json
import sqlite3

import pytest

from wexport.site import chunks
from wexport.site.build import ASSETS  # noqa: F401 - keeps the import surface honest


@pytest.fixture()
def chat_db(tmp_path):
    path = tmp_path / "chat.db"
    con = sqlite3.connect(path)
    con.executescript(
        """
        CREATE TABLE message (id INTEGER PRIMARY KEY, ts INTEGER, ts_local TEXT, from_me INTEGER,
            sender TEXT, kind TEXT, text TEXT, quoted_id INTEGER, edited INTEGER,
            revoked INTEGER, status INTEGER, payload TEXT);
        CREATE TABLE media (id INTEGER PRIMARY KEY, message_id INTEGER, path TEXT, mime TEXT,
            bytes INTEGER, width INTEGER, height INTEGER, duration_ms INTEGER, thumb TEXT,
            peaks TEXT, alt TEXT, name TEXT, missing INTEGER);
        CREATE TABLE reaction (message_id INTEGER, emoji TEXT);
        """
    )
    rows = [
        (1, 1763382725000, "2026-11-17 14:32:05 +01:00", 0, None, "text",
         "hi </script> \u2028 end", None, None, 0, 13, None),
        (2, 1763382725001, "2026-11-17 14:32:05 +01:00", 1, None, "text",
         "second", 1, None, 0, 13, None),
        (3, 1764756000000, "2026-12-03 10:00:00 +01:00", 0, "39333@s.whatsapp.net", "image",
         "cap", None, None, 0, 13, None),
    ]
    con.executemany("INSERT INTO message VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", rows)
    con.execute(
        "INSERT INTO media (id, message_id, path, mime, bytes, width, height, thumb, name, missing)"
        " VALUES (1, 3, 'media/ab/x.png', 'image/png', 10, 2, 3, 'deriv/ab.webp', 'IMG-1.png', 0)"
    )
    con.execute("INSERT INTO reaction (message_id, emoji) VALUES (1, '👍')")
    con.commit()
    con.close()
    return path


def test_escaping_and_split(chat_db, tmp_path):
    out = tmp_path / "data"
    emitted = chunks.emit(chat_db, out, {"title": "t", "built": "fixed"})
    assert [chunk.name for chunk in emitted] == ["2026-11", "2026-12"]
    november = (out / "2026-11.js").read_text(encoding="utf-8")
    assert "\\u003c/script" in november
    assert "\\u2028" in november


def test_manifest_carries_counts_and_bounds(chat_db, tmp_path):
    out = tmp_path / "data"
    chunks.emit(chat_db, out, {"title": "t", "built": "fixed"})
    manifest = json.loads(
        (out / "manifest.js").read_text(encoding="utf-8").strip()[len("WEX.manifest(") : -2]
    )
    assert manifest["messages"] == 3
    assert manifest["chunks"][0]["c"] == 2
    assert manifest["chunks"][0]["f"] < manifest["chunks"][1]["f"]


def test_emission_is_deterministic(chat_db, tmp_path):
    first = tmp_path / "a"
    second = tmp_path / "b"
    chunks.emit(chat_db, first, {"title": "t", "built": "fixed"})
    chunks.emit(chat_db, second, {"title": "t", "built": "fixed"})
    for name in ("manifest.js", "2026-11.js", "2026-12.js"):
        assert (first / name).read_bytes() == (second / name).read_bytes()


def test_two_media_rows_produce_one_message(tmp_path):
    """An album is several media rows; the record must not be duplicated for each of them."""
    database = tmp_path / "chat.db"
    con = sqlite3.connect(database)
    con.executescript(
        """
        CREATE TABLE message (id INTEGER PRIMARY KEY, ts INTEGER, ts_local TEXT, from_me INTEGER,
            sender TEXT, kind TEXT, text TEXT, quoted_id INTEGER, edited INTEGER,
            revoked INTEGER, status INTEGER, payload TEXT);
        CREATE TABLE media (id INTEGER PRIMARY KEY, message_id INTEGER, path TEXT, mime TEXT,
            bytes INTEGER, width INTEGER, height INTEGER, duration_ms INTEGER, thumb TEXT,
            peaks TEXT, alt TEXT, name TEXT, missing INTEGER);
        CREATE TABLE reaction (message_id INTEGER, emoji TEXT);
        """
    )
    con.execute(
        "INSERT INTO message VALUES (1, 1763382725000, '2026-11-17 14:32:05 +01:00', 0, NULL,"
        " 'image', 'album', NULL, 0, 0, 13, NULL)"
    )
    con.executemany(
        "INSERT INTO media (id, message_id, path, mime, name) VALUES (?,?,?,?,?)",
        [(1, 1, "media/aa/one.png", "image/png", "one.png"), (2, 1, "media/bb/two.png", "image/png", "two.png")],
    )
    con.commit()
    con.close()

    out = tmp_path / "data"
    emitted = chunks.emit(database, out, {"title": "t", "built": "fixed"})
    assert emitted[0].messages == 1
    text = (out / "2026-11.js").read_text(encoding="utf-8")
    assert text.count('"i":1') == 1
    assert '"nm":"one.png"' in text


def test_media_and_reactions_reach_the_record(chat_db, tmp_path):
    out = tmp_path / "data"
    chunks.emit(chat_db, out, {"title": "t", "built": "fixed"})
    december = (out / "2026-12.js").read_text(encoding="utf-8")
    assert '"th":"deriv/ab.webp"' in december
    assert '"nm":"IMG-1.png"' in december
    november = (out / "2026-11.js").read_text(encoding="utf-8")
    assert '"rx":["👍"]' in november
