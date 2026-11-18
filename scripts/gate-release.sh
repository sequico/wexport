#!/bin/sh
# The release gate: everything the push gate runs, plus the slow proofs.
#
# Mutation testing, fuzzing and ThreadSanitizer each cost minutes, which is why they run once per
# release and not on every push.
set -eu
cd "$(dirname "$0")/.."

# rustup installs here, and a non-login shell does not always know it.
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi

for tool in cargo-mutants cargo-fuzz; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing $tool: cargo install --locked $tool" >&2
        exit 1
    fi
done

./scripts/gate.sh

echo "== release build"
cargo build --release --locked

echo "== mutations"
# One job per core (capped): 625 mutants at four seconds each is forty minutes in series.
jobs=$(nproc 2>/dev/null || echo 2)
[ "$jobs" -gt 8 ] && jobs=8
cargo mutants --timeout 180 --jobs "$jobs"

echo "== fuzzing"
for target in parse_header decrypt normalize_hex; do
    echo "-- $target"
    cargo +nightly fuzz run "$target" -- -max_total_time=60
done

echo "== thread sanitizer"
rustup component add rust-src --toolchain nightly >/dev/null 2>&1 || true
RUSTFLAGS="-Zsanitizer=thread" cargo +nightly test -Zbuild-std \
    --target x86_64-unknown-linux-gnu --test fixture

echo "release gate: ok"
