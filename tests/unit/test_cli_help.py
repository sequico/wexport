"""`wexport -h` must document every flag, from the same data that builds the parser."""

from __future__ import annotations

from wexport.cli import FLAGS, build_parser


def test_top_level_help_mentions_every_option():
    parser = build_parser()
    help_text = parser.format_help()
    for action in parser._actions:
        for option in action.option_strings:
            assert option in help_text, f"{option} is missing from `wexport -h`"


def test_help_is_generated_from_the_same_flag_data():
    parser = build_parser()
    documented = {option for action in parser._actions for option in action.option_strings}
    declared = {name for flag in FLAGS for name in flag["names"] if name.startswith("-")}
    # --version and -h come from argparse itself, and every declared flag must really exist.
    assert declared <= documented
    assert {"--key", "--chat", "--list", "--yes", "--out"} <= documented


def test_help_carries_exit_codes_requirements_and_examples():
    text = build_parser().format_help()
    for code in ("1", "2", "3", "4", "5"):
        assert f"  {code}  " in text
    assert "ffmpeg" in text and "tesseract" in text
    assert "examples:" in text


def test_positional_folder_is_optional():
    args = build_parser().parse_args([])
    assert args.input == []
    args = build_parser().parse_args(["/tmp/x"])
    assert args.input == ["/tmp/x"]
