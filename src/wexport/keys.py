"""Turn the three accepted key forms into a crypt15 key.

Accepted: 64 hex digits (spaces, newlines and a 4x4 grid are tolerated), an
`encrypted_backup.key` file, or a screenshot of the key. Only the first two work without
tesseract; the third fails with the install command rather than guessing.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from .errors import MissingDependency, UnusableKey

_HEX = re.compile(r"^[0-9a-fA-F]{64}$")
_IMAGE_SUFFIXES = {".png", ".jpg", ".jpeg", ".webp", ".bmp", ".tif", ".tiff"}
_KEY_SUFFIXES = {".key", ".bin"}


def normalize_hex(spec: str) -> str | None:
    """64 hex digits in any spacing, or None when the text is not a hex key."""
    digits = "".join(ch for ch in spec if not ch.isspace())
    digits = digits.replace("-", "").replace(":", "")
    return digits.lower() if _HEX.match(digits) else None


def load_key(spec: str) -> Any:
    """A `Key15` for the CLI's `--key`, or `UnusableKey` naming what was tried."""
    from wa_crypt_tools import KeyFactory

    hexkey = normalize_hex(spec)
    if hexkey:
        return KeyFactory.from_hex(hexkey)

    path = Path(spec).expanduser()
    if not path.is_file():
        raise UnusableKey(
            f"key not found: {spec}\n"
            "Give one of: the 64 hex digits, a screenshot of them, or encrypted_backup.key."
        )

    if path.suffix.lower() in _IMAGE_SUFFIXES:
        _require_tesseract()
        try:
            return KeyFactory.from_image(path)
        except ImportError as exc:
            raise MissingDependency(
                "reading the key from an image needs the OCR extra:\n"
                "  uv sync --extra ocr      (or: pip install pytesseract)\n"
                f"({exc})"
            ) from exc
        except Exception as exc:
            raise UnusableKey(
                f"could not read the key from {path.name}: {exc}\n"
                "Crop the screenshot to the key itself, or type the 64 digits."
            ) from exc

    try:
        return KeyFactory.new(path)
    except Exception as exc:
        # A text file holding the 64 digits is a common way people store the key.
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            text = ""
        hexkey = normalize_hex(text)
        if hexkey:
            return KeyFactory.from_hex(hexkey)
        raise UnusableKey(
            f"{path.name} is not a usable key file: {exc}\n"
            "It must be the phone's encrypted_backup.key, or the 64 hex digits in a text file."
        ) from exc


def _require_tesseract() -> None:
    import shutil

    if shutil.which("tesseract") is None:
        raise MissingDependency(
            "tesseract not found on PATH, needed to read the key from a screenshot.\n"
            "  Debian/Ubuntu   sudo apt install tesseract-ocr\n"
            "  Fedora/RHEL     sudo dnf install tesseract\n"
            "  Arch            sudo pacman -S tesseract\n"
            "  Windows         winget install UB-Mannheim.TesseractOCR\n"
            "Or type the 64 hex digits instead of passing an image."
        )
