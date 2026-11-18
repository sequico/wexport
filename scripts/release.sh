#!/bin/sh
# The release artifacts: one binary per platform, each with its SHA-256, in dist/.
#
# The same two builds every time: a release that ships only one platform is not this product. The
# checks are here because a mistyped target is otherwise a silent, wrong download for a user.
set -eu
cd "$(dirname "$0")/.."

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

if ! rustup target list --installed | grep -qx x86_64-pc-windows-gnu; then
    echo "missing the Windows target: rustup target add x86_64-pc-windows-gnu" >&2
    exit 1
fi
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "missing the Windows linker: sudo apt install gcc-mingw-w64-x86-64" >&2
    exit 1
fi

echo "== build $version (linux x86-64)"
cargo build --release --locked
echo "== build $version (windows x86-64)"
cargo build --release --locked --target x86_64-pc-windows-gnu

rm -rf dist
mkdir -p dist
cp target/release/wexport dist/wexport-linux-x86_64
cp target/x86_64-pc-windows-gnu/release/wexport.exe dist/wexport-windows-x86_64.exe
chmod 0755 dist/wexport-linux-x86_64

# The two files must be the two different things they claim to be.
file dist/wexport-linux-x86_64 | grep -q "ELF 64-bit" || {
    echo "dist/wexport-linux-x86_64 is not an ELF binary" >&2
    exit 1
}
file dist/wexport-windows-x86_64.exe | grep -q "PE32+ executable" || {
    echo "dist/wexport-windows-x86_64.exe is not a PE32+ executable" >&2
    exit 1
}
dist/wexport-linux-x86_64 -v | grep -qx "wexport $version" || {
    echo "the Linux binary does not report $version" >&2
    exit 1
}

for asset in dist/wexport-linux-x86_64 dist/wexport-windows-x86_64.exe; do
    sha256sum "$asset" | awk '{print $1}' > "$asset.sha256"
done

echo
ls -l dist
echo
echo "tag v$version, then:"
echo "  gh release create v$version dist/* --title \"wexport $version\" --notes-file <notes>"
