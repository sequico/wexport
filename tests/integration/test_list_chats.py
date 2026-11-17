"""`check --chats`: choosing a chat before building anything."""

from __future__ import annotations

import pytest

from wexport.cli import main

KEY = "0123456789abcdef" * 4
NAMES = ["Alice", "Empty chat", "Family"]


def _order(output: str) -> list[str]:
    """The fixture chats the listing printed, in the order printed."""
    positions = {name: output.index(name) for name in NAMES if name in output}
    return sorted(positions, key=positions.__getitem__)


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


def test_match_filters_by_name_or_jid(listing):
    assert _order(listing("--match", "fam")) == ["Family"]
    assert _order(listing("--match", "120363")) == ["Family"]
    assert _order(listing("--match", "nothing-like-this")) == []


def test_match_ignores_accents_and_case(listing):
    assert _order(listing("--match", "FAMILY")) == ["Family"]


def test_limit_truncates_and_says_how_to_see_more(listing):
    output = listing("--limit", "1")
    assert "1 of 3 chat(s) shown" in output
    assert "--limit 0" in output
    assert len(_order(output)) == 1


def test_limit_zero_shows_everything(listing):
    output = listing("--limit", "0")
    assert "3 of 3 chat(s) shown" in output
    assert len(_order(output)) == 3


def test_every_row_carries_the_last_message_and_a_jid(listing):
    output = listing("--tz", "Europe/Rome")
    assert "2026-12-02 12:00:00 +01:00" in output
    assert "120363000000000001@g.us" in output
    assert "3 chat(s)" in output


def test_listing_without_a_key_says_why(phone, capsys):
    assert main(["check", str(phone), "--chats"]) == 2
    assert "needs --key" in capsys.readouterr().err
