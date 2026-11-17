"""One command: point it at the folder you copied, answer two questions.

`wexport <folder>` finds the backup, finds (or asks for) the key, prints the chats numbered, and
builds what you pick. `--list` only looks, `--chat` and `--yes` skip the questions for scripts.
There is no server: an archive opens by double-clicking it (plan §2, §6).
"""

from __future__ import annotations

import argparse
import contextlib
import os
import subprocess
import sys
import tempfile
import time
import unicodedata
from pathlib import Path

from . import __version__, preflight
from .backup.decrypt import decrypt_to
from .backup.merge import merge
from .db.msgstore import Msgstore
from .db.slim import FrozenTime
from .errors import IncompleteBuild, UnreadableInput, UnusableKey, WexError
from .keys import load_key
from .site.build import build_chat, write_index

DEFAULT_RECENT = 10
_OUTPUT_DIR = "archives"
_OLD_COMMANDS = {"check", "build", "serve", "list"}


FLAGS = [
    {
        "names": ["input"],
        "nargs": "*",
        "metavar": "FOLDER",
        "help": "the folder you copied from the phone. Any shape works: Databases/, WhatsApp/, "
        "Android/media/com.whatsapp/WhatsApp, a folder holding msgstore.db.crypt15, or the file "
        "itself. Default: the current directory",
    },
    {
        "names": ["--key"],
        "metavar": "HEX|IMAGE|FILE",
        "help": "the 64 hex digits, a screenshot, or encrypted_backup.key. Without it the key is "
        "looked up next to the backup (key.png, key.txt, encrypted_backup.key) and, failing that, "
        "asked once on the terminal",
    },
    {
        "names": ["--chat"],
        "action": "append",
        "metavar": "N|NAME",
        "help": "build these chats without asking: row numbers from the list, or part of a name or "
        "JID, comma-separated and repeatable; 'all' for every chat",
    },
    {
        "names": ["--list"],
        "action": "store_true",
        "help": "only print the numbered chat list (last event, messages, media) and exit; nothing "
        "is written",
    },
    {
        "names": ["--yes"],
        "action": "store_true",
        "help": "build every chat without asking, for scripts and unattended runs",
    },
    {
        "names": ["--out"],
        "metavar": "DIR",
        "help": f"where the archive folders go (default: <input>/{_OUTPUT_DIR})",
    },
]

EXIT_CODES = [
    ("0", "success"),
    ("1", "unreadable input, or a question that could not be asked without a terminal"),
    ("2", "unusable key: not a key, or the key does not open this backup"),
    ("3", "wrong backup type: crypt14, password-based or passkey-based backup"),
    ("4", "build incomplete: some media could not be processed, the archive is still usable"),
    ("5", "missing external dependency: ffmpeg, ffprobe, or tesseract for a screenshot key"),
]

EXAMPLES = [
    "wexport ~/Downloads/a",
    "wexport ~/Downloads/a --list",
    "wexport ~/Downloads/a --chat 2",
    "wexport ~/Downloads/a --chat family --chat 3 --yes",
]


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="wexport",
        usage="wexport [folder] [options]",
        description="Turn an Android WhatsApp crypt15 backup into self-contained archives that open by "
        "double-clicking index.html: no server, no wexport and no network on the reading machine. "
        "Point it at the folder you copied; it does the rest.",
        epilog=_reference(),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--version", action="version", version=f"wexport {__version__}")
    for flag in FLAGS:
        kwargs = {key: value for key, value in flag.items() if key != "names"}
        parser.add_argument(*flag["names"], **kwargs)
    return parser


def _flag_label(flag: dict) -> str:
    names = flag["names"]
    if not names[0].startswith("-"):
        return flag.get("metavar") or names[0].upper()
    head = " | ".join(names)
    if flag.get("action") in {"store_true", "store_false", "version", "help"}:
        return head
    metavar = flag.get("metavar") or head.lstrip("-").upper()
    return f"{head} {metavar}"


def _reference() -> str:
    """The extra sections `wexport -h` carries below the flags argparse already prints."""
    lines = ["exit codes:"]
    lines.extend(f"  {code}  {text}" for code, text in EXIT_CODES)
    lines.append("")
    lines.append("requirements: ffmpeg and ffprobe on PATH; tesseract only for a screenshot key.")
    lines.append("")
    lines.append("examples:")
    lines.extend(f"  {example}" for example in EXAMPLES)
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return _run(args)
    except WexError as error:
        print(f"wexport: {error}", file=sys.stderr)
        return error.exit_code


def _run(args: argparse.Namespace) -> int:
    folder = _one_folder(args.input)
    preflight.require_binaries("ffmpeg", "ffprobe")
    inputs = preflight.discover(folder)
    header = preflight.read_header(inputs.database)
    preflight.require_openable(header)

    key_spec = args.key or _key_beside_the_backup(inputs) or _ask_for_key()
    key = load_key(key_spec)
    time = FrozenTime(_system_zone())
    out_root = Path(args.out).expanduser().resolve() if args.out else inputs.root / _OUTPUT_DIR
    if inputs.ignored:
        print(f"ignored      {len(inputs.ignored)} file(s) from the older backup chain (crypt14)")

    with tempfile.TemporaryDirectory(prefix="wexport-") as workspace:
        database = Path(workspace) / "msgstore.db"
        print(f"decrypting   {inputs.database.name}")
        decrypt_to(inputs.database, database, key)
        if inputs.incrementals:
            print(f"merging      {len(inputs.incrementals)} incremental file(s)")
            merge(database, _decrypt_incrementals(inputs, workspace, key))

        with Msgstore(database) as store:
            if inputs.contacts is not None:
                contacts = _contacts_database(inputs.contacts, workspace, key)
                if contacts is not None:
                    store.apply_contacts(contacts)
            stats = store.chat_stats()
            if not stats:
                raise UnreadableInput(f"{inputs.database.name} holds no chat")
            # Most recent conversation first: that is the order a person recognises, and it makes
            # "the 10 most recent" a real default instead of the database's insertion order.
            stats.sort(key=lambda stat: stat.last_ts, reverse=True)
            if args.list or (not args.yes and not args.chat):
                _show_chats(stats, time)  # the list is the question; print it before asking
            else:
                print(f"\n{len(stats)} chat(s) in the backup; --list shows them")
            if args.list:
                return 0
            chosen = _choose(stats, args)
            summaries = _build(store, chosen, out_root, time, header, inputs, args)

    return _report(summaries, out_root, args)


# ---------------------------------------------------------------- the questions


def _ask_for_key() -> str:
    if not sys.stdin.isatty():
        raise UnusableKey(
            "no key found and no terminal to ask on.\n"
            "Pass --key with the 64 hex digits, a screenshot, or encrypted_backup.key."
        )
    print()
    print("key: the 64 hex digits, or the path of the screenshot / encrypted_backup.key")
    answer = input("> ").strip()
    if not answer:
        raise UnusableKey(
            "no key given. WhatsApp shows it once, when end-to-end encrypted backup is turned on."
        )
    return answer


def _choose(stats, args: argparse.Namespace) -> list:
    if args.yes:
        return stats
    if args.chat:
        return _match(stats, args.chat)
    if not sys.stdin.isatty():
        raise UnreadableInput(
            "no terminal to ask which chats to build.\n"
            "Pass --chat <number|name> or --yes (or --list to only look)."
        )
    recent = sorted(stats, key=lambda stat: stat.last_ts, reverse=True)[:DEFAULT_RECENT]
    default = ", ".join(str(_row(stats, stat)) for stat in recent)
    while True:
        answer = input(f"which? numbers or part of a name [the {len(recent)} most recent: {default}]\n> ")
        tokens = [token.strip() for token in answer.replace(",", " ").split() if token.strip()]
        if not tokens:
            return recent
        matches = _match(stats, tokens)
        if matches:
            return matches
        print("  nothing matched: try a number from the list, part of a name, or 'all'")


def _match(stats, tokens: list[str]) -> list:
    """Rows by number, or chats by name/JID substring; 'all' selects everything."""
    selected: dict[int, object] = {}
    for token in tokens:
        lowered = token.lower()
        if lowered in {"all", "tutte", "*"}:
            for stat in stats:
                selected[_row(stats, stat)] = stat
            continue
        if lowered.isdigit():
            index = int(lowered)
            if 1 <= index <= len(stats):
                selected[index] = stats[index - 1]
            continue
        needle = _fold(lowered)
        for stat in stats:
            if needle in _fold(stat.chat.name) or needle in _fold(stat.chat.jid):
                selected[_row(stats, stat)] = stat
    return [selected[key] for key in sorted(selected)]


def _row(stats, stat) -> int:
    return stats.index(stat) + 1


def _show_chats(stats, time: FrozenTime) -> None:
    width = max((len(stat.chat.name) for stat in stats), default=8)
    print()
    print(f"{'#':>3}  {'last event':<27} {'msgs':>7} {'media':>6} {'size':>9}  {'chat':<{width}}  jid")
    for index, stat in enumerate(stats, start=1):
        stamp = time.local(stat.last_ts) if stat.last_ts else "—"
        print(
            f"{index:>3}  {stamp:<27} {stat.messages:>7} {stat.media_count:>6} "
            f"{_human_bytes(stat.media_bytes):>9}  {stat.chat.name:<{width}}  {stat.chat.jid}"
        )
    messages = sum(stat.messages for stat in stats)
    print(f"\n{len(stats)} chat(s), {messages} message(s), {_human_bytes(sum(s.media_bytes for s in stats))} of media")


def _build(store, stats, out_root, time, header, inputs, args) -> list:
    out_root.mkdir(parents=True, exist_ok=True)
    jobs = max(1, min(8, (os.cpu_count() or 2) // 2))
    print(f"\nbuilding     {len(stats)} chat(s) into {out_root} ({jobs} workers)")
    summaries = []
    for position, stat in enumerate(stats, start=1):
        chat = stat.chat
        bar = _Bar(f"{position}. {chat.name}", stat.media_count)
        summary = build_chat(
            msgstore=store,
            chat=chat,
            out_root=out_root,
            time=time,
            app_version=header.app_version,
            copy_root=inputs.root,
            media_root=inputs.media,
            link=True,
            jobs=jobs,
            on_media=bar.step,
        )
        bar.close()
        summaries.append(summary)
        if summary.empty:
            print(f"  {summary.name}: skipped, no messages")
            continue
        print(
            f"  {summary.archive.name}: {summary.messages} messages, {summary.media} media "
            f"({summary.missing} missing), {summary.bytes / 1e6:.1f} MB"
        )
        for failure in summary.failures[:5]:
            print(f"    {failure}")
    return summaries


class _Bar:
    """A one-line progress bar for the part that takes time: media, per chat.

    Silent when stdout is not a terminal, so a log file or a pipe stays clean; throttled to ten
    updates a second, so drawing costs nothing next to a thumbnail or a transcode.
    """

    def __init__(self, label: str, total: int, stream=None):
        self.label = label[:24]
        self.total = max(0, total)
        self.stream = stream or sys.stdout
        self.enabled = bool(self.total) and self.stream.isatty()
        self.done = 0
        self._last = 0.0
        self._open = False

    def step(self, count: int = 1) -> None:
        self.done += count
        if not self.enabled or self.done >= self.total:
            return  # the final line is close()'s job, drawn once
        moment = time.monotonic()
        if moment - self._last < 0.1:
            return
        self._last = moment
        self.stream.write("\r" + self.render())
        self.stream.flush()
        self._open = True

    def render(self) -> str:
        width = 24
        ratio = 1.0 if not self.total else min(1.0, self.done / self.total)
        filled = int(width * ratio)
        return (
            f"  {self.label:<24} [{'#' * filled}{'-' * (width - filled)}] "
            f"{int(ratio * 100):3d}% {self.done}/{self.total} files"
        )

    def close(self) -> None:
        if not self.enabled:
            return
        if self.done < self.total:  # a chat with fewer media than the count we were given
            self.done = self.total
        self.stream.write("\r" + self.render() + "\n")
        self.stream.flush()
        self._open = False


def _report(summaries, out_root: Path, args: argparse.Namespace) -> int:
    built = [summary for summary in summaries if not summary.empty]
    failures = sum(summary.failure_count for summary in summaries)
    if built:
        page = write_index(out_root, summaries)
        print(f"\nindex        {page}")
        _open(page)
    skipped = len(summaries) - len(built)
    print(f"done: {len(built)} archive(s) in {out_root}" + (f", {skipped} empty chat(s) skipped" if skipped else ""))
    if failures:
        print(f"{failures} file(s) could not be processed; the archives are usable", file=sys.stderr)
        return IncompleteBuild.exit_code
    return 0


# ---------------------------------------------------------------- helpers


def _key_beside_the_backup(inputs) -> str | None:
    places = [inputs.database.parent, inputs.root, Path.cwd()]
    for name in ("key.png", "key.jpg", "key.jpeg", "key.txt", "encrypted_backup.key"):
        for place in places:
            candidate = place / name
            if candidate.is_file():
                return str(candidate)
    return None


def _decrypt_incrementals(inputs, workspace: str, key) -> list[Path]:
    plain: list[Path] = []
    for index, increment in enumerate(inputs.incrementals, start=1):
        target = Path(workspace) / f"increment-{index}.db"
        decrypt_to(increment, target, key)
        plain.append(target)
    return plain


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


def _open(page: Path) -> None:
    """Open the index on a desktop, stay quiet on a server."""
    if not sys.stdout.isatty() or not (os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")):
        return
    opener = "open" if sys.platform == "darwin" else ("start" if os.name == "nt" else "xdg-open")
    with contextlib.suppress(OSError):
        subprocess.Popen([opener, str(page)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _one_folder(tokens: list[str]) -> str:
    """The single folder argument, with a useful answer when someone still types check/build."""
    if not tokens:
        return "."
    _reject_old_command(tokens[0])
    if len(tokens) > 1:
        raise UnreadableInput(
            "one folder at a time, please: " + " ".join(tokens) + "\n"
            "Point it at the copy; it lists the chats and asks which to build."
        )
    return tokens[0]


def _reject_old_command(name: str) -> None:
    if name in _OLD_COMMANDS:
        raise UnreadableInput(
            f"'{name}' is not a command any more: point wexport at the folder instead.\n"
            f"  wexport <folder>              (asks which chats)\n"
            f"  wexport <folder> --list       (only lists)\n"
            f"  wexport <folder> --chat 2     (builds one)"
        )


def _system_zone() -> str:
    """Best effort, and never silent about it: the zone ends up in every frozen timestamp."""
    path = Path("/etc/timezone")
    if path.is_file():
        name = path.read_text(encoding="utf-8", errors="replace").strip()
        if name:
            return name
    try:
        parts = Path("/etc/localtime").resolve().parts
        return "/".join(parts[parts.index("zoneinfo") + 1 :]) or "UTC"
    except (OSError, ValueError):
        return os.environ.get("TZ") or "UTC"


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


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
