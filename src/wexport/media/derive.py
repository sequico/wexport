"""Derivatives the viewer can draw without decoding media: thumbnails, posters, waveforms.

The external tools are injected so unit tests can prove the pipeline without ffmpeg (see
`.opencode/skills/wexport-implement/references/fixtures.md`).
"""

from __future__ import annotations

import json
import os
import subprocess
import threading
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

PEAKS = 500
Prober = Callable[[Path], dict]
Runner = Callable[[list[str]], None]


@dataclass
class Derived:
    width: int | None = None
    height: int | None = None
    duration_ms: int | None = None
    thumb: str | None = None  # relative to the archive root
    peaks: list[int] | None = None


def ffprobe(path: Path) -> dict:
    result = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,duration:format=duration",
            "-of",
            "json",
            str(path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "ffprobe failed")
    return json.loads(result.stdout or "{}")


def ffmpeg(args: list[str]) -> None:
    result = subprocess.run(["ffmpeg", "-nostdin", "-v", "error", *args], capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.decode(errors="replace").strip() or "ffmpeg failed")


def derive(
    kind: str,
    source: Path,
    archive_root: Path,
    sha256: str,
    *,
    probe: Prober = ffprobe,
    run: Runner = ffmpeg,
) -> Derived:
    """Produce what this kind needs. Raises on failure; the caller decides what a failure means."""
    if kind == "image":
        return _image(source, archive_root, sha256)
    if kind == "sticker":
        return Derived()
    if kind in {"video", "gif"}:
        return _video(source, archive_root, sha256, probe=probe, run=run)
    if kind == "audio":
        return _audio(source, archive_root, sha256, run=run)
    return Derived()


def _image(source: Path, archive_root: Path, sha256: str) -> Derived:
    from PIL import Image, ImageOps

    with Image.open(source) as image:
        image = ImageOps.exif_transpose(image)
        width, height = image.size
        thumb = image.copy()
        thumb.thumbnail((320, 320))
        rel = Path("deriv") / f"{sha256}.webp"
        target = archive_root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        thumb.convert("RGB").save(target, "WEBP", quality=80, method=4)
    return Derived(width=width, height=height, thumb=rel.as_posix())


def _video(source: Path, archive_root: Path, sha256: str, *, probe: Prober, run: Runner) -> Derived:
    info = probe(source)
    stream = (info.get("streams") or [{}])[0]
    duration = info.get("format", {}).get("duration") or stream.get("duration")
    rel = Path("deriv") / f"{sha256}.jpg"
    target = archive_root / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    run(
        [
            "-i", str(source), "-frames:v", "1", "-vf", "scale=480:-2",
            "-pix_fmt", "yuvj420p", "-q:v", "3", "-y", str(target),
        ]
    )
    return Derived(
        width=stream.get("width"),
        height=stream.get("height"),
        duration_ms=int(float(duration) * 1000) if duration else None,
        thumb=rel.as_posix(),
    )


def _audio(source: Path, archive_root: Path, sha256: str, *, run: Runner) -> Derived:
    rel = Path("deriv") / f"{sha256}.json"
    target = archive_root / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    raw = target.with_name(f"{sha256}.{os.getpid()}.{threading.get_ident()}.pcm")
    run(["-i", str(source), "-ac", "1", "-ar", "4000", "-f", "s16le", "-y", str(raw)])
    try:
        peaks = _peaks(raw.read_bytes())
    finally:
        raw.unlink(missing_ok=True)
    target.write_text(json.dumps(peaks, separators=(",", ":")), encoding="utf-8")
    return Derived(peaks=peaks)


def _peaks(pcm: bytes) -> list[int]:
    """`PEAKS` amplitudes in 0..100, so the browser draws a waveform without decoding audio."""
    samples = memoryview(pcm).cast("h") if pcm else memoryview(b"")
    if len(samples) < PEAKS:
        return [0] * PEAKS
    step = len(samples) // PEAKS
    out: list[int] = []
    for index in range(PEAKS):
        window = samples[index * step : (index + 1) * step]
        peak = max((abs(value) for value in window), default=0)
        out.append(min(100, round(peak * 100 / 32768)))
    return out
