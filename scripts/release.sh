#!/bin/sh
# The release artifacts: one archive per platform, each holding the binary as `wexport` with the
# licence files beside it, and a SHA-256 beside each archive in dist/.
#
#   release.sh --macos            (on macOS) both Apple targets, joined with lipo, packaged
#   release.sh --stage DIR        (on Linux) build Linux and Windows, take the macOS archive from DIR
#
# A release without the macOS archive is not this product, so the Linux run refuses to finish one.
set -eu
cd "$(dirname "$0")/.."

stage_dir=""
macos=0
while [ $# -gt 0 ]; do
    case "$1" in
        --macos) macos=1; shift ;;
        --stage)
            [ $# -ge 2 ] || { echo "--stage needs a folder" >&2; exit 1; }
            stage_dir=$2
            shift 2
            ;;
        *)
            echo "unknown option: $1" >&2
            exit 1
            ;;
    esac
done

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi
for tool in tar zip unzip; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing $tool: install it and run again" >&2
        exit 1
    fi
done
for file in LICENSE THIRD-PARTY.md; do
    [ -f "$file" ] || { echo "missing $file: a binary distribution carries it" >&2; exit 1; }
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

echo "== wexport $version"
rm -rf dist
mkdir -p dist
dist=$(pwd)/dist
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
epoch=$(git log -1 --format=%ct 2>/dev/null || echo 0)
for file in LICENSE THIRD-PARTY.md; do
    cp "$file" "$stage/$file"
done

# The same bytes make the same archive: sorted entries, no owner names, one timestamp for everything.
if [ "$macos" = 1 ]; then
    stamp=$(date -r "$epoch" +%Y%m%d%H%M.%S 2>/dev/null || echo 202001010000.00)
else
    stamp=$(date -d "@$epoch" +%Y%m%d%H%M.%S 2>/dev/null || echo 202001010000.00)
fi
touch -t "$stamp" "$stage"/*

if [ "$macos" = 1 ]; then
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        rustup target list --installed | grep -qx "$target" || rustup target add "$target"
        echo "== build $version ($target)"
        cargo build --release --locked --target "$target"
    done
    lipo -create \
        -output "$stage/wexport" \
        "target/aarch64-apple-darwin/release/wexport" \
        "target/x86_64-apple-darwin/release/wexport"
    chmod 0755 "$stage/wexport"
    touch -t "$stamp" "$stage/wexport"
    arches=$(lipo -archs "$stage/wexport")
    case "$arches" in
        *arm64*) ;;
        *) echo "the macOS binary has no arm64 slice: $arches" >&2; exit 1 ;;
    esac
    case "$arches" in
        *x86_64*) ;;
        *) echo "the macOS binary has no x86_64 slice: $arches" >&2; exit 1 ;;
    esac
    "$stage/wexport" -v | grep -qx "wexport $version" || {
        echo "the macOS binary does not report $version" >&2
        exit 1
    }
    codesign -dv "$stage/wexport" >/dev/null 2>&1 || {
        echo "the macOS binary carries no signature at all" >&2
        exit 1
    }
    archive="dist/wexport-$version-macos-universal.tar.gz"
    (cd "$stage" && COPYFILE_DISABLE=1 tar -czf "$dist/wexport-$version-macos-universal.tar.gz" \
        wexport LICENSE THIRD-PARTY.md)
    mkdir "$stage/check"
    tar -xzf "$archive" -C "$stage/check"
    [ "$(shasum -a 256 < "$stage/check/wexport")" = "$(shasum -a 256 < "$stage/wexport")" ] || {
        echo "the macOS archive does not hold the binary that was built" >&2
        exit 1
    }
    [ -f "$stage/check/LICENSE" ] && [ -f "$stage/check/THIRD-PARTY.md" ] || {
        echo "the macOS archive is missing its licence files" >&2
        exit 1
    }
    # Two fields with the bare name, so `sha256sum -c` works in the folder a user downloaded to.
    digest=$(shasum -a 256 "$archive" | awk '{print $1}')
    printf '%s  %s\n' "$digest" "$(basename "$archive")" > "$archive.sha256"
    echo
    ls -l dist
    exit 0
fi

if ! rustup target list --installed | grep -qx x86_64-pc-windows-gnu; then
    echo "missing the Windows target: rustup target add x86_64-pc-windows-gnu" >&2
    exit 1
fi
for compiler in x86_64-w64-mingw32-gcc x86_64-w64-mingw32-g++; do
    if ! command -v "$compiler" >/dev/null 2>&1; then
        echo "missing the Windows C/C++ compiler: sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64" >&2
        exit 1
    fi
done
macos_asset="wexport-$version-macos-universal.tar.gz"
if [ -z "$stage_dir" ] || [ ! -f "$stage_dir/$macos_asset" ]; then
    echo "missing the macOS archive: run the macOS job and pass --stage <folder>" >&2
    echo "  gh run download <run-id> -n dist-macos -D dist-macos" >&2
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

cp "$linux" "$stage/wexport"
chmod 0755 "$stage/wexport"
touch -t "$stamp" "$stage/wexport"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" \
    -czf "dist/wexport-$version-linux-x86_64.tar.gz" \
    -C "$stage" wexport LICENSE THIRD-PARTY.md

cp "$windows" "$stage/wexport.exe"
chmod 0755 "$stage/wexport.exe"
touch -t "$stamp" "$stage/wexport.exe"
(cd "$stage" && TZ=UTC zip -X -q -o "$dist/wexport-$version-windows-x86_64.zip" \
    wexport.exe LICENSE THIRD-PARTY.md)

cp "$stage_dir/$macos_asset" "dist/$macos_asset"

verify_archive() {
    archive=$1
    tar -tzf "$archive" | grep -qx "LICENSE" || {
        echo "$archive does not hold LICENSE" >&2
        exit 1
    }
    tar -tzf "$archive" | grep -qx "THIRD-PARTY.md" || {
        echo "$archive does not hold THIRD-PARTY.md" >&2
        exit 1
    }
}
verify_archive "dist/wexport-$version-linux-x86_64.tar.gz"
verify_archive "dist/$macos_asset"
[ "$(unzip -Z1 "dist/wexport-$version-windows-x86_64.zip" | sort | tr '\n' ' ')" \
    = "LICENSE THIRD-PARTY.md wexport.exe " ] || {
    echo "the Windows archive does not hold the binary and the licences" >&2
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

for asset in \
    "dist/wexport-$version-linux-x86_64.tar.gz" \
    "dist/wexport-$version-windows-x86_64.zip" \
    "dist/$macos_asset"; do
    digest=$(sha256sum "$asset" | awk '{print $1}')
    printf '%s  %s\n' "$digest" "$(basename "$asset")" > "$asset.sha256"
done

echo
ls -l dist
echo
echo "tag v$version, then:"
echo "  gh release create v$version dist/* --title \"wexport $version\" --notes-file <notes> --target \$(git rev-parse HEAD)"
