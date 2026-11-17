"""Package marker for wexport's database layer, plus the one shared way to open a database read-only."""

from __future__ import annotations

import sqlite3
from pathlib import Path
from urllib.parse import quote


def ro_connect(path: str | Path) -> sqlite3.Connection:
    """A read-only connection that survives spaces, `#` and `?` in the path.

    The URI form needs the path quoted: an unquoted `#` turns the rest of the path into a fragment
    and SQLite reports "unable to open database file" for a file that is right there.
    """
    return sqlite3.connect(f"file:{quote(str(Path(path).resolve()))}?mode=ro", uri=True)
