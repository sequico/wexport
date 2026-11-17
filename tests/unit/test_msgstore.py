"""The msgstore adapter: kinds, quotes, edits, revokes, contacts, calls."""

from __future__ import annotations

import sqlite3
from pathlib import Path

import pytest

from wexport.db.msgstore import Msgstore

HERE = Path(__file__).resolve().parent.parent / "fixtures" / "synthetic"


@pytest.fixture()
def store(tmp_path):
    database = tmp_path / "msgstore.db"
    with sqlite3.connect(database) as con:
        con.executescript((HERE / "schema.sql").read_text(encoding="utf-8"))
        con.executescript((HERE / "data.sql").read_text(encoding="utf-8"))
    with Msgstore(database) as opened:
        yield opened


def test_chats_are_named_from_the_messages_when_no_contacts(store):
    names = sorted(chat.name for chat in store.chats())
    assert names == ["393331234567", "Empty chat", "Family"]


def test_contacts_rename_a_one_to_one_chat(store, tmp_path):
    contacts = tmp_path / "wa.db"
    with sqlite3.connect(contacts) as con:
        con.executescript(
            "CREATE TABLE wa_contacts (jid TEXT, display_name TEXT);"
            "INSERT INTO wa_contacts VALUES ('393331234567@s.whatsapp.net', 'Alice');"
        )
    store.apply_contacts(contacts)
    assert "Alice" in [chat.name for chat in store.chats()]


def test_every_kind_is_mapped(store):
    chats = {chat.name: chat for chat in store.chats()}
    family = store.messages(chats["Family"])
    kinds = {message.kind for message in family}
    assert {"text", "image", "audio", "video", "document", "sticker", "gif", "system"} <= kinds

    alice = list(store.messages(chats["393331234567"]))
    kinds = {message.kind for message in alice}
    assert {"text", "deleted", "contact", "call", "image"} <= kinds


def test_quote_edit_and_revoke_are_kept(store):
    chat = next(c for c in store.chats() if c.jid.startswith("393331234567"))
    messages = {message.id: message for message in store.messages(chat)}
    assert messages[2].quoted_id == 1
    assert messages[3].edited is True
    assert messages[14].revoked is True and messages[14].kind == "deleted"


def test_missing_media_is_still_a_media_row(store):
    chat = next(c for c in store.chats() if c.jid.startswith("393331234567"))
    messages = {message.id: message for message in store.messages(chat)}
    assert messages[12].media is not None
    assert messages[12].media.rel_path.endswith("IMG-deleted-from-phone.png")


def test_call_log_is_attributed_to_its_chat(store):
    chat = next(c for c in store.chats() if c.jid.startswith("393331234567"))
    calls = [m for m in store.messages(chat) if m.kind == "call"]
    assert len(calls) == 1
    assert calls[0].payload == {"duration_s": 332, "video": True, "missed": False}


def test_a_link_preview_add_on_does_not_turn_a_message_into_a_poll(store):
    chat = next(c for c in store.chats() if c.jid.startswith("393331234567"))
    messages = {message.id: message for message in store.messages(chat)}
    assert messages[2].kind == "text"  # it has an add-on row, and it is still a text message


def test_reactions_are_read_and_attached(store):
    chat = next(c for c in store.chats() if c.jid == "120363000000000001@g.us")
    messages = {message.id: message for message in store.messages(chat)}
    assert messages[16].reactions == ["👍"]


def test_unknown_schema_fails_loudly(tmp_path):
    database = tmp_path / "msgstore.db"
    with sqlite3.connect(database) as con:
        con.execute("CREATE TABLE message (_id INTEGER PRIMARY KEY, weird TEXT)")
    with pytest.raises(Exception) as error:
        Msgstore(database)
    assert "timestamp" in str(error.value)


def test_a_path_with_a_hash_still_opens(tmp_path):
    """A `#` in the path turns an unquoted URI into a fragment; the connection must still work."""
    folder = tmp_path / "chat #1 (backup)"
    folder.mkdir()
    database = folder / "msgstore.db"
    with sqlite3.connect(database) as con:
        con.execute(
            "CREATE TABLE message (_id INTEGER PRIMARY KEY, timestamp INTEGER,"
            " key_remote_jid TEXT, from_me INTEGER DEFAULT 0)"
        )
        con.execute("INSERT INTO message VALUES (1, 1, '123@s.whatsapp.net', 0)")
    with Msgstore(database) as store:
        assert [chat.jid for chat in store.chats()] == ["123@s.whatsapp.net"]
