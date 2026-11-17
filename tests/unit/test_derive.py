"""Derivatives with the external tools injected: no ffmpeg needed to judge the pipeline."""

from __future__ import annotations

from pathlib import Path

from wexport.media import derive


def test_peaks_are_bounded_and_fixed_length():
    pcm = b"".join(int(1000 * (i % 2)).to_bytes(2, "little", signed=True) for i in range(4000))
    peaks = derive._peaks(pcm)
    assert len(peaks) == derive.PEAKS
    assert all(0 <= value <= 100 for value in peaks)


def test_empty_audio_is_quiet_not_broken():
    assert derive._peaks(b"") == [0] * derive.PEAKS


def test_video_uses_the_injected_tools(tmp_path):
    calls: list[list[str]] = []

    def probe(path: Path) -> dict:
        return {"streams": [{"width": 640, "height": 360}], "format": {"duration": "2.5"}}

    def run(args: list[str]) -> None:
        calls.append(args)

    derived = derive.derive("video", tmp_path / "v.mp4", tmp_path, "abc", probe=probe, run=run)
    assert derived.thumb == "deriv/abc.jpg"
    assert derived.width == 640 and derived.duration_ms == 2500
    assert calls and calls[0][0] == "-i"


def test_audio_derivative_writes_peaks_and_cleans_the_pcm(tmp_path):
    def run(args: list[str]) -> None:
        Path(args[-1]).write_bytes(b"\x00\x10" * 2000)

    derived = derive.derive("audio", tmp_path / "a.opus", tmp_path, "def", run=run)
    assert derived.peaks and len(derived.peaks) == derive.PEAKS
    assert (tmp_path / "deriv" / "def.json").is_file()
    assert not (tmp_path / "deriv" / "def.pcm").exists()


def test_images_get_a_thumbnail(tmp_path):
    from PIL import Image

    source = tmp_path / "p.png"
    Image.new("RGB", (800, 400), (10, 20, 30)).save(source)
    derived = derive.derive("image", source, tmp_path, "aaa")
    assert derived.thumb == "deriv/aaa.webp"
    assert (derived.width, derived.height) == (800, 400)
    assert (tmp_path / "deriv" / "aaa.webp").is_file()
