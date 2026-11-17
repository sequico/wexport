"""Failures that carry the message a person reads and the exit code a script reads.

The exit codes are the CLI contract (plan §2):

    0  ok
    1  unreadable input
    2  unusable key
    3  wrong backup type (crypt14 / password / passkey)
    4  build incomplete (some derivatives could not be produced; the archive is usable)
    5  missing external dependency
"""

from __future__ import annotations

EXIT_OK = 0
EXIT_INPUT = 1
EXIT_KEY = 2
EXIT_BACKUP_TYPE = 3
EXIT_INCOMPLETE = 4
EXIT_DEPENDENCY = 5


class WexError(Exception):
    """An error the CLI reports as a message, not a traceback."""

    exit_code = EXIT_INPUT


class UnreadableInput(WexError):
    exit_code = EXIT_INPUT


class UnusableKey(WexError):
    exit_code = EXIT_KEY


class WrongBackupType(WexError):
    exit_code = EXIT_BACKUP_TYPE


class IncompleteBuild(WexError):
    exit_code = EXIT_INCOMPLETE


class MissingDependency(WexError):
    exit_code = EXIT_DEPENDENCY
