"""Find the file a row points at, and put one copy of it in the archive.

The database stores paths relative to the WhatsApp root (`Media/WhatsApp Images/…`), so the
lookup is against the folder the user copied, not against a guess (plan §5).
"""

from __future__ import annotations

import hashlib
import os
import shutil
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Placed:
    """Where a media file ended up inside the archive."""

    sha256: str
    rel_path: str  # relative to the archive root, e.g. media/ab/ab12….jpg
    missing: bool = False


def source_for(rel_path: str, media_root: Path | None, copy_root: Path) -> Path | None:
    """The file on disk for a database path, or None when the user did not copy it."""
    rel = rel_path.replace("\\", "/").lstrip("/")
    candidates: list[Path] = [copy_root / rel]
    if media_root is not None:
        _, _, tail = rel.partition("Media/")
        if tail:
            candidates.append(media_root / tail)
        candidates.append(media_root / rel)
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return None


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def place(source: Path, archive_root: Path, link: bool = False) -> Placed:
    """Copy (or hardlink) the file under its content address. One file per content, ever."""
    digest = sha256_of(source)
    extension = source.suffix.lower() or ""
    rel = Path("media") / digest[:2] / f"{digest}{extension}"
    destination = archive_root / rel
    if not destination.exists():
        destination.parent.mkdir(parents=True, exist_ok=True)
        if link:
            try:
                os.link(source, destination)
            except OSError:
                shutil.copy2(source, destination)
        else:
            shutil.copy2(source, destination)
    return Placed(sha256=digest, rel_path=rel.as_posix())
