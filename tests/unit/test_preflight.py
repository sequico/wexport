"""Preflight: recognise the backup, refuse what cannot be opened, name what is missing."""

from __future__ import annotations

import pytest

from wexport import preflight
from wexport.errors import MissingDependency, WrongBackupType


def test_discover_finds_the_backup_and_media(phone):
    inputs = preflight.discover(phone)
    assert inputs.database.name == "msgstore.db.crypt15"
    assert inputs.media is not None and inputs.media.is_dir()
    assert inputs.contacts is not None and inputs.contacts.name == "wa.db.crypt15"


def test_header_names_the_key_type(phone):
    header = preflight.read_header(phone / "Databases" / "msgstore.db.crypt15")
    assert header.format == "crypt15"
    assert header.key_type == "E2EE_ENCRYPTION_KEY"
    assert header.app_version


def test_crypt14_is_refused_with_the_way_out(tmp_path):
    """`discover` owns this refusal, because only it knows the folder the files are in."""
    databases = tmp_path / "Databases"
    databases.mkdir()
    (databases / "msgstore.db.crypt14").write_bytes(b"")
    with pytest.raises(WrongBackupType) as error:
        preflight.discover(tmp_path)
    assert "64-digit key" in str(error.value)


def test_only_incrementals_is_refused_with_the_way_out(tmp_path):
    databases = tmp_path / "Databases"
    databases.mkdir()
    (databases / "msgstore-increment-2026-12-02.db.crypt15").write_bytes(b"")
    with pytest.raises(Exception) as error:
        preflight.discover(tmp_path)
    assert "Back up now" in str(error.value)


def test_header_of_another_format_is_refused():
    header = preflight.Header(path=__file__, format="crypt12", app_version="", key_type="", size=1)
    with pytest.raises(WrongBackupType) as error:
        preflight.require_openable(header)
    assert "only crypt15" in str(error.value)


def test_password_backup_explains_the_hsm():
    header = preflight.Header(path=__file__, format="crypt15", app_version="", key_type="E2EE_PASSWORD", size=1)
    with pytest.raises(WrongBackupType) as error:
        preflight.require_openable(header)
    assert "password only asks WhatsApp for the key" in str(error.value)


def test_missing_binary_lists_install_commands(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda name: None)
    with pytest.raises(MissingDependency) as error:
        preflight.require_binaries("ffmpeg")
    message = str(error.value)
    assert "apt install ffmpeg" in message and "winget" in message


def test_missing_folder_is_an_input_error(tmp_path):
    from wexport.errors import UnreadableInput

    with pytest.raises(UnreadableInput):
        preflight.discover(tmp_path / "absent")
