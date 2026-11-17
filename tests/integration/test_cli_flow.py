"""The command: one folder, two questions, five exit codes."""

from __future__ import annotations

import builtins
from pathlib import Path

from wexport.cli import main

KEY = "0123456789abcdef" * 4


def _stdin_is_a_terminal(monkeypatch, answers: list[str] | None = None) -> None:
    monkeypatch.setattr("sys.stdin.isatty", lambda: True)
    if answers is not None:
        remaining = iter(answers)
        monkeypatch.setattr(builtins, "input", lambda *_: next(remaining))


def test_list_only_looks_and_writes_nothing(phone, tmp_path, capsys):
    assert main([str(phone), "--key", KEY, "--list"]) == 0
    out = capsys.readouterr().out
    assert "Family" in out and "3 chat(s)" in out
    assert not (phone / "archives").exists()
    assert list(tmp_path.iterdir()) == []


def test_numbers_select_a_chat(tmp_path, phone, capsys):
    out = tmp_path / "out"
    # row 1 is the most recent conversation: Alice (the call), row 2 is Family
    assert main([str(phone), "--key", KEY, "--chat", "2", "--out", str(out)]) == 0
    assert [p.name for p in out.iterdir() if p.is_dir()] == ["2026-12-02-family-9fd6"]


def test_names_select_chats_case_and_accent_insensitively(phone, tmp_path):
    out = tmp_path / "out"
    assert main([str(phone), "--key", KEY, "--chat", "FAMILY", "--out", str(out)]) == 0
    assert (out / "2026-12-02-family-9fd6").is_dir()


def test_all_selects_everything(phone, tmp_path):
    out = tmp_path / "out"
    assert main([str(phone), "--key", KEY, "--chat", "all", "--out", str(out)]) == 0
    assert len([p for p in out.iterdir() if p.is_dir()]) == 2  # the empty chat is skipped


def test_yes_builds_everything_without_asking(phone, tmp_path):
    out = tmp_path / "out"
    assert main([str(phone), "--key", KEY, "--yes", "--out", str(out)]) == 0
    assert (out / "index.html").is_file()


def test_interactive_pick_by_number(phone, tmp_path, monkeypatch):
    out = tmp_path / "out"
    _stdin_is_a_terminal(monkeypatch, ["2"])
    assert main([str(phone), "--key", KEY, "--out", str(out)]) == 0
    assert [p.name for p in out.iterdir() if p.is_dir()] == ["2026-12-02-family-9fd6"]


def test_interactive_empty_answer_takes_the_most_recent(phone, tmp_path, monkeypatch):
    out = tmp_path / "out"
    _stdin_is_a_terminal(monkeypatch, [""])
    assert main([str(phone), "--key", KEY, "--out", str(out)]) == 0
    assert (out / "2026-12-02-alice-8c02").is_dir()


def test_interactive_reprompts_on_nonsense(phone, tmp_path, monkeypatch, capsys):
    out = tmp_path / "out"
    _stdin_is_a_terminal(monkeypatch, ["zzz", "family"])
    assert main([str(phone), "--key", KEY, "--out", str(out)]) == 0
    assert "nothing matched" in capsys.readouterr().out


def test_no_terminal_without_chat_or_yes_says_what_to_pass(phone, monkeypatch, capsys):
    monkeypatch.setattr("sys.stdin.isatty", lambda: False)
    assert main([str(phone), "--key", KEY]) == 1
    assert "--chat" in capsys.readouterr().err


def test_default_output_folder_is_inside_the_copy(phone, tmp_path):
    copy = tmp_path / "phone"
    import shutil

    shutil.copytree(phone, copy)
    assert main([str(copy), "--key", KEY, "--chat", "2"]) == 0
    assert (copy / "archives" / "2026-12-02-family-9fd6" / "index.html").is_file()


def test_the_old_subcommand_is_answered_with_the_new_form(phone, capsys):
    assert main(["check", str(phone)]) == 1
    message = capsys.readouterr().err
    assert "is not a command any more" in message
    assert "wexport <folder> --list" in message


def test_key_is_looked_up_next_to_the_backup(phone, tmp_path):
    copy = tmp_path / "phone"
    import shutil

    shutil.copytree(phone, copy)
    (copy / "Databases" / "key.txt").write_text(KEY)
    out = tmp_path / "out"
    assert main([str(copy), "--chat", "2", "--out", str(out)]) == 0


def test_missing_key_without_a_terminal_explains(phone, monkeypatch, capsys):
    monkeypatch.setattr("sys.stdin.isatty", lambda: False)
    assert main([str(phone), "--list"]) == 2
    assert "--key" in capsys.readouterr().err


def test_old_chain_files_are_named_not_silently_skipped(phone, tmp_path, capsys):
    copy = tmp_path / "phone"
    import shutil

    shutil.copytree(phone, copy)
    (copy / "Databases" / "msgstore-2026-11-11.1.db.crypt14").write_bytes(b"")
    assert main([str(copy), "--key", KEY, "--list"]) == 0
    assert "older backup chain (crypt14)" in capsys.readouterr().out


def test_missing_dependency_is_five(phone, monkeypatch):
    monkeypatch.setattr("shutil.which", lambda name: None)
    assert main([str(phone), "--key", KEY]) == 5


def test_a_crypt14_only_folder_is_refused_with_three(tmp_path):
    databases = tmp_path / "Databases"
    databases.mkdir()
    (databases / "msgstore.db.crypt14").write_bytes(b"")
    assert main([str(tmp_path), "--key", KEY]) == 3


def test_the_packaged_entry_point_runs():
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
