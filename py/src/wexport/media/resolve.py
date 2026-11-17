"""Find the file a row points at, and put one copy of it in the archive.

The database stores paths relative to the WhatsApp root (`Media/WhatsApp Images/…`), so the
lookup is against the folder the user copied, not against a guess (plan §5).
"""

from __future__ import annotations

import hashlib
import os
import shutil
import threading
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Placed:
    """Where a media file ended up inside the archive."""

    sha256: str
    rel_path: str  # relative to the archive root, e.g. media/ab/ab12….jpg
    missing: bool = False


def source_for(rel_path: str, media_root: Path | None, copy_root: Path) -> Path | None:
    """The file on disk for a database path, or None when the user did not copy it.

    The path comes from the backup, so it is a trust boundary: a row pointing at `../../etc/passwd`
    must not be able to pull a file from outside the folder the user copied.
    """
    rel = rel_path.replace("\\", "/").lstrip("/")
    candidates: list[Path] = []
    roots = [copy_root]
    if media_root is not None:
        roots.append(media_root)
        _, _, tail = rel.partition("Media/")
        if tail:
            candidates.append(media_root / tail)
    candidates.extend(root / rel for root in roots)
    for candidate in candidates:
        try:
            resolved = candidate.resolve()
        except OSError:
            continue
        if not any(resolved.is_relative_to(root.resolve()) for root in roots):
            continue
        if resolved.is_file():
            return resolved
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
    if destination.exists():
        return Placed(sha256=digest, rel_path=rel.as_posix())
    destination.parent.mkdir(parents=True, exist_ok=True)
    if link:
        try:
            os.link(source, destination)
            return Placed(sha256=digest, rel_path=rel.as_posix())
        except OSError:
            pass  # another filesystem, or a filesystem without hardlinks: copy instead
    # Two workers can place the same content at once; write beside the target and rename, so the
    # visible file is always complete.
    partial = destination.with_name(f"{destination.name}.{os.getpid()}.{threading.get_ident()}.part")
    shutil.copy2(source, partial)
    os.replace(partial, destination)
    return Placed(sha256=digest, rel_path=rel.as_posix())
