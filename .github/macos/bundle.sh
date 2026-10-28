#!/bin/bash
# Makes the macOS release files from the release binary, in the current
# directory: gutenberg-<target>.tar.gz with the bare binary (what install.sh
# unpacks into ~/.local/bin) and Gutenberg-<target>.zip with Gutenberg.app.
# Both are signed ad hoc (`codesign -s -`): there is no Developer ID, and
# arm64 macOS won't run unsigned code, which a stripped binary is.
# Usage: bundle.sh <gutenberg binary> <target triple> <version>
# Needs rsvg-convert (brew install librsvg) for the icon; strip, codesign,
# plutil, iconutil and ditto come with macOS.
set -euo pipefail
bin=$1
target=$2
version=$3
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)

mkdir -p dist/gutenberg
cp "$bin" dist/gutenberg/gutenberg
# strip -x keeps the symbols the dynamic linker needs and drops the
# linker's ad-hoc signature with the rest, so the binary is signed again.
strip -x dist/gutenberg/gutenberg
codesign --force --sign - dist/gutenberg/gutenberg
cp "$root/LICENSE" "$root/CHANGELOG.md" dist/gutenberg/
dist/gutenberg/gutenberg --version
# No AppleDouble (._*) entries: tar on Linux would unpack them as files.
COPYFILE_DISABLE=1 tar -C dist -czf "gutenberg-$target.tar.gz" gutenberg

app=dist/Gutenberg.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
sed "s/@VERSION@/$version/g" "$here/Info.plist.in" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
cp dist/gutenberg/gutenberg "$app/Contents/MacOS/gutenberg"

# The icon: the SVG the Linux desktop entry uses, at each size iconutil
# wants, the @2x ones at twice their nominal pixels.
iconset=dist/gutenberg.iconset
rm -rf "$iconset"
mkdir -p "$iconset"
svg="$root/share/icons/hicolor/scalable/apps/gutenberg.svg"
for size in 16 32 128 256 512; do
  rsvg-convert -w "$size" -h "$size" "$svg" > "$iconset/icon_${size}x${size}.png"
  rsvg-convert -w "$((size * 2))" -h "$((size * 2))" "$svg" > "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/gutenberg.icns"

# Nothing is nested in the bundle, so one signature covers the executable
# and seals Info.plist and the icon.
codesign --force --sign - "$app"
codesign --verify --strict --verbose=2 "$app"
"$app/Contents/MacOS/gutenberg" --version
# ditto keeps the bundle's structure and metadata as Finder would.
ditto -c -k --keepParent "$app" "Gutenberg-$target.zip"
ls -l "gutenberg-$target.tar.gz" "Gutenberg-$target.zip"
