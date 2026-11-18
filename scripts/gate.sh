#!/bin/sh
# The push gate: whole tree, about twenty seconds, nothing that takes minutes.
#
# This list is the gate. The README points here instead of keeping a second copy that drifts.
set -eu
cd "$(dirname "$0")/.."

# rustup installs here, and a non-login shell does not always know it.
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi

echo "== format"
cargo fmt --check

echo "== lints"
cargo clippy --all-targets --quiet

echo "== tests"
cargo nextest run

echo "== dependencies"
cargo deny check

echo "== windows"
cargo check --target x86_64-pc-windows-gnu --quiet

echo "gate: ok"
