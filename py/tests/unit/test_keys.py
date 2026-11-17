"""Key input: hex in any spacing, a key file, an image, or a clear refusal."""

from __future__ import annotations

import pytest

from wexport.errors import UnusableKey
from wexport.keys import load_key, normalize_hex


def test_hex_accepts_grid_spacing_and_case():
    spaced = "0123 4567 89AB CDEF\n0123 4567 89AB CDEF\n0123 4567 89AB CDEF\n0123 4567 89AB CDEF"
    assert normalize_hex(spaced) == "0123456789abcdef" * 4


def test_hex_rejects_short_or_non_hex():
    assert normalize_hex("deadbeef") is None
    assert normalize_hex("z" * 64) is None


def test_load_key_from_hex(fixture_key):
    assert load_key(fixture_key) is not None


def test_missing_key_file_names_the_options(tmp_path):
    with pytest.raises(UnusableKey) as error:
        load_key(str(tmp_path / "nope.key"))
    assert "64 hex digits" in str(error.value)


def test_non_key_file_is_refused(tmp_path):
    bogus = tmp_path / "x.key"
    bogus.write_bytes(b"not a java object")
    with pytest.raises(UnusableKey):
        load_key(str(bogus))


def test_a_text_file_with_the_digits_is_a_key(tmp_path):
    from wexport.keys import load_key

    stored = tmp_path / "key.txt"
    stored.write_text("0123 4567 89ab cdef\n" * 4, encoding="utf-8")
    assert load_key(str(stored)) is not None


def test_screenshot_without_tesseract_says_what_to_install(tmp_path, monkeypatch):
    monkeypatch.setattr("shutil.which", lambda name: None)
    image = tmp_path / "key.png"
    image.write_bytes(b"\x89PNG\r\n\x1a\n")
    with pytest.raises(Exception) as error:
        load_key(str(image))
    message = str(error.value)
    assert "tesseract" in message and "apt install" in message
