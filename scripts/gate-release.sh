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

# Mutation testing copies the tree per job and builds it there: on a machine whose /tmp is a small
# tmpfs that runs out of space before it runs out of mutants. The work goes under target/ instead.
TMPDIR="$PWD/target/tmp"
mkdir -p "$TMPDIR"
export TMPDIR

echo "== release build"
cargo build --release --locked

echo "== release build (windows)"
# The same build the release ships; CI extracts the packaged zip and runs it on Windows.
cargo build --release --locked --target x86_64-pc-windows-gnu

echo "== mutations"
# One job per core (capped): 625 mutants at four seconds each is forty minutes in series.
jobs=$(nproc 2>/dev/null || echo 2)
[ "$jobs" -gt 8 ] && jobs=8
# Mutation testing judges logic. The presentation modules (ui, report, progress) are mostly println
# and arithmetic on numbers already asserted end-to-end; mutating them says nothing about the product.
set -- --timeout 180 --jobs "$jobs"
for file in src/crypt15.rs src/keys.rs src/media.rs src/chunks.rs src/msgstore.rs src/merge.rs \
    src/discover.rs src/ios.rs src/container.rs src/archive.rs src/error.rs; do
    set -- "$@" --file "$file"
done
# Two mutants have no observable effect, so no test can kill them: the media batch size only decides
# how often buffered records are written (the archive is byte-identical either way), and a call id is
# negative while a message id is positive, so the two tuples are never equal.
set -- "$@" --exclude-re 'replace >= with < in build_chat'
set -- "$@" --exclude-re 'replace < with <= in Msgstore::each_message'
cargo mutants "$@"

echo "== fuzzing"
# The fuzz crate builds under target/, which mutation testing never copies: in fuzz/target it made
# every one of the four mutant copies 700 MB and filled /tmp.
for target in parse_header decrypt normalize_hex; do
    echo "-- $target"
    CARGO_TARGET_DIR="$PWD/target/fuzz" cargo +nightly fuzz run "$target" -- -max_total_time=60
done

echo "== thread sanitizer"
rustup component add rust-src --toolchain nightly >/dev/null 2>&1 || true
RUSTFLAGS="-Zsanitizer=thread" cargo +nightly test -Zbuild-std \
    --target x86_64-unknown-linux-gnu --test fixture

echo "release gate: ok"
