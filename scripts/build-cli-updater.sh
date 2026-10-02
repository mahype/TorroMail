#!/usr/bin/env bash
# Build Sparkle's official CLI drivers with TorroMail's restricted entrypoint.
# Resolve/build the Swift package and embed Sparkle.framework first. Source
# and binary come from the same Package.resolved revision; no second download.
# Usage: build-cli-updater.sh <TorroMail.app> [--universal]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
app="${1:?app path required}"
arch_args=(-arch "$(uname -m)")
case "${2:-}" in
    '') ;;
    --universal) arch_args=(-arch arm64 -arch x86_64) ;;
    *) echo 'usage: build-cli-updater.sh <app> [--universal]' >&2; exit 2 ;;
esac
sparkle_src="$root/apps/TorroMailApp/.build/checkouts/Sparkle"
helper="$app/Contents/Helpers/TorroMailUpdater.app"
mkdir -p "$helper/Contents/MacOS" "$helper/Contents/Resources"
cp "$root/apps/TorroMailApp/CLIUpdater/Info.plist" "$helper/Contents/Info.plist"
cp "$sparkle_src/LICENSE" "$helper/Contents/Resources/Sparkle-LICENSE.txt"
xcrun clang "${arch_args[@]}" -mmacosx-version-min=14.0 -fobjc-arc \
    '-DSPU_OBJC_DIRECT=__attribute__((objc_direct))' \
    '-DSPU_OBJC_DIRECT_MEMBERS=__attribute__((objc_direct_members))' \
    -include Sparkle/Sparkle.h -I "$sparkle_src/sparkle-cli" \
    -F "$app/Contents/Frameworks" -framework Cocoa -framework Sparkle \
    -Wl,-rpath,@executable_path/../../../../Frameworks \
    "$root/apps/TorroMailApp/CLIUpdater/main.m" \
    "$sparkle_src/sparkle-cli/SPUCommandLineDriver.m" \
    "$sparkle_src/sparkle-cli/SPUCommandLineUserDriver.m" \
    -o "$helper/Contents/MacOS/TorroMailUpdater"
