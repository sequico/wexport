"""`python -m wexport`, and the entry point the one-file binary compiles.

Absolute imports on purpose: this module is executed as a script by the packager, where relative
imports have no parent package.
"""

from __future__ import annotations

import sys

from wexport.cli import main

if __name__ == "__main__":
    sys.exit(main())
