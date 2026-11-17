"""Decide, before any work, whether this input can be opened — and with what.

This module never decrypts. It reads the crypt15 header through the pinned library, and it is
the only place that turns a backup into one of the plan's exit codes (plan §2).
"""

from __future__ import annotations

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


def discover(root: str | Path) -> Inputs:
    """Find the backup inside a folder the user copied from the phone."""
    root = Path(root).expanduser().resolve()
    if not root.is_dir():
        raise UnreadableInput(f"not a folder: {root}")

    databases = _first_dir(root, "Databases", "WhatsApp/Databases", "WhatsApp Business/Databases")
    if databases is None:
        raise UnreadableInput(
            f"no Databases/ folder under {root}\n"
            "Copy Android/media/com.whatsapp/WhatsApp (or WhatsApp Business) and point me at it."
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

    backups = _first_dir(root, "Backups", "WhatsApp/Backups", "WhatsApp Business/Backups")
    contacts = None
    if backups is not None:
        for name in ("wa.db.crypt15", "wa.db"):
            candidate = backups / name
            if candidate.is_file():
                contacts = candidate
                break

    media = _first_dir(root, "Media", "WhatsApp/Media", "WhatsApp Business/Media")
    return Inputs(root=root, database=full, incrementals=incrementals, contacts=contacts, media=media)


def _first_dir(root: Path, *candidates: str) -> Path | None:
    for rel in candidates:
        path = root / rel
        if path.is_dir():
            return path
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
