"""Frozen timestamps: written once, with their offset, never re-derived."""

from __future__ import annotations

import pytest

from wexport.db.slim import ChatDB, FrozenTime, tzdata_version
from wexport.errors import UnreadableInput

WINTER = 1794922325000  # 2026-11-17T13:32:05Z
SUMMER = 1783000000000  # 2026-07-02T13:46:40Z, the same zone, the other offset


def test_winter_offset():
    assert FrozenTime("Europe/Rome").local(WINTER) == "2026-11-17 14:32:05 +01:00"


def test_summer_offset_is_computed_once_from_the_zone():
    assert FrozenTime("Europe/Rome").local(SUMMER).endswith("+02:00")


def test_utc_is_available_deliberately():
    assert FrozenTime("UTC").local(WINTER) == "2026-11-17 13:32:05 +00:00"


def test_unknown_zone_is_refused():
    with pytest.raises(UnreadableInput):
        FrozenTime("Europe/Atlantis")


def test_chat_db_records_the_zone_and_tzdata(tmp_path):
    path = tmp_path / "chat.db"
    with ChatDB(path, FrozenTime("Europe/Rome"), "2.26.34.7") as db:
        db.add_chat("1@s.whatsapp.net", "Alice", False)
    import sqlite3

    con = sqlite3.connect(path)
    meta = dict(con.execute("SELECT key, value FROM meta"))
    assert meta["tz"] == "Europe/Rome"
    assert meta["tzdata"] == tzdata_version()
    assert meta["source_app_version"] == "2.26.34.7"
