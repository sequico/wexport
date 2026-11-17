"""`wexport -h` must document every flag of every command, from one source."""

from __future__ import annotations

import argparse

from wexport.cli import build_parser


def _subparsers(parser: argparse.ArgumentParser) -> dict[str, argparse.ArgumentParser]:
    action = next(a for a in parser._actions if isinstance(a, argparse._SubParsersAction))
    return action.choices  # type: ignore[attr-defined]


def test_top_level_help_mentions_every_option_and_positional():
    parser = build_parser()
    help_text = parser.format_help()
    for command, subparser in _subparsers(parser).items():
        for action in subparser._actions:
            names = action.option_strings or [action.dest.upper()]
            for name in names:
                assert name in help_text, f"{command}: {name} is missing from `wexport -h`"


def test_help_carries_exit_codes_and_requirements():
    text = build_parser().format_help()
    for code in ("1", "2", "3", "4", "5"):
        assert f"  {code}  " in text
    assert "ffmpeg" in text and "tesseract" in text
    assert "examples:" in text


def test_each_command_still_has_its_own_help():
    for name, subparser in _subparsers(build_parser()).items():
        text = subparser.format_help()
        assert f"wexport {name}" in text
        assert "--key" in text
