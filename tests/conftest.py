"""Shared fixtures: the synthetic phone folder, and the paths tests build into."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent
SYNTHETIC = ROOT / "fixtures" / "synthetic"
PHONE = SYNTHETIC / "phone"
KEY_HEX = "0123456789abcdef" * 4


def _load_builder():
    spec = importlib.util.spec_from_file_location("fixture_build", SYNTHETIC / "build.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="session")
def phone() -> Path:
    """The copied-phone fixture, regenerated only when it is absent."""
    if not (PHONE / "Databases" / "msgstore.db.crypt15").is_file():
        _load_builder().build(PHONE)
    return PHONE


@pytest.fixture(scope="session")
def fixture_key() -> str:
    return KEY_HEX
