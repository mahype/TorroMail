#!/bin/sh
# Builds the SwiftPM app and wraps it into a runnable dev bundle at
# apps/TorroMailApp/.build/TorroMail.app (icon + localization included).
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG="$ROOT/apps/TorroMailApp"
APP="$PKG/.build/TorroMail.app"

swift build --package-path "$PKG" --scratch-path "$PKG/.build"
BIN="$(swift build --package-path "$PKG" --scratch-path "$PKG/.build" --show-bin-path)"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/TorroMailApp" "$APP/Contents/MacOS/TorroMail"
cp -R "$BIN/TorroMailApp_TorroMailApp.bundle" "$APP/Contents/Resources/"
cp "$PKG/Icon/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>TorroMail</string>
    <key>CFBundleIdentifier</key><string>com.torromail.app</string>
    <key>CFBundleName</key><string>TorroMail</string>
    <key>CFBundleDisplayName</key><string>TorroMail</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>3</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleLocalizations</key><array><string>en</string><string>de</string></array>
</dict>
</plist>
PLIST

echo "Bundle: $APP"
