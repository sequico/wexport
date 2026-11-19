#!/bin/sh
# The release artifacts: one archive per platform, each holding the binary as `wexport`, with a SHA-256
# beside it in dist/. The same two builds every time: a release that ships one platform is not this
# product, and a file without a platform in its name is a download the user has to guess about.
set -eu
cd "$(dirname "$0")/.."

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi
for tool in tar zip unzip; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing $tool: sudo apt install $tool" >&2
        exit 1
    fi
done

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

linux=target/release/wexport
windows=target/x86_64-pc-windows-gnu/release/wexport.exe
file "$linux" | grep -q "ELF 64-bit" || {
    echo "$linux is not an ELF binary" >&2
    exit 1
}
file "$windows" | grep -q "PE32+ executable" || {
    echo "$windows is not a PE32+ executable" >&2
    exit 1
}
"$linux" -v | grep -qx "wexport $version" || {
    echo "the Linux binary does not report $version" >&2
    exit 1
}

rm -rf dist
mkdir -p dist
dist=$(pwd)/dist
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
# The same bytes make the same archive: sorted entries, no owner names, one timestamp for everything.
epoch=$(git log -1 --format=%ct 2>/dev/null || echo 0)

cp "$linux" "$stage/wexport"
chmod 0755 "$stage/wexport"
touch -d "@$epoch" "$stage/wexport"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" \
    -czf "dist/wexport-$version-linux-x86_64.tar.gz" -C "$stage" wexport

cp "$windows" "$stage/wexport.exe"
chmod 0755 "$stage/wexport.exe"
touch -d "@$epoch" "$stage/wexport.exe"
(cd "$stage" && TZ=UTC zip -X -q -o "$dist/wexport-$version-windows-x86_64.zip" wexport.exe)

# Each archive holds the binary it claims, unchanged, and nothing else.
tar -tzf "dist/wexport-$version-linux-x86_64.tar.gz" | grep -qx wexport || {
    echo "the Linux archive does not hold wexport" >&2
    exit 1
}
[ "$(unzip -Z1 "dist/wexport-$version-windows-x86_64.zip")" = "wexport.exe" ] || {
    echo "the Windows archive does not hold wexport.exe" >&2
    exit 1
}
mkdir "$stage/check"
tar -xzf "dist/wexport-$version-linux-x86_64.tar.gz" -C "$stage/check"
[ "$(sha256sum < "$stage/check/wexport")" = "$(sha256sum < "$linux")" ] || {
    echo "the Linux archive does not hold the binary that was built" >&2
    exit 1
}
unzip -qq -o "dist/wexport-$version-windows-x86_64.zip" -d "$stage/check"
[ "$(sha256sum < "$stage/check/wexport.exe")" = "$(sha256sum < "$windows")" ] || {
    echo "the Windows archive does not hold the binary that was built" >&2
    exit 1
}

for asset in dist/wexport-"$version"-linux-x86_64.tar.gz dist/wexport-"$version"-windows-x86_64.zip; do
    sha256sum "$asset" | awk '{print $1}' > "$asset.sha256"
done

echo
ls -l dist
echo
echo "tag v$version, then:"
echo "  gh release create v$version dist/* --title \"wexport $version\" --notes-file <notes>"
