"""Per-chat builder: messages in, one self-contained folder out.

Media are placed and derived here, through resolve.py and derive.py; the archive never links to
anything outside itself (plan §5, §6).
"""

from __future__ import annotations

import contextlib
import hashlib
import html
import json
import os
import re
import shutil
import sqlite3
from collections.abc import Callable, Iterable, Iterator
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path

from .. import __version__
from ..db.msgstore import Chat, Message, Msgstore
from ..db.slim import ChatDB, FrozenTime
from ..media import derive as derive_mod
from ..media import resolve as resolve_mod
from . import chunks

ASSETS = Path(__file__).parent / "assets"
BATCH = 256
FAILURE_SAMPLES = 20


@dataclass
class Summary:
    archive: Path
    name: str
    messages: int = 0
    media: int = 0
    missing: int = 0
    failures: list[str] = field(default_factory=list)  # a sample, not everything
    failure_count: int = 0
    bytes: int = 0
    first_ts: int = 0
    last_ts: int = 0
    people: set[str] = field(default_factory=set)
    empty: bool = False


def _record_failure(summary: Summary, text: str) -> None:
    """Count every failure, keep a bounded sample: a broken folder must not fill memory."""
    summary.failure_count += 1
    if len(summary.failures) < FAILURE_SAMPLES:
        summary.failures.append(text)


def build_chat(
    *,
    msgstore: Msgstore,
    chat: Chat,
    out_root: Path,
    time: FrozenTime,
    app_version: str,
    copy_root: Path,
    media_root: Path | None,
    link: bool = False,
    transcode: str = "none",
    jobs: int = 1,
    built: str | None = None,
    probe: derive_mod.Prober = derive_mod.ffprobe,
    run: derive_mod.Runner = derive_mod.ffmpeg,
) -> Summary:
    """Write one chat archive under `out_root` and return what it contains."""
    archive = out_root / _archive_name(chat, msgstore, time)
    archive.mkdir(parents=True, exist_ok=True)
    intermediate = out_root / ".wexport"
    intermediate.mkdir(exist_ok=True)
    chat_db = intermediate / f"{_slug(chat)}.{os.getpid()}.db"
    chat_db.unlink(missing_ok=True)

    summary = Summary(archive=archive, name=chat.name)
    with ChatDB(chat_db, time, app_version) as db:
        db.add_chat(chat.jid, chat.name, chat.is_group)
        messages = msgstore.messages(chat)
        prepare = _preparer(archive, copy_root, media_root, link, transcode, probe, run, summary)
        with ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
            for batch in _batched(messages, BATCH):
                prepared = list(pool.map(prepare, batch))
                for message, media_row in zip(batch, prepared, strict=False):
                    db.add_message(message)
                    for emoji in message.reactions:
                        # shortcut: the add-on author is not read, so the sender stays NULL; the
                        # viewer shows the emoji only. Read message_add_on.sender_jid when needed.
                        db.add_reaction(message.id, None, emoji, message.ts)
                    if media_row is not None:
                        db.add_media(message.id, media_row)
                        summary.media += 1
                        if media_row.get("missing"):
                            summary.missing += 1
                    if summary.messages == 0:
                        summary.first_ts = message.ts
                    summary.last_ts = message.ts
                    if message.sender:
                        summary.people.add(message.sender)
                    summary.messages += 1
        db.set_chat_range(summary.first_ts, summary.last_ts, summary.messages)
        db.commit()

        if summary.messages == 0:
            # A chat row with no messages is not worth a folder; say so instead of shipping an
            # archive that opens on "Empty archive".
            summary.empty = True
        else:
            manifest = _manifest(chat, time, app_version, summary, built)
            shutil.rmtree(archive / "data", ignore_errors=True)  # no stale chunk from a previous run
            chunks.emit(chat_db, archive / "data", manifest)
            _copy_assets(archive)
            _write_readme(archive, chat, time, app_version, summary, built)
            _prune(archive, chat_db)
            summary.bytes = _tree_bytes(archive)

    chat_db.unlink(missing_ok=True)
    with contextlib.suppress(OSError):  # leave no empty working directory in the user's output folder
        intermediate.rmdir()
    if summary.empty:
        with contextlib.suppress(OSError):
            archive.rmdir()  # it only ever held the folder we just created
    return summary


def write_index(out_root: Path, summaries: Iterable[Summary]) -> Path:
    """The only page that lists chats together; it never lives inside a chat folder."""
    items = "\n".join(
        f'<li><a href="{html.escape(summary.archive.name)}/index.html">'
        f"{html.escape(summary.name)}</a> <span>{summary.messages} messages</span></li>"
        for summary in summaries
        if not summary.empty
    )
    page = f"""<!doctype html>
<html lang="en"><meta charset="utf-8"><title>wexport archives</title>
<style>body{{font:16px/1.5 system-ui,sans-serif;max-width:44rem;margin:4rem auto;padding:0 1rem}}
li{{margin:.4rem 0}}span{{color:#666;font-size:.9em}}</style>
<h1>Archives</h1>
<p>Each folder below is a complete archive: open its <code>index.html</code>. Nothing here needs a
server or wexport installed.</p>
<ul>
{items}
</ul>
<p>Built with wexport {html.escape(__version__)}.</p>
</html>
"""
    target = out_root / "index.html"
    target.write_text(page, encoding="utf-8")
    return target


# ---------------------------------------------------------------- helpers


def _preparer(
    archive: Path,
    copy_root: Path,
    media_root: Path | None,
    link: bool,
    transcode: str,
    probe: derive_mod.Prober,
    run: derive_mod.Runner,
    summary: Summary,
) -> Callable[[Message], dict | None]:
    def prepare(message: Message) -> dict | None:
        media = message.media
        if media is None:
            return None
        row: dict = {"kind": media.kind, "mime": media.mime, "name": Path(media.rel_path).name}
        try:
            source = resolve_mod.source_for(media.rel_path, media_root, copy_root)
            if source is None:
                row["missing"] = 1
                return row
            placed = resolve_mod.place(source, archive, link=link)
            row.update(
                sha256=placed.sha256, path=placed.rel_path, bytes=media.bytes or source.stat().st_size
            )
            derived = derive_mod.derive(
                media.kind, source, archive, placed.sha256, probe=probe, run=run
            )
        except (OSError, ValueError, RuntimeError) as exc:
            _record_failure(summary, f"{message.id}: {media.rel_path}: {_first_line(exc)}")
            return row
        row.update(
            width=media.width or derived.width,
            height=media.height or derived.height,
            duration_ms=media.duration_ms or derived.duration_ms,
            thumb=derived.thumb,
        )
        if derived.peaks is not None:
            row["peaks"] = json.dumps(derived.peaks, separators=(",", ":"))
        if _wants_transcode(transcode, media.kind):
            row["alt"] = _transcode(media.kind, source, archive, placed.sha256, run, summary, message)
        return row

    return prepare


def _wants_transcode(choice: str, kind: str) -> bool:
    if choice == "all":
        return kind in {"audio", "video", "gif"}
    if choice == "audio":
        return kind == "audio"
    if choice == "video":
        return kind in {"video", "gif"}
    return False


def _transcode(
    kind: str,
    source: Path,
    archive: Path,
    sha256: str,
    run: derive_mod.Runner,
    summary: Summary,
    message: Message,
) -> str | None:
    try:
        if kind == "audio":
            rel = Path("deriv") / f"{sha256}.m4a"
            run(["-i", str(source), "-vn", "-c:a", "aac", "-b:a", "96k", "-y", str(archive / rel)])
        else:
            rel = Path("deriv") / f"{sha256}.mp4"
            run(
                [
                    "-i", str(source), "-vf", "scale=-2:720", "-c:v", "libx264", "-crf", "28",
                    "-preset", "veryfast", "-c:a", "aac", "-movflags", "+faststart", "-y",
                    str(archive / rel),
                ]
            )
        return rel.as_posix()
    except Exception as exc:
        _record_failure(summary, f"{message.id}: transcode {kind}: {_first_line(exc)}")
        return None


def _first_line(exc: BaseException) -> str:
    text = str(exc).strip().splitlines()
    return text[0][:300] if text else exc.__class__.__name__


def _batched(items: Iterator[Message], size: int) -> Iterator[list[Message]]:
    batch: list[Message] = []
    for item in items:
        batch.append(item)
        if len(batch) >= size:
            yield batch
            batch = []
    if batch:
        yield batch


def _archive_name(chat: Chat, msgstore: Msgstore, time: FrozenTime) -> str:
    """`<date>-<slug>`, dated by the newest message so folders sort by recency."""
    newest = msgstore.last_ts(chat)
    stamp = time.local(newest)[:10] if newest else "0000-00-00"
    return f"{stamp}-{_slug(chat)}"


def _slug(chat: Chat) -> str:
    base = re.sub(r"[^a-z0-9]+", "-", chat.name.lower().replace("+", " ")).strip("-") or "chat"
    return f"{base[:40]}-{hashlib.sha1(chat.jid.encode()).hexdigest()[:4]}"


def _manifest(
    chat: Chat, time: FrozenTime, app_version: str, summary: Summary, built: str | None
) -> dict:
    return {
        "title": chat.name,
        "is_group": chat.is_group,
        "tz": time.tz_name,
        "tzdata": _tzdata(),
        "source_app_version": app_version,
        "builder": f"wexport {__version__}",
        "built": built or datetime.now(UTC).isoformat(timespec="seconds"),
        "media": summary.media,
        "missing": summary.missing,
        "people": len(summary.people) + (1 if chat.is_group else 0),  # group senders plus me
        "first": time.local(summary.first_ts) if summary.first_ts else "",
        "last": time.local(summary.last_ts) if summary.last_ts else "",
    }


def _tzdata() -> str:
    from ..db.slim import tzdata_version

    return tzdata_version()


def _prune(archive: Path, chat_db: Path) -> None:
    """Delete media and derivatives that this build no longer references.

    A rebuild after messages were removed from the phone must not leave the old files behind: the
    archive is a copy of a conversation, and a deleted photo that survives in it is a privacy defect,
    not a spare byte.
    """
    con = sqlite3.connect(f"file:{chat_db}?mode=ro", uri=True)
    try:
        keep = {
            row[0]
            for row in con.execute(
                "SELECT path FROM media WHERE path IS NOT NULL "
                "UNION SELECT thumb FROM media WHERE thumb IS NOT NULL "
                "UNION SELECT alt FROM media WHERE alt IS NOT NULL"
            )
        }
    finally:
        con.close()
    for folder in ("media", "deriv"):
        root = archive / folder
        if not root.is_dir():
            continue
        for path in root.rglob("*"):
            if path.is_file() and path.relative_to(archive).as_posix() not in keep:
                path.unlink()
        for directory in sorted((p for p in root.rglob("*") if p.is_dir()), reverse=True):
            with contextlib.suppress(OSError):
                directory.rmdir()
        with contextlib.suppress(OSError):
            root.rmdir()


def _copy_assets(archive: Path) -> None:
    for name in ("index.html", "app.js", "app.css"):
        shutil.copy2(ASSETS / name, archive / name)


def _write_readme(
    archive: Path, chat: Chat, time: FrozenTime, app_version: str, summary: Summary, built: str | None
) -> None:
    text = f"""wexport archive — {chat.name}

Open index.html in any browser. No server is needed, nothing needs to be installed, and
nothing is loaded from the network.

Built ......... {built or datetime.now(UTC).isoformat(timespec="seconds")} by wexport {__version__}
Source app .... WhatsApp {app_version}
Time zone ..... {time.tz_name} (timestamps are fixed strings with their UTC offset)
Messages ...... {summary.messages}
Media files ... {summary.media} ({summary.missing} referenced but not present in the copied folder)

This folder is complete on its own: messages, data chunks, images, video and voice notes.
Keep it as a folder; if you share it, share the whole folder (a zip of it works).
"""
    (archive / "README.txt").write_text(text, encoding="utf-8")


def _tree_bytes(root: Path) -> int:
    return sum(path.stat().st_size for path in root.rglob("*") if path.is_file())
