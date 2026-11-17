"""crypt15 → msgstore.db, and nothing else.

The payload is zlib-compressed SQLite, or a ZIP when the backup is incremental or multi-file
(docs/android-backup-formats.md §2). Both shapes end as a single SQLite file.
"""

from __future__ import annotations

import io
import zipfile
import zlib
from pathlib import Path

from ..errors import UnreadableInput, UnusableKey


def decrypt_to(src: str | Path, dest: str | Path, key) -> Path:
    """Decrypt `src` into `dest` and return `dest`. Raises `UnreadableInput` on any failure."""
    from wa_crypt_tools import DatabaseFactory, IntegrityError, WaCryptError

    src, dest = Path(src), Path(dest)
    try:
        with src.open("rb") as stream:
            database = DatabaseFactory.from_file(stream)
            payload = database.decrypt(key, stream.read())
    except IntegrityError as exc:
        raise UnusableKey(
            f"{src.name}: the key does not match this backup, or the backup is corrupt.\n"
            f"({exc})\n"
            "Check the 64 digits, or use a backup made after the key was created."
        ) from exc
    except WaCryptError as exc:
        raise UnreadableInput(f"{src.name} cannot be decrypted: {exc}") from exc

    plain = _decompress(payload, src)
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(plain)
    return dest


def _decompress(payload: bytes, src: Path) -> bytes:
    try:
        data = zlib.decompress(payload)
    except zlib.error as exc:
        raise UnreadableInput(f"{src.name}: payload is not zlib data ({exc})") from exc
    if data[:4] == b"PK\x03\x04":
        return _from_zip(data, src)
    if not data.startswith(b"SQLite format 3\x00"):
        raise UnreadableInput(f"{src.name}: decrypted payload is not a SQLite database")
    return data


def _from_zip(data: bytes, src: Path) -> bytes:
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        names = [n for n in archive.namelist() if n.endswith(".db")]
        if not names:
            raise UnreadableInput(f"{src.name}: the ZIP payload holds no .db file")
        # The msgstore is the biggest database in the archive; the others are contacts and settings.
        name = max(names, key=lambda n: archive.getinfo(n).file_size)
        return archive.read(name)
