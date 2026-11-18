#!/usr/bin/env bash
# Build target/release/financi.app on macOS (3.63): the release binary, an
# Info.plist and an AppIcon.icns rendered from the committed PNGs with
# iconutil. The Dock then shows the app icon for the bundle; `cargo run`
# sets it at startup instead (window_icon::set_dock_icon).
# Re-run scripts/icons.sh first if assets/icon/png/ is missing or stale.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(uname)" != Darwin ]; then
    echo "bundle-macos.sh must run on macOS" >&2
    exit 1
fi

cargo build --release -p financi

# The version the app itself shows (3.39): build.rs bakes in
# `git describe --tags`; CFBundleShortVersionString wants digits only, so
# keep the tag minus the leading v and any -N-g<hash> suffix, falling back
# to the package version when git has nothing. The `|| true` keeps
# `set -o pipefail` from aborting the script when git is missing — the
# empty result is what reaches the check below.
version=$(git describe --tags --always 2>/dev/null | sed 's/^v//' | cut -d- -f1 || true)
if [ -z "$version" ]; then
    version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
fi

app=target/release/financi.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/financi "$app/Contents/MacOS/financi"

# iconutil wants the Apple iconset naming; the PNG sizes map onto it as
# 16/32/64/128/256/512/1024 → the 10 entries below.
iconset=$(mktemp -d)/financi.iconset
mkdir -p "$iconset"
cp assets/icon/png/financi-16.png "$iconset/icon_16x16.png"
cp assets/icon/png/financi-32.png "$iconset/icon_16x16@2x.png"
cp assets/icon/png/financi-32.png "$iconset/icon_32x32.png"
cp assets/icon/png/financi-64.png "$iconset/icon_32x32@2x.png"
cp assets/icon/png/financi-128.png "$iconset/icon_128x128.png"
cp assets/icon/png/financi-256.png "$iconset/icon_128x128@2x.png"
cp assets/icon/png/financi-256.png "$iconset/icon_256x256.png"
cp assets/icon/png/financi-512.png "$iconset/icon_256x256@2x.png"
cp assets/icon/png/financi-512.png "$iconset/icon_512x512.png"
cp assets/icon/png/financi-1024.png "$iconset/icon_512x512@2x.png"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>financi</string>
    <key>CFBundleIdentifier</key>
    <string>dev.financi.app</string>
    <key>CFBundleName</key>
    <string>financi</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon.icns</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
</dict>
</plist>
PLIST

echo "built $app (version $version)"
