"""End to end on the synthetic phone folder: the archive is what the plan promises."""

from __future__ import annotations

import json
import re
from pathlib import Path

import pytest

from wexport.cli import main


@pytest.fixture(scope="module")
def built(tmp_path_factory, phone):
    out = tmp_path_factory.mktemp("archives")
    code = main(
        [
            "build",
            str(phone),
            "--key",
            "0123456789abcdef" * 4,
            "--out",
            str(out),
            "--index",
            "--tz",
            "Europe/Rome",
        ]
    )
    assert code == 0
    return out


def _manifest(archive: Path) -> dict:
    text = (archive / "data" / "manifest.js").read_text(encoding="utf-8").strip()
    return json.loads(text[len("WEX.manifest(") : -2])


def _archives(root: Path) -> list[Path]:
    return sorted(p for p in root.iterdir() if p.is_dir() and not p.name.startswith("."))


def test_two_archives_and_an_index(built):
    archives = _archives(built)
    assert len(archives) == 2
    assert (built / "index.html").is_file()


def test_each_archive_is_self_contained(built):
    for archive in _archives(built):
        for name in ("index.html", "app.js", "app.css", "README.txt", "data/manifest.js"):
            assert (archive / name).is_file(), name
        assert "No server is needed" in (archive / "README.txt").read_text(encoding="utf-8")


def test_a_chat_archive_never_links_another_chat(built):
    for archive in _archives(built):
        page = (archive / "index.html").read_text(encoding="utf-8")
        assert "../" not in page
        assert not re.search(r'index\.html', page)


def test_manifest_counts_and_zone(built):
    family = next(a for a in _archives(built) if "family" in a.name)
    manifest = _manifest(family)
    assert manifest["messages"] == 9
    assert manifest["tz"] == "Europe/Rome"
    assert manifest["source_app_version"]
    assert manifest["people"] == 2  # one other sender plus me
    assert manifest["first"].startswith("2026-12-01") and manifest["last"].startswith("2026-12-02")


def test_timestamps_are_frozen_strings_with_offsets(built):
    family = next(a for a in _archives(built) if "family" in a.name)
    chunk = (family / "data" / "2026-12.js").read_text(encoding="utf-8")
    stamps = re.findall(r'"s":"([^"]+)"', chunk)
    assert stamps
    assert all(re.fullmatch(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} [+-]\d{2}:\d{2}", s) for s in stamps)


def test_media_is_content_addressed_and_present(built):
    family = next(a for a in _archives(built) if "family" in a.name)
    chunk = (family / "data" / "2026-12.js").read_text(encoding="utf-8")
    assert '"th":"deriv/' in chunk  # a thumbnail exists for the photo
    assert re.search(r'"p":"media/[0-9a-f]{2}/[0-9a-f]{64}\.png"', chunk)


def test_missing_media_becomes_a_placeholder(built):
    alice = next(a for a in _archives(built) if "alice" in a.name)
    chunk = "".join(p.read_text(encoding="utf-8") for p in sorted((alice / "data").glob("*.js")))
    assert '"mi":1' in chunk


def test_reactions_reach_the_archive(built):
    family = next(a for a in _archives(built) if "family" in a.name)
    chunk = (family / "data" / "2026-12.js").read_text(encoding="utf-8")
    assert '"rx":["👍"]' in chunk


def test_viewer_uses_no_fetch_modules_or_workers(built):
    for archive in _archives(built):
        script = (archive / "app.js").read_text(encoding="utf-8")
        assert "fetch(" not in script
        assert "type=\"module\"" not in (archive / "index.html").read_text(encoding="utf-8")
        assert "new Worker(" not in script


def test_deterministic_second_build(tmp_path, phone):
    out = tmp_path / "again"
    code = main(
        ["build", str(phone), "--key", "0123456789abcdef" * 4, "--out", str(out), "--tz", "Europe/Rome"]
    )
    assert code == 0
    family = next(p for p in out.iterdir() if p.is_dir() and "family" in p.name)
    before = (family / "data" / "2026-12.js").read_bytes()
    manifest_before = json.loads(
        (family / "data" / "manifest.js").read_text().strip()[len("WEX.manifest(") : -2]
    )

    code = main(
        ["build", str(phone), "--key", "0123456789abcdef" * 4, "--out", str(out), "--tz", "Europe/Rome"]
    )
    assert code == 0
    assert (family / "data" / "2026-12.js").read_bytes() == before
    manifest_after = json.loads(
        (family / "data" / "manifest.js").read_text().strip()[len("WEX.manifest(") : -2]
    )
    assert manifest_after["chunks"] == manifest_before["chunks"]
