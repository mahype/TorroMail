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
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "$BIN/TorroMailApp" "$APP/Contents/MacOS/TorroMail"
cp "$ROOT/target/debug/torromail-mcp" "$APP/Contents/MacOS/torromail-mcp"
cp -R "$BIN/TorroMailApp_TorroMailApp.bundle" "$APP/Contents/Resources/"
cp "$PKG/Icon/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"

SPARKLE_FRAMEWORK="$PKG/.build/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64/Sparkle.framework"
if [ ! -d "$SPARKLE_FRAMEWORK" ]; then
    echo "error: Sparkle.framework not found at $SPARKLE_FRAMEWORK" >&2
    exit 1
fi
cp -R "$SPARKLE_FRAMEWORK" "$APP/Contents/Frameworks/"
if ! otool -l "$APP/Contents/MacOS/TorroMail" | grep -Fq '@executable_path/../Frameworks'; then
    install_name_tool -add_rpath "@executable_path/../Frameworks" "$APP/Contents/MacOS/TorroMail"
fi

# Share the checked-in Info.plist template with the release build so the two
# never drift. Stamp a dev version so the bundle is identifiable.
cp "$PKG/Resources/Info.plist" "$APP/Contents/Info.plist"
DEV_VERSION="$(git -C "$ROOT" describe --tags --always --dirty 2>/dev/null | sed 's/^v//')"
/usr/libexec/PlistBuddy \
    -c "Set :CFBundleShortVersionString ${DEV_VERSION:-dev}" \
    -c "Add :TorroMailDisableUpdates bool true" \
    "$APP/Contents/Info.plist" >/dev/null

# Sign both binaries with a stable team identity. The keychain grants read
# access to a stored password by the caller's signing Team ID, so the app and
# the bundled server — same team — reach it without a consent dialog, and the
# grant survives a rebuild. Adhoc/unsigned builds carry no team, so the
# keychain would fall back to prompting once per item on every rebuild.
# Sign the nested server first, then the bundle, so both executables are
# covered (a bundle signature does not reach a second Mach-O in MacOS/).
IDENTITY="${CODESIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Apple Development|Developer ID Application/ { print $2; exit }')}"
if [ -n "$IDENTITY" ]; then
    codesign --force --sign "$IDENTITY" "$APP/Contents/MacOS/torromail-mcp"
    codesign --force --deep --sign "$IDENTITY" "$APP"
    echo "Signed: $IDENTITY"
else
    echo "warning: no codesigning identity found; keychain will prompt per item" >&2
    echo "         set CODESIGN_IDENTITY or install an Apple Development certificate" >&2
fi

echo "Bundle: $APP"
