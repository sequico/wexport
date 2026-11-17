"""Preflight: recognise the backup, refuse what cannot be opened, name what is missing."""

from __future__ import annotations

import shutil

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


def test_a_crypt14_chain_next_to_crypt15_is_reported_not_used(phone, tmp_path):
    """A real phone keeps the old crypt14 files; they must be named as ignored, never mixed in."""
    copy = tmp_path / "phone"
    shutil.copytree(phone, copy)
    databases = copy / "Databases"
    for name in (
        "msgstore-2026-11-11.1.db.crypt14",
        "msgstore-increment-1-2026-11-17.db.crypt14",
    ):
        (databases / name).write_bytes(b"")
    inputs = preflight.discover(copy)
    assert inputs.database.name == "msgstore.db.crypt15"
    assert inputs.incrementals == ()
    assert {path.name for path in inputs.ignored} == {
        "msgstore-2026-11-11.1.db.crypt14",
        "msgstore-increment-1-2026-11-17.db.crypt14",
    }


def test_a_nested_android_media_copy_is_found(phone, tmp_path):
    """An MTP drag often keeps Android/media/com.whatsapp/WhatsApp: find the backup anyway."""
    nested = tmp_path / "Chats" / "Android" / "media" / "com.whatsapp" / "WhatsApp"
    shutil.copytree(phone, nested)
    inputs = preflight.discover(tmp_path / "Chats")
    assert inputs.database.name == "msgstore.db.crypt15"
    assert inputs.media is not None


def test_pointing_straight_at_the_databases_folder_works(phone, tmp_path):
    copy = tmp_path / "phone"
    shutil.copytree(phone, copy)
    inputs = preflight.discover(copy / "Databases")
    assert inputs.database.name == "msgstore.db.crypt15"


def test_pointing_at_the_whatsapp_folder_works(phone, tmp_path):
    copy = tmp_path / "phone"
    shutil.copytree(phone, copy)
    inputs = preflight.discover(copy / "Databases" / "msgstore.db.crypt15")
    assert inputs.database.name == "msgstore.db.crypt15"


def test_a_folder_without_a_backup_says_what_to_copy(tmp_path):
    from wexport.errors import UnreadableInput

    (tmp_path / "Chats").mkdir()
    (tmp_path / "Chats" / "notes.txt").write_text("nothing here")
    with pytest.raises(UnreadableInput) as error:
        preflight.discover(tmp_path / "Chats")
    assert "Databases/msgstore.db.crypt15" in str(error.value)


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
