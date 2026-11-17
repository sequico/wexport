"""`check --chats`: choosing a chat before building anything."""

from __future__ import annotations

import pytest

from wexport.cli import main

KEY = "0123456789abcdef" * 4
NAMES = ["Alice", "Empty chat", "Family"]


def _order(output: str) -> list[str]:
    """The three fixture chats, in the order the listing printed them."""
    positions = {name: output.index(name) for name in NAMES}
    return sorted(NAMES, key=positions.__getitem__)


@pytest.fixture()
def listing(phone, capsys):
    def run(*extra: str) -> str:
        code = main(["check", str(phone), "--key", KEY, "--chats", *extra])
        assert code == 0
        return capsys.readouterr().out

    return run


def test_default_order_is_newest_last_event_first(listing):
    # Alice's last event is the call at 12:00, later than the Family group's last message.
    assert _order(listing()) == ["Alice", "Family", "Empty chat"]


def test_order_by_messages(listing):
    assert _order(listing("--sort", "messages")) == ["Family", "Alice", "Empty chat"]


def test_order_by_media_size(listing):
    assert _order(listing("--sort", "size")) == ["Family", "Alice", "Empty chat"]


def test_order_by_name_and_reverse(listing):
    assert _order(listing("--sort", "name")) == NAMES
    assert _order(listing("--sort", "name", "--reverse")) == list(reversed(NAMES))


def test_every_row_carries_the_last_message_and_a_jid(listing):
    output = listing("--tz", "Europe/Rome")
    assert "2026-12-02 12:00:00 +01:00" in output
    assert "120363000000000001@g.us" in output
    assert "3 chat(s)" in output


def test_listing_without_a_key_says_why(phone, capsys):
    assert main(["check", str(phone), "--chats"]) == 2
    assert "needs --key" in capsys.readouterr().err
