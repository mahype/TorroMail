#!/bin/sh
# Builds the SwiftPM app and wraps it into a runnable dev bundle at
# apps/TorroMailApp/.build/TorroMail.app (icon + localization included).
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG="$ROOT/apps/TorroMailApp"
APP="$PKG/.build/TorroMail.app"

swift build --package-path "$PKG" --scratch-path "$PKG/.build"
BIN="$(swift build --package-path "$PKG" --scratch-path "$PKG/.build" --show-bin-path)"

# The app ships its own MCP server so launches never depend on cwd or PATH.
cargo build -p torromail-mcp --manifest-path "$ROOT/Cargo.toml"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/TorroMailApp" "$APP/Contents/MacOS/TorroMail"
cp "$ROOT/target/debug/torromail-mcp" "$APP/Contents/MacOS/torromail-mcp"
cp -R "$BIN/TorroMailApp_TorroMailApp.bundle" "$APP/Contents/Resources/"
cp "$PKG/Icon/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"

# Share the checked-in Info.plist template with the release build so the two
# never drift. Stamp a dev version so the bundle is identifiable.
cp "$PKG/Resources/Info.plist" "$APP/Contents/Info.plist"
DEV_VERSION="$(git -C "$ROOT" describe --tags --always --dirty 2>/dev/null | sed 's/^v//')"
/usr/libexec/PlistBuddy \
    -c "Set :CFBundleShortVersionString ${DEV_VERSION:-dev}" \
    "$APP/Contents/Info.plist" >/dev/null

echo "Bundle: $APP"
