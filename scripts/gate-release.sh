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
# Each mutant's build is one thread. Eight mutants, each starting four compilers, is thirty-two
# compilers on four cores: the first run spent most of its wall time thrashing, not compiling.
export CARGO_BUILD_JOBS=1

echo "== release build"
cargo build --release --locked

echo "== release build (windows)"
# The same build the release ships; CI extracts the packaged zip and runs it on Windows.
cargo build --release --locked --target x86_64-pc-windows-gnu

echo "== mutations"
# One job per core (capped): 625 mutants at four seconds each is forty minutes in series.
jobs=$(nproc 2>/dev/null || echo 2)
[ "$jobs" -gt 8 ] && jobs=8
# Twice the jobs of cores: each mutant is a small build plus one test run, so the extra jobs hide
# their I/O without competing for a core. nextest starts the suite faster than cargo test does.
set -- --timeout 120 --jobs $((jobs * 2)) --test-tool nextest
for file in src/crypt15.rs src/keys.rs src/media.rs src/chunks.rs src/msgstore.rs src/merge.rs \
    src/discover.rs src/ios.rs src/container.rs src/archive.rs src/error.rs; do
    set -- "$@" --file "$file"
done
# Two mutants have no observable effect, so no test can kill them: the media batch size only decides
# how often buffered records are written (the archive is byte-identical either way), and a call id is
# negative while a message id is positive, so the two tuples are never equal.
set -- "$@" --exclude-re 'replace >= with < in build_chat'
set -- "$@" --exclude-re 'replace < with <= in Msgstore::each_message'
# A shift by eight leaves the low byte empty, so OR and XOR are the same operation.
set -- "$@" --exclude-re 'replace \| with \^ in (uint|annex_b|ebml_vint)'
# At the end of the data the next element is empty and the walk returns: one more iteration says
# nothing.
set -- "$@" --exclude-re 'replace < with <= in ebml_walk'
# With fewer than sixteen bytes read the missing largesize bytes are zeros, and the size check that
# follows stops the walk anyway.
set -- "$@" --exclude-re 'replace < with > in find_box'
# The mutation makes the offset stay zero, which is an endless loop: no test can observe it except by
# hanging, and a hang is not a wrong archive.
set -- "$@" --exclude-re 'replace \+= with \*= in (find_box|ebml_walk)'
# The memory test is left out of the test run: it measures an allocation ceiling, which no mutant of
# the parsers can move, and it costs six seconds on every one of a thousand mutants. Everything after
# `--` belongs to the test command, so it goes last.
set -- "$@" -- -E 'not binary(memory)'
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
