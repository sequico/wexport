"""The CLI: two commands, five exit codes, no network.

`check` answers "can this backup be opened, and by what"; `build` writes the archives. There is no
server command: an archive is opened by double-clicking it (plan §2, §6).
"""

from __future__ import annotations

import argparse
import os
import sys
import tempfile
from pathlib import Path

from . import __version__, preflight
from .backup.decrypt import decrypt_to
from .backup.merge import merge
from .db.msgstore import Chat, Msgstore
from .db.slim import FrozenTime
from .errors import IncompleteBuild, UnreadableInput, UnusableKey, WexError
from .keys import load_key
from .site.build import build_chat, write_index


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="wexport",
        description="Turn an Android WhatsApp crypt15 backup into a self-contained offline chat "
        "archive. Every archive opens by double-clicking its index.html; nothing is installed.",
    )
    parser.add_argument("--version", action="version", version=f"wexport {__version__}")
    sub = parser.add_subparsers(dest="command", required=True)

    check = sub.add_parser("check", help="report what the backup contains and whether it can be opened")
    check.add_argument("input", help="the folder copied from the phone (contains Databases/)")
    check.add_argument("--key", help="64 hex digits, a screenshot, or encrypted_backup.key")
    check.add_argument("--chats", action="store_true", help="list the chats without building anything")
    check.add_argument(
        "--sort",
        choices=["last", "messages", "size", "name"],
        default="last",
        help="listing order: newest last message, most messages, heaviest media, or name",
    )
    check.add_argument("--reverse", action="store_true", help="flip the listing order")
    check.add_argument("--tz", default=None, help="IANA timezone for the printed timestamps")

    build = sub.add_parser("build", help="write one archive folder per chat")
    build.add_argument("input", help="the folder copied from the phone")
    build.add_argument("--key", required=True, help="64 hex digits, a screenshot, or encrypted_backup.key")
    build.add_argument("--out", required=True, help="where to write the archive folders")
    group = build.add_mutually_exclusive_group()
    group.add_argument("--all", action="store_true", help="every chat (default)")
    group.add_argument("--chat", action="append", metavar="JID|NAME", help="only these chats; repeatable")
    build.add_argument("--index", action="store_true", help="also write <out>/index.html listing the chats")
    build.add_argument("--jobs", type=int, default=4, help="media workers (default 4)")
    build.add_argument("--tz", default=None, help="IANA timezone for the rendered timestamps")
    build.add_argument(
        "--transcode",
        choices=["none", "audio", "video", "all"],
        default="none",
        help="re-encode media for picky browsers (default none)",
    )
    build.add_argument("--link", action="store_true", help="hardlink media instead of copying")
    return parser


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

    width = max((len(stat.chat.name) for stat in stats), default=8)
    print(f"\n{'last message':<27} {'msgs':>7} {'media':>6} {'size':>9}  {'chat':<{width}}  jid")
    for stat in stats:
        stamp = time.local(stat.last_ts) if stat.last_ts else "—"
        print(
            f"{stamp:<27} {stat.messages:>7} {stat.media_count:>6} "
            f"{_human_bytes(stat.media_bytes):>9}  {stat.chat.name:<{width}}  {stat.chat.jid}"
        )
    total_messages = sum(stat.messages for stat in stats)
    total_bytes = sum(stat.media_bytes for stat in stats)
    print(
        f"\n{len(stats)} chat(s), {total_messages} message(s), "
        f"{_human_bytes(total_bytes)} of media\n"
        "Build one or more: wexport build <input> --key <key> --out <folder> --chat <name|jid>"
    )


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
