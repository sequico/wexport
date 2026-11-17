"""Decide, before any work, whether this input can be opened — and with what.

This module never decrypts. It reads the crypt15 header through the pinned library, and it is
the only place that turns a backup into one of the plan's exit codes (plan §2).
"""

from __future__ import annotations

import os
import platform
import shutil
from dataclasses import dataclass
from pathlib import Path

from .errors import MissingDependency, UnreadableInput, WrongBackupType

# The key types we can open offline. Everything else is recognised and refused (docs/android-backup-formats.md §3).
OPENABLE_KEY_TYPES = {"E2EE_ENCRYPTION_KEY"}

_INSTALL_HINTS = {
    "ffmpeg": [
        ("Debian/Ubuntu", "sudo apt install ffmpeg"),
        ("Fedora/RHEL", "sudo dnf install ffmpeg"),
        ("Arch", "sudo pacman -S ffmpeg"),
        ("Windows", "winget install Gyan.FFmpeg"),
    ],
    "ffprobe": [
        ("Debian/Ubuntu", "sudo apt install ffmpeg"),
        ("Fedora/RHEL", "sudo dnf install ffmpeg"),
        ("Arch", "sudo pacman -S ffmpeg"),
        ("Windows", "winget install Gyan.FFmpeg"),
    ],
}


@dataclass(frozen=True)
class Header:
    path: Path
    format: str  # crypt12 / crypt14 / crypt15
    app_version: str
    key_type: str
    size: int


@dataclass(frozen=True)
class Inputs:
    root: Path
    database: Path
    incrementals: tuple[Path, ...]
    contacts: Path | None
    media: Path | None
    ignored: tuple[Path, ...] = ()  # same family, other backup chain: crypt14 next to a crypt15


def discover(root: str | Path) -> Inputs:
    """Find the backup inside a folder the user copied from the phone."""
    root = Path(root).expanduser().resolve()
    if root.is_file():  # someone pointed straight at msgstore.db.crypt15
        root = root.parent
    if not root.is_dir():
        raise UnreadableInput(f"not a folder: {root}")

    databases = _find_databases(root)
    if databases is None:
        raise UnreadableInput(
            f"no Databases/ folder with a msgstore backup under {root}\n"
            "Point me at the folder you copied from the phone: it must contain Databases/msgstore.db.crypt15.\n"
            "Copy Android/media/com.whatsapp/WhatsApp (or WhatsApp Business) and try again."
        )

    crypt15 = sorted(databases.glob("msgstore*.crypt15"))
    crypt14 = sorted(databases.glob("msgstore*.crypt14"))
    if not crypt15:
        if crypt14:
            raise WrongBackupType(
                f"only crypt14 backups in {databases} — end-to-end encryption is off, or the\n"
                "backup was never redone after enabling it. Turn on end-to-end encrypted backup\n"
                "with the 64-digit key, run a full backup, copy the folder again."
            )
        raise UnreadableInput(f"no msgstore.db.crypt15 in {databases}")

    # The full backup is msgstore.db.crypt15; only `msgstore-increment-*` are deltas to replay.
    # The rolling `msgstore-YYYY-MM-DD.1.db.crypt15` copies are superseded full snapshots: merging
    # one back in could resurrect messages WhatsApp has already dropped.
    full = next((p for p in crypt15 if p.name == "msgstore.db.crypt15"), None)
    if full is None:
        full = max(crypt15, key=lambda p: p.stat().st_mtime)
    if full.name.startswith("msgstore-increment"):
        raise UnreadableInput(
            f"{databases} has only incremental files and no full backup.\n"
            "Open WhatsApp → Settings → Chats → Chat Backup → Back up now, then copy the folder again."
        )
    incrementals = tuple(p for p in crypt15 if p != full and p.name.startswith("msgstore-increment"))
    # A folder usually still holds the previous chain (crypt14 files from before E2EE was turned on).
    # They are not ours to read, and silence about them is what makes a person pick the wrong file.
    ignored = tuple(p for p in crypt14 if p.name.startswith("msgstore"))

    # The media and the contacts sit beside the backup, not necessarily beside the folder the user
    # pointed at: look in both places (a copy can be rooted at the phone folder or one level down).
    contacts: Path | None = None
    media: Path | None = None
    for candidate_root in (root, databases.parent):
        backups = _first_dir(candidate_root, "Backups", "WhatsApp/Backups", "WhatsApp Business/Backups")
        if backups is not None and contacts is None:
            for name in ("wa.db.crypt15", "wa.db"):
                candidate = backups / name
                if candidate.is_file():
                    contacts = candidate
                    break
        media = media or _first_dir(candidate_root, "Media", "WhatsApp/Media", "WhatsApp Business/Media")

    return Inputs(
        root=root,
        database=full,
        incrementals=incrementals,
        contacts=contacts,
        media=media,
        ignored=ignored,
    )


def _first_dir(root: Path, *candidates: str) -> Path | None:
    for rel in candidates:
        path = root / rel
        if path.is_dir():
            return path
    return None


def _find_databases(root: Path) -> Path | None:
    """The folder holding the backup, wherever the copy put it.

    A phone copy can arrive as `<root>/Databases`, as `<root>/WhatsApp/Databases`, as the whole
    `Android/media/com.whatsapp/WhatsApp/Databases` chain, or as the folder the file lives in —
    an MTP drag is not obliged to preserve any of it. Look a few levels down, skip the media tree
    (tens of thousands of files that cannot contain a backup), and never guess beyond that.
    """
    if root.name in {"Databases", "databases"} and any(root.glob("msgstore*.crypt1*")):
        return root
    explicit = _first_dir(root, "Databases", "WhatsApp/Databases", "WhatsApp Business/Databases")
    if explicit is not None:
        return explicit

    max_depth = 6
    skip = {"node_modules", ".git"}
    for current, directories, files in os.walk(root):
        depth = len(Path(current).relative_to(root).parts)
        if depth >= max_depth:
            directories[:] = []
            continue
        directories[:] = [d for d in directories if d not in skip]
        if any(name.startswith("msgstore") and ".crypt1" in name for name in files):
            return Path(current)
    return None


def read_header(path: str | Path) -> Header:
    """Parse the header without decrypting a byte."""
    from wa_crypt_tools import DatabaseFactory

    path = Path(path)
    try:
        with path.open("rb") as stream:
            database = DatabaseFactory.from_file(stream)
    except Exception as exc:
        raise UnreadableInput(f"{path.name} is not a readable WhatsApp backup: {exc}") from exc

    kind = type(database).__name__  # Database12 / Database14 / Database15
    key_type = _key_type_name(getattr(database, "key_type", "") or "")
    app_version = ""
    props = getattr(database, "props", None)
    if props is not None:
        app_version = getattr(props, "get_wa_version", lambda: "")() or ""
    return Header(
        path=path,
        format="crypt" + kind.removeprefix("Database"),
        app_version=app_version,
        key_type=str(key_type),
        size=path.stat().st_size,
    )


def _key_type_name(value: object) -> str:
    """The library hands back the wire enum number; say the name a person can read."""
    from wa_crypt_tools.proto import backup_prefix_pb2 as prefix

    try:
        enum = prefix.BackupPrefix.DESCRIPTOR.fields_by_name["key_type_new"].enum_type
        return enum.values_by_number[int(value)].name  # type: ignore[arg-type]
    except (KeyError, TypeError, ValueError, AttributeError):
        return str(value)


def require_openable(header: Header) -> None:
    """Refuse what cannot be decrypted offline, naming the way out.

    A crypt14 folder is already refused by `discover`, with the folder it looked at; this function
    only has to judge the key type of a header that was parsed.
    """
    name = Path(header.path).name
    if header.format != "crypt15":
        raise WrongBackupType(f"{name} is {header.format}: only crypt15 backups can be opened offline.")
    if header.key_type in OPENABLE_KEY_TYPES:
        return
    if header.key_type == "E2EE_PASSWORD":
        raise WrongBackupType(
            f"{name} was made with a password, not the 64-digit key.\n"
            "The password only asks WhatsApp for the key, so this file cannot be opened offline.\n"
            "Re-enable end-to-end encrypted backup choosing the 64-digit key and make a new backup;\n"
            "older backups stay closed, and only the newest one is needed."
        )
    if header.key_type == "E2EE_PASSKEY":
        raise WrongBackupType(
            f"{name} was made with a passkey, not the 64-digit key.\n"
            "There is no offline way to open it: re-enable end-to-end encrypted backup with the\n"
            "64-digit key and make a new backup."
        )
    raise WrongBackupType(
        f"{name} uses key type {header.key_type or 'unknown'}, which this tool cannot open.\n"
        "Make a fresh backup after enabling end-to-end encrypted backup with the 64-digit key."
    )


def require_binaries(*names: str) -> None:
    """Abort with the install command when an external binary is missing (plan §2)."""
    for name in names:
        if shutil.which(name):
            continue
        hints = _INSTALL_HINTS.get(name, [])
        lines = "\n".join(f"  {system:<15} {cmd}" for system, cmd in hints)
        raise MissingDependency(
            f"{name} not found on PATH.\n{lines}\n"
            f"Then re-run the same command. `wexport check` prints the full environment report."
        )


def environment_report() -> list[str]:
    """The lines `wexport check` prints before anything else."""
    rows = [f"platform      {platform.system()} {platform.machine()}"]
    for name in ("ffmpeg", "ffprobe", "tesseract"):
        found = shutil.which(name)
        rows.append(f"{name:<13} {found or 'MISSING'}")
    return rows
