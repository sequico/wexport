"""The one test that needs real data, and skips on purpose when it has none.

Skipped by design when `WEXPORT_REAL_BACKUP` is unset: a synthetic fixture cannot prove anything
about a real WhatsApp schema, and a stub pretending to would be worse than no test. Set
`WEXPORT_REAL_BACKUP` to a copied phone folder and `WEXPORT_REAL_KEY` to its 64-digit key to run it.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from wexport import preflight
from wexport.backup.decrypt import decrypt_to
from wexport.db.msgstore import Msgstore
from wexport.keys import load_key

REAL = os.environ.get("WEXPORT_REAL_BACKUP")
KEY = os.environ.get("WEXPORT_REAL_KEY")


@pytest.mark.skipif(
    not (REAL and KEY),
    reason="needs WEXPORT_REAL_BACKUP and WEXPORT_REAL_KEY; the synthetic fixture is the default",
)
def test_real_backup_decrypts_and_maps_every_chat(tmp_path):
    root = Path(REAL)
    inputs = preflight.discover(root)
    preflight.require_openable(preflight.read_header(inputs.database))
    database = tmp_path / "msgstore.db"
    decrypt_to(inputs.database, database, load_key(KEY))

    with Msgstore(database) as store:
        chats = store.chats()
        assert chats, "a real backup with no chat is not a backup"
        for chat in chats[:5]:
            messages = list(store.messages(chat))
            assert isinstance(messages, list)  # an empty chat is legal; the read must not raise
