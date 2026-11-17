"""The exit codes are the CLI contract (plan §2): one test per code."""

from __future__ import annotations

from pathlib import Path

from wexport.cli import main


def test_ok_is_zero(phone, tmp_path):
    code = main(["check", str(phone), "--key", "0123456789abcdef" * 4])
    assert code == 0


def test_missing_dependency_is_five(phone, monkeypatch):
    monkeypatch.setattr("shutil.which", lambda name: None)
    assert main(["check", str(phone)]) == 5


def test_unreadable_input_is_one(tmp_path):
    assert main(["check", str(tmp_path / "nope")]) == 1


def test_wrong_key_is_two(phone, tmp_path):
    wrong = "f" * 64
    code = main(["build", str(phone), "--key", wrong, "--out", str(tmp_path / "out")])
    assert code == 2


def test_check_verifies_the_key_instead_of_only_parsing_it(phone, capsys):
    code = main(["check", str(phone), "--key", "f" * 64])
    assert code == 2
    assert "does not match this backup" in capsys.readouterr().err


def test_build_without_media_folder_reports_placeholders(phone, tmp_path):
    """A copied folder without Media/ is a partial archive, not an error (plan §2)."""
    copy = tmp_path / "phone"
    copy.mkdir()
    for name in ("Databases", "Backups"):
        target = copy / name
        target.mkdir()
        for item in (phone / name).iterdir():
            target.joinpath(item.name).write_bytes(item.read_bytes())
    code = main(["build", str(copy), "--key", "0123456789abcdef" * 4, "--out", str(tmp_path / "out")])
    assert code == 0


def test_module_entry_point_works():
    """The packaged entry point is `python -m wexport`; a relative import there breaks the binary."""
    import subprocess
    import sys

    root = Path(__file__).resolve().parents[2]
    result = subprocess.run(
        [sys.executable, "-m", "wexport", "--version"],
        capture_output=True,
        text=True,
        cwd=root,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert "wexport" in result.stdout
