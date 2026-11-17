"""The progress bar: visible on a terminal, silent in a pipe, wired to the media work."""

from __future__ import annotations

from wexport.cli import _Bar, main

KEY = "0123456789abcdef" * 4


class _FakeTerminal:
    def __init__(self, tty: bool = True):
        self.parts: list[str] = []
        self._tty = tty

    def isatty(self) -> bool:
        return self._tty

    def write(self, text: str) -> None:
        self.parts.append(text)

    def flush(self) -> None:
        pass


def test_the_bar_shows_percentage_and_counts():
    bar = _Bar("1. Family", 4, stream=_FakeTerminal())
    bar.step(2)
    line = bar.render()
    assert "50%" in line
    assert "2/4 files" in line
    assert "1. Family" in line


def test_the_bar_is_silent_without_a_terminal():
    stream = _FakeTerminal(tty=False)
    bar = _Bar("Family", 4, stream=stream)
    bar.step(4)
    bar.close()
    assert stream.parts == []


def test_closing_prints_a_newline_and_reaches_100_percent():
    stream = _FakeTerminal()
    bar = _Bar("Family", 3, stream=stream)
    bar.step(2)
    bar.close()
    assert stream.parts[-1].endswith("\n")
    assert "100%" in stream.parts[-1]


def test_a_chat_with_no_media_draws_nothing():
    stream = _FakeTerminal()
    bar = _Bar("Family", 0, stream=stream)
    bar.step(1)
    bar.close()
    assert stream.parts == []


def test_the_build_wires_the_bar_to_the_media_count(phone, tmp_path, monkeypatch):
    seen: list[tuple[str, int]] = []

    class _Spy:
        def __init__(self, label, total):
            seen.append((label, total))

        def step(self, count=1):
            pass

        def close(self):
            pass

    monkeypatch.setattr("wexport.cli._Bar", _Spy)
    assert main([str(phone), "--key", KEY, "--yes", "--out", str(tmp_path / "out")]) == 0
    # every chat gets a bar sized on its own media count; the group in the fixture has six files
    assert ("2. Family", 6) in seen
    assert all(total >= 0 for _, total in seen)
