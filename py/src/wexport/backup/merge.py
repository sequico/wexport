"""Replay incremental backups into the full one.

Incrementals are SQLite deltas — rows for the tables WhatsApp changed since the last full backup —
so both arguments are already decrypted databases. Merging is row-level, oldest delta first.

shortcut: INSERT OR REPLACE by primary key, not a semantic replay. A message deleted from the
phone is absent from the delta rather than marked, so it stays in the archive. Upgrade when a
real incremental proves this wrong (plan §10, "incremental merge ordering").
"""

from __future__ import annotations

import sqlite3
from collections.abc import Iterable
from pathlib import Path

from ..errors import IncompleteBuild, UnreadableInput

# Tables we read, in dependency order for later reference; merged generically anyway.
MERGED_TABLES = (
    "jid",
    "chat",
    "message",
    "message_text",
    "message_media",
    "message_quoted",
    "message_revoked",
    "message_add_on",
    "message_add_on_reaction",
    "message_system",
    "message_location",
    "message_vcard",
    "receipt_user",
    "group_participant_user",
    "call_log",
)


def merge(base: str | Path, increments: Iterable[str | Path]) -> Path:
    """Apply deltas in order. Returns `base`, which is modified in place."""
    base = Path(base)
    for inc in sorted((Path(p) for p in increments), key=lambda p: p.name):
        _apply_one(base, inc)
    return base


def _apply_one(base: Path, inc: Path) -> None:
    con: sqlite3.Connection | None = None
    try:
        con = sqlite3.connect(base)
        con.execute("ATTACH DATABASE ? AS inc", (str(inc),))
    except sqlite3.Error as exc:
        if con is not None:
            con.close()
        raise UnreadableInput(f"{inc.name} cannot be attached: {exc}") from exc

    try:
        for table in MERGED_TABLES:
            if not _has(con, "inc", table):
                continue
            if not _has(con, "main", table):
                raise IncompleteBuild(
                    f"{inc.name} carries table {table}, which the full backup does not have.\n"
                    "The delta comes from a different WhatsApp version. Make a fresh full backup\n"
                    "and copy the folder again."
                )
            base_cols = _columns(con, "main", table)
            inc_cols = _columns(con, "inc", table)
            if base_cols != inc_cols:
                raise IncompleteBuild(
                    f"{inc.name}: table {table} has different columns from the full backup\n"
                    f"  full: {', '.join(base_cols)}\n  delta: {', '.join(inc_cols)}\n"
                    "Make a fresh full backup and copy the folder again."
                )
            quoted = f'"{table}"'
            con.execute(f"INSERT OR REPLACE INTO main.{quoted} SELECT * FROM inc.{quoted}")
        con.commit()
    finally:
        con.execute("DETACH DATABASE inc")
        con.close()


def _has(con: sqlite3.Connection, schema: str, table: str) -> bool:
    row = con.execute(
        f"SELECT 1 FROM {schema}.sqlite_master WHERE type='table' AND name=?", (table,)
    ).fetchone()
    return row is not None


def _columns(con: sqlite3.Connection, schema: str, table: str) -> tuple[str, ...]:
    return tuple(row[1] for row in con.execute(f"PRAGMA {schema}.table_info('{table}')"))
