"""The CLI: two commands, five exit codes, no network.

`check` answers "can this backup be opened, and by what"; `build` writes the archives. There is no
server command: an archive is opened by double-clicking it (plan §2, §6).
"""

from __future__ import annotations

import argparse
import os
import sys
import tempfile
import unicodedata
from pathlib import Path

from . import __version__, preflight
from .backup.decrypt import decrypt_to
from .backup.merge import merge
from .db.msgstore import Chat, Msgstore
from .db.slim import FrozenTime
from .errors import IncompleteBuild, UnreadableInput, UnusableKey, WexError
from .keys import load_key
from .site.build import build_chat, write_index

CHECK_FLAGS = [
    {
        "names": ["input"],
        "help": "the folder copied from the phone; it must contain Databases/msgstore.db.crypt15 "
        "(WhatsApp or WhatsApp Business, the Android/media path or the older sdcard one)",
    },
    {
        "names": ["--key"],
        "metavar": "HEX|IMAGE|FILE",
        "help": "the 64 hex digits (spaces or a 4x4 grid are fine), a screenshot of them, or the "
        "phone's encrypted_backup.key. Needed to verify the key or to list chats: names live "
        "inside the encrypted database",
    },
    {
        "names": ["--chats"],
        "action": "store_true",
        "help": "list the conversations instead of building: last event with its offset, message "
        "count, media count, media weight, name and JID. Nothing is written to disk",
    },
    {
        "names": ["--sort"],
        "choices": ["last", "messages", "size", "name"],
        "default": "last",
        "help": "listing order: last = newest event first (default), messages = most messages "
        "first, size = heaviest media first, name = alphabetical",
    },
    {
        "names": ["--reverse"],
        "action": "store_true",
        "help": "invert the chosen order, e.g. --sort size --reverse = lightest first",
    },
    {
        "names": ["--match"],
        "metavar": "TEXT",
        "help": "keep only chats whose name or JID contains TEXT; case and accents are ignored",
    },
    {
        "names": ["--limit"],
        "type": int,
        "default": 50,
        "metavar": "N",
        "help": "print at most N chats; 0 prints every chat (default 50, the rest is summed and "
        "reported)",
    },
    {
        "names": ["--tz"],
        "metavar": "AREA/CITY",
        "help": "IANA timezone used to render the timestamps, e.g. Europe/Rome (default: this "
        "machine's zone, and the build records whichever is used)",
    },
]

BUILD_FLAGS = [
    {
        "names": ["input"],
        "help": "the folder copied from the phone (Databases/, optionally Backups/ and Media/)",
    },
    {
        "names": ["--key"],
        "required": True,
        "metavar": "HEX|IMAGE|FILE",
        "help": "the 64 hex digits, a screenshot of them, or the phone's encrypted_backup.key",
    },
    {
        "names": ["--out"],
        "required": True,
        "metavar": "DIR",
        "help": "where the archive folders go: one folder per chat, dated and slugged",
    },
    {
        "names": ["--all"],
        "action": "store_true",
        "exclusive": "selection",
        "help": "build every chat (the default when --chat is absent)",
    },
    {
        "names": ["--chat"],
        "action": "append",
        "metavar": "JID|NAME",
        "exclusive": "selection",
        "help": "build only these chats; repeatable; each value is a JID or part of the chat name, "
        "case and accents ignored",
    },
    {
        "names": ["--index"],
        "action": "store_true",
        "help": "also write <out>/index.html listing the built chats; it stays outside every chat "
        "folder, so sharing one chat never reveals the others",
    },
    {
        "names": ["--jobs"],
        "type": int,
        "default": 4,
        "metavar": "N",
        "help": "workers used for thumbnails, posters and waveform peaks (default 4)",
    },
    {
        "names": ["--tz"],
        "metavar": "AREA/CITY",
        "help": "IANA timezone used to render every timestamp, e.g. Europe/Rome (default: this "
        "machine's zone; the frozen strings never change afterwards)",
    },
    {
        "names": ["--transcode"],
        "choices": ["none", "audio", "video", "all"],
        "default": "none",
        "help": "re-encode media for browsers that are picky: none (default), audio = M4A beside "
        "the Opus voice note, video = 720p H.264 beside the original, all = both",
    },
    {
        "names": ["--link"],
        "action": "store_true",
        "help": "hardlink media instead of copying it (same filesystem only; falls back to a copy)",
    },
]

COMMANDS = {
    "check": {
        "summary": "inspect a copied backup: what it is, whether it opens, and which chats it holds",
        "usage": "wexport check <input-dir> [options]",
        "flags": CHECK_FLAGS,
    },
    "build": {
        "summary": "write one self-contained archive folder per chat",
        "usage": "wexport build <input-dir> --key <key> --out <dir> [options]",
        "flags": BUILD_FLAGS,
    },
}

EXIT_CODES = [
    ("0", "success"),
    ("1", "unreadable input: the folder, the backup file or the database cannot be used"),
    ("2", "unusable key: not a key, or the key does not open this backup"),
    ("3", "wrong backup type: crypt14, password-based or passkey-based backup"),
    ("4", "build incomplete: some media could not be processed, the archive is still usable"),
    ("5", "missing external dependency: ffmpeg, ffprobe, or tesseract for a screenshot key"),
]

EXAMPLES = [
    "wexport check ~/whatsapp/phone-copy --key <64 hex digits> --chats --sort size --limit 20",
    "wexport check ~/whatsapp/phone-copy --key key.png --chats --match family --limit 0",
    "wexport build ~/whatsapp/phone-copy --key key.png --out ~/whatsapp/archives --index",
    "wexport build ~/whatsapp/phone-copy --key key.png --out ~/whatsapp/archives --chat Family",
]


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="wexport",
        description="Turn an Android WhatsApp crypt15 backup into self-contained archives that open by "
        "double-clicking index.html: no server, no wexport and no network on the reading machine. "
        "It never uploads anything and never touches the phone.",
        epilog=_reference(),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--version", action="version", version=f"wexport {__version__}")
    sub = parser.add_subparsers(dest="command", required=True, metavar="{check,build}")

    for name, command in COMMANDS.items():
        command_parser = sub.add_parser(
            name,
            help=command["summary"],
            description=command["summary"],
            usage=command["usage"],
            formatter_class=argparse.RawDescriptionHelpFormatter,
        )
        exclusive: dict[str, argparse._MutuallyExclusiveGroup] = {}
        groups: dict[str, argparse._ArgumentGroup] = {}
        for flag in command["flags"]:
            kwargs = {
                key: value
                for key, value in flag.items()
                if key not in {"names", "exclusive", "group"}
            }
            target: argparse._ActionsContainer = command_parser
            if "exclusive" in flag:
                target = exclusive.setdefault(
                    flag["exclusive"], command_parser.add_mutually_exclusive_group()
                )
            elif "group" in flag:
                target = groups.setdefault(flag["group"], command_parser.add_argument_group(flag["group"]))
            target.add_argument(*flag["names"], **kwargs)
    return parser


def _flag_label(flag: dict) -> str:
    """How a flag is spelled in the reference: the one source for both parser and help."""
    names = flag["names"]
    if not names[0].startswith("-"):
        return flag.get("metavar") or names[0].upper()
    head = " | ".join(names)
    if flag.get("action") in {"store_true", "store_false", "version", "help"}:
        return head
    metavar = flag.get("metavar")
    if metavar is None and flag.get("choices"):
        metavar = "{" + ",".join(str(choice) for choice in flag["choices"]) + "}"
    return f"{head} {metavar or head.lstrip('-').upper()}"


def _reference() -> str:
    """The full reference that `wexport -h` prints, generated from the same flags the parser uses."""
    import textwrap

    lines = []
    for name, command in COMMANDS.items():
        lines.append("")
        lines.append(f"wexport {name} options — {command['usage']}")
        for flag in command["flags"]:
            label = _flag_label(flag)
            wrapped = textwrap.wrap(flag["help"], width=96)
            lines.append(f"  {label}")
            lines.extend(f"      {line}" for line in wrapped)
    lines.append("")
    lines.append("exit codes:")
    lines.extend(f"  {code}  {text}" for code, text in EXIT_CODES)
    lines.append("")
    lines.append("requirements: ffmpeg and ffprobe on PATH (thumbnails, posters, video metadata);")
    lines.append("tesseract only when the key is given as a screenshot.")
    lines.append("")
    lines.append("examples:")
    lines.extend(f"  {example}" for example in EXAMPLES)
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "check":
            return _check(args)
        return _build(args)
    except WexError as error:
        print(f"wexport: {error}", file=sys.stderr)
        return error.exit_code


# ---------------------------------------------------------------- check


def _check(args: argparse.Namespace) -> int:
    # The report first: `require_binaries` tells the user to run `check`, which is only useful if
    # this command prints the environment before it refuses.
    for line in preflight.environment_report():
        print(line)
    preflight.require_binaries("ffmpeg", "ffprobe")

    inputs = preflight.discover(args.input)
    header = preflight.read_header(inputs.database)
    print()
    print(f"backup        {header.path}")
    print(f"format        {header.format}")
    print(f"app version   {header.app_version or 'unknown'}")
    print(f"key type      {header.key_type or 'unknown'}")
    print(f"size          {header.size / 1e6:.1f} MB")
    print(f"incrementals  {len(inputs.incrementals)}")
    for increment in inputs.incrementals:
        print(f"              {increment.name}")
    if inputs.ignored:
        print(f"ignored       {len(inputs.ignored)} file(s) from the older backup chain (crypt14)")
        for path in inputs.ignored[:10]:
            print(f"              {path.name}")
        if len(inputs.ignored) > 10:
            print(f"              … and {len(inputs.ignored) - 10} more")
    print(f"contacts      {inputs.contacts.name if inputs.contacts else 'none (numbers will be shown)'}")
    print(f"media folder  {inputs.media if inputs.media else 'MISSING (every attachment will be a placeholder)'}")

    preflight.require_openable(header)
    if args.chats and not args.key:
        raise UnusableKey(
            "listing the chats needs --key: the names live inside the encrypted database.\n"
            "Give the 64 digits, the screenshot, or encrypted_backup.key."
        )
    if args.key:
        key = load_key(args.key)
        print("\nverifying    the key against the backup (this decrypts it once)")
        with tempfile.TemporaryDirectory(prefix="wexport-check-") as workspace:
            probe = Path(workspace) / "probe.db"
            decrypt_to(inputs.database, probe, key)
            if args.chats:
                _list_chats(inputs, probe, workspace, key, args)
                return 0
        print("key           verified against this backup")
    print()
    print("ready to build: wexport build <input> --key <key> --out <folder>")
    return 0


# ---------------------------------------------------------------- listing


def _list_chats(inputs, database: Path, workspace: str, key, args: argparse.Namespace) -> None:
    """Print the chats with what matters for choosing one, in the order asked for."""
    time = FrozenTime(args.tz or _system_zone())
    with Msgstore(database) as store:
        if inputs.contacts is not None:
            contacts = _contacts_database(inputs.contacts, workspace, key)
            if contacts is not None:
                store.apply_contacts(contacts)
        stats = store.chat_stats()

    def last(stat) -> float:
        return stat.last_ts

    sorters = {
        "last": (last, True),
        "messages": (lambda stat: stat.messages, True),
        "size": (lambda stat: stat.media_bytes, True),
        "name": (lambda stat: stat.chat.name.lower(), False),
    }
    keyfunc, descending = sorters[args.sort]
    if args.reverse:
        descending = not descending
    stats.sort(key=lambda stat: (keyfunc(stat), stat.chat.name.lower()), reverse=descending)

    if args.match:
        needle = _fold(args.match)
        stats = [
            stat
            for stat in stats
            if needle in _fold(stat.chat.name) or needle in _fold(stat.chat.jid)
        ]
    matched = len(stats)
    shown = stats if args.limit <= 0 else stats[: args.limit]

    width = max((len(stat.chat.name) for stat in shown), default=8)
    print(f"\n{'last message':<27} {'msgs':>7} {'media':>6} {'size':>9}  {'chat':<{width}}  jid")
    for stat in shown:
        stamp = time.local(stat.last_ts) if stat.last_ts else "—"
        print(
            f"{stamp:<27} {stat.messages:>7} {stat.media_count:>6} "
            f"{_human_bytes(stat.media_bytes):>9}  {stat.chat.name:<{width}}  {stat.chat.jid}"
        )
    total_messages = sum(stat.messages for stat in stats)
    total_bytes = sum(stat.media_bytes for stat in stats)
    print(
        f"\n{len(shown)} of {matched} chat(s) shown, {total_messages} message(s), "
        f"{_human_bytes(total_bytes)} of media"
        + ("" if len(shown) == matched else " — use --limit 0 for all, --match to filter")
    )
    print("Build one or more: wexport build <input> --key <key> --out <folder> --chat <name|jid>")


def _fold(text: str) -> str:
    """Case- and accent-insensitive, so 'caffe' matches 'Caffè'."""
    return unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode().lower()


def _human_bytes(count: int) -> str:
    size = float(count)
    for unit in ("B", "kB", "MB", "GB"):
        if size < 1024 or unit == "GB":
            return f"{size:.0f} {unit}" if unit == "B" else f"{size:.1f} {unit}"
        size /= 1024
    return f"{size:.1f} GB"  # pragma: no cover - the loop always returns


# ---------------------------------------------------------------- build


def _build(args: argparse.Namespace) -> int:
    preflight.require_binaries("ffmpeg", "ffprobe")
    inputs = preflight.discover(args.input)
    header = preflight.read_header(inputs.database)
    preflight.require_openable(header)

    key = load_key(args.key)
    out_root = Path(args.out).expanduser().resolve()
    out_root.mkdir(parents=True, exist_ok=True)
    time = FrozenTime(args.tz or _system_zone())

    with tempfile.TemporaryDirectory(prefix="wexport-") as workspace:
        database = Path(workspace) / "msgstore.db"
        print(f"decrypting   {inputs.database.name}")
        decrypt_to(inputs.database, database, key)
        if inputs.incrementals:
            print(f"merging      {len(inputs.incrementals)} incremental file(s)")
            deltas = []
            for index, increment in enumerate(inputs.incrementals, start=1):
                plain = Path(workspace) / f"increment-{index}.db"
                decrypt_to(increment, plain, key)
                deltas.append(plain)
            merge(database, deltas)

        with Msgstore(database) as store:
            if inputs.contacts is not None:
                contacts = _contacts_database(inputs.contacts, workspace, key)
                if contacts is not None:
                    store.apply_contacts(contacts)
            chats = _select_chats(store, args)
            if not chats:
                names = ", ".join(chat.name for chat in store.chats()[:10])
                raise UnreadableInput(
                    f"no chat matched {', '.join(args.chat or [])}.\nAvailable: {names or 'none'}"
                )
            print(f"building     {len(chats)} chat(s) into {out_root}")
            summaries = []
            for chat in chats:
                summary = build_chat(
                    msgstore=store,
                    chat=chat,
                    out_root=out_root,
                    time=time,
                    app_version=header.app_version,
                    copy_root=inputs.root,
                    media_root=inputs.media,
                    link=args.link,
                    transcode=args.transcode,
                    jobs=max(1, args.jobs),
                )
                summaries.append(summary)
                if summary.empty:
                    print(f"  {summary.name}: skipped, no messages")
                    continue
                print(
                    f"  {summary.archive.name}: {summary.messages} messages, "
                    f"{summary.media} media ({summary.missing} missing), "
                    f"{summary.bytes / 1e6:.1f} MB"
                )
                if summary.failure_count:
                    print(f"    {summary.failure_count} file(s) could not be processed")
                    for failure in summary.failures[:5]:
                        print(f"      {failure}")

    if args.index:
        page = write_index(out_root, summaries)
        print(f"index        {page}")

    failures = sum(summary.failure_count for summary in summaries)
    built = [summary for summary in summaries if not summary.empty]
    skipped = len(summaries) - len(built)
    print()
    print(f"done: {len(built)} archive(s) in {out_root}" + (f", {skipped} empty chat(s) skipped" if skipped else ""))
    if failures:
        print(
            f"{failures} file(s) could not be processed; the archives are usable (exit {IncompleteBuild.exit_code})",
            file=sys.stderr,
        )
        return IncompleteBuild.exit_code
    return 0


def _contacts_database(contacts: Path, workspace: str, key) -> Path | None:
    """wa.db is crypt15 too on recent phones; decrypt it when needed, else use it as-is."""
    if contacts.suffix == ".crypt15":
        target = Path(workspace) / "wa.db"
        try:
            decrypt_to(contacts, target, key)
        except WexError as error:
            print(f"warning: contacts not readable — names will be numbers ({error})", file=sys.stderr)
            return None
        return target
    return contacts


def _select_chats(store: Msgstore, args: argparse.Namespace) -> list[Chat]:
    chats = store.chats()
    wanted = getattr(args, "chat", None)
    if not wanted:
        return chats
    lowered = [w.lower() for w in wanted]
    selected = [
        chat
        for chat in chats
        if chat.jid.lower() in lowered or any(w in chat.name.lower() for w in lowered)
    ]
    return selected


def _system_zone() -> str:
    """Best effort, and never silent about it: the zone ends up in every frozen timestamp."""
    for candidate in ("/etc/timezone",):
        path = Path(candidate)
        if path.is_file():
            name = path.read_text(encoding="utf-8", errors="replace").strip()
            if name:
                return name
    try:
        parts = Path("/etc/localtime").resolve().parts
        return "/".join(parts[parts.index("zoneinfo") + 1 :]) or "UTC"
    except (OSError, ValueError):
        return os.environ.get("TZ") or "UTC"


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
