"""End to end on the synthetic phone folder: the archive is what the plan promises."""

from __future__ import annotations

import json
import re
import shutil
from pathlib import Path

import pytest

from wexport.cli import main

KEY = "0123456789abcdef" * 4


@pytest.fixture(scope="module")
def built(tmp_path_factory, phone):
    out = tmp_path_factory.mktemp("archives")
    assert main([str(phone), "--key", KEY, "--yes", "--out", str(out)]) == 0
    return out


def _manifest(archive: Path) -> dict:
    text = (archive / "data" / "manifest.js").read_text(encoding="utf-8").strip()
    return json.loads(text[len("WEX.manifest(") : -2])


def _archives(root: Path) -> list[Path]:
    return sorted(p for p in root.iterdir() if p.is_dir() and not p.name.startswith("."))


def test_two_archives_and_an_index(built):
    assert len(_archives(built)) == 2
    assert (built / "index.html").is_file()


def test_the_index_credits_the_builder_not_whatsapp(built):
    page = (built / "index.html").read_text(encoding="utf-8")
    assert "Built with wexport 0.1.0" in page
    assert "2.26.34.7" not in page


def test_each_archive_is_self_contained(built):
    for archive in _archives(built):
        for name in ("index.html", "app.js", "app.css", "README.txt", "data/manifest.js"):
            assert (archive / name).is_file(), name
        assert "No server is needed" in (archive / "README.txt").read_text(encoding="utf-8")


def test_a_chat_archive_never_links_another_chat(built):
    for archive in _archives(built):
        page = (archive / "index.html").read_text(encoding="utf-8")
        assert "../" not in page
        assert not re.search(r"index\.html", page)


def test_manifest_counts_zone_and_range(built):
    family = next(a for a in _archives(built) if "family" in a.name)
    manifest = _manifest(family)
    assert manifest["messages"] == 9
    assert manifest["tz"] == "Europe/Rome"
    assert manifest["people"] == 2
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
    assert '"th":"deriv/' in chunk
    assert re.search(r'"p":"media/[0-9a-f]{2}/[0-9a-f]{64}\.png"', chunk)


def test_missing_media_becomes_a_placeholder(built):
    alice = next(a for a in _archives(built) if "alice" in a.name)
    chunk = "".join(p.read_text(encoding="utf-8") for p in sorted((alice / "data").glob("*.js")))
    assert '"mi":1' in chunk


def test_reactions_reach_the_archive(built):
    family = next(a for a in _archives(built) if "family" in a.name)
    assert '"rx":["👍"]' in (family / "data" / "2026-12.js").read_text(encoding="utf-8")


def test_viewer_uses_no_fetch_modules_or_workers(built):
    for archive in _archives(built):
        assert "fetch(" not in (archive / "app.js").read_text(encoding="utf-8")
        assert 'type="module"' not in (archive / "index.html").read_text(encoding="utf-8")
        assert "new Worker(" not in (archive / "app.js").read_text(encoding="utf-8")


def test_a_rebuild_is_deterministic(phone, tmp_path):
    out = tmp_path / "again"
    assert main([str(phone), "--key", KEY, "--yes", "--out", str(out)]) == 0
    family = next(p for p in out.iterdir() if p.is_dir() and "family" in p.name)
    before = (family / "data" / "2026-12.js").read_bytes()
    assert main([str(phone), "--key", KEY, "--yes", "--out", str(out)]) == 0
    assert (family / "data" / "2026-12.js").read_bytes() == before


def test_a_rebuild_removes_media_it_no_longer_references(phone, tmp_path):
    out = tmp_path / "out"
    args = [str(phone), "--key", KEY, "--yes", "--out", str(out)]
    assert main(args) == 0
    family = next(p for p in out.iterdir() if "family" in p.name)
    orphan = family / "media" / "zz" / "deadbeef.png"
    orphan.parent.mkdir(parents=True)
    orphan.write_bytes(b"a photo that no longer exists anywhere")
    assert main(args) == 0
    assert not orphan.exists()


def test_an_empty_chat_is_skipped_not_shipped(phone, tmp_path, capsys):
    out = tmp_path / "out"
    assert main([str(phone), "--key", KEY, "--yes", "--out", str(out)]) == 0
    assert not any("empty" in archive.name for archive in _archives(out))
    output = capsys.readouterr().out
    assert "skipped, no messages" in output
    assert "empty chat(s) skipped" in output


def test_a_folder_without_media_still_builds(phone, tmp_path):
    copy = tmp_path / "phone"
    copy.mkdir()
    for name in ("Databases", "Backups"):
        shutil.copytree(phone / name, copy / name)
    out = tmp_path / "out"
    assert main([str(copy), "--key", KEY, "--yes", "--out", str(out)]) == 0
    family = next(p for p in out.iterdir() if "family" in p.name)
    chunk = (family / "data" / "2026-12.js").read_text(encoding="utf-8")
    assert '"mi":1' in chunk
