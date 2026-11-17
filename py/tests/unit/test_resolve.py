"""Media placement: the database path is a trust boundary, and the archive is content-addressed."""

from __future__ import annotations

from wexport.media import resolve


def test_source_found_under_the_copied_folder(tmp_path):
    media = tmp_path / "Media" / "WhatsApp Images"
    media.mkdir(parents=True)
    photo = media / "IMG-1.png"
    photo.write_bytes(b"x")
    assert resolve.source_for("Media/WhatsApp Images/IMG-1.png", tmp_path / "Media", tmp_path) == photo


def test_a_path_escaping_the_copy_is_refused(tmp_path):
    """A row pointing outside the copied folder must not pull a local file into the archive."""
    outside = tmp_path.parent / "secret.txt"
    outside.write_bytes(b"not yours")
    copy_root = tmp_path / "phone"
    copy_root.mkdir()
    assert resolve.source_for("../secret.txt", None, copy_root) is None
    assert resolve.source_for(str(outside), None, copy_root) is None


def test_place_is_content_addressed_and_idempotent(tmp_path):
    source = tmp_path / "a.bin"
    source.write_bytes(b"hello")
    archive = tmp_path / "archive"
    first = resolve.place(source, archive)
    second = resolve.place(source, archive)
    assert first == second
    assert first.rel_path.startswith("media/")
    assert (archive / first.rel_path).read_bytes() == b"hello"


def test_place_leaves_no_partial_files(tmp_path):
    source = tmp_path / "b.bin"
    source.write_bytes(b"payload")
    archive = tmp_path / "archive"
    placed = resolve.place(source, archive)
    leftovers = list(archive.rglob("*.part"))
    assert not leftovers
    assert (archive / placed.rel_path).is_file()


def test_hardlink_falls_back_to_copy(tmp_path, monkeypatch):
    source = tmp_path / "c.bin"
    source.write_bytes(b"data")

    def refuse(*args, **kwargs):
        raise OSError("cross-device link")

    monkeypatch.setattr(resolve.os, "link", refuse)
    placed = resolve.place(source, tmp_path / "archive", link=True)
    assert (tmp_path / "archive" / placed.rel_path).read_bytes() == b"data"
