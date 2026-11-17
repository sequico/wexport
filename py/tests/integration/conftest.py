"""Integration fixtures: a fixed timezone, so the frozen timestamps are predictable in tests."""

from __future__ import annotations

import pytest


@pytest.fixture(scope="session", autouse=True)
def fixed_zone():
    """The rendered strings depend on the machine zone; pin it instead of hoping."""
    patch = pytest.MonkeyPatch()
    patch.setattr("wexport.cli._system_zone", lambda: "Europe/Rome")
    yield
    patch.undo()
