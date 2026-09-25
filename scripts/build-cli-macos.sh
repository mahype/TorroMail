#!/usr/bin/env bash
# Builds the macOS terminal programs for a release:
#
#   torromail      the terminal control surface
#   torromail-mcp  the MCP server assistants spawn
#
# Both universal (arm64 + x86_64), signed with the Developer ID certificate
# under the hardened runtime, notarized, and packed side by side into
# dist/torromail-<version>-universal-macos.tar.gz with a .sha256 beside it.
#
# The signature is not decoration here. Keychain items are shared by Team ID:
# the app, its bundled server and these two programs read one another's
# secrets without a dialog only because all of them carry the same team. An
# unsigned build of this could not read a single password the app stored.
#
# A bare executable cannot carry a stapled ticket, so Gatekeeper looks the
# notarization up online on first start; the submission below only has to be
# accepted.
#
# Required environment:
#   VERSION                       e.g. 0.11.0 or 0.11.0-rc.1 (goes into the file name)
#   MACOS_SIGN_IDENTITY           "Developer ID Application: … (TEAMID)"
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_SPECIFIC_PASSWORD   for notarization
#
# NOTARIZE=0 skips notarization (a local dry run with a development identity).

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

: "${VERSION:?VERSION must be set}"
: "${MACOS_SIGN_IDENTITY:?MACOS_SIGN_IDENTITY must be set}"
notarize="${NOTARIZE:-1}"
if [[ "$notarize" != "0" ]]; then
    : "${APPLE_ID:?APPLE_ID must be set}"
    : "${APPLE_TEAM_ID:?APPLE_TEAM_ID must be set}"
    : "${APPLE_APP_SPECIFIC_PASSWORD:?APPLE_APP_SPECIFIC_PASSWORD must be set}"
fi

name="torromail-${VERSION}-universal-macos"
stage="dist/$name"
programs=(torromail torromail-mcp)

for target in aarch64-apple-darwin x86_64-apple-darwin; do
    echo "==> Building for $target"
    cargo build --release --locked --target "$target" -p torromail-mcp -p torromail-tui
done

rm -rf "$stage" "dist/$name.tar.gz" "dist/$name.tar.gz.sha256"
mkdir -p "$stage"
for program in "${programs[@]}"; do
    lipo -create \
        "target/aarch64-apple-darwin/release/$program" \
        "target/x86_64-apple-darwin/release/$program" \
        -output "$stage/$program"
    lipo -info "$stage/$program"
done

echo "==> Signing with hardened runtime"
for program in "${programs[@]}"; do
    codesign --force --timestamp --options=runtime --sign "$MACOS_SIGN_IDENTITY" "$stage/$program"
    codesign --verify --strict --verbose=2 "$stage/$program"
done
team="$(codesign -dv "$stage/torromail" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
if [[ -z "$team" || "$team" == "not set" ]]; then
    echo "error: the signature carries no Team ID — the keychain could not share secrets with the app" >&2
    exit 1
fi
echo "Team ID: $team"

echo "==> Smoke test"
"$stage/torromail" --version
"$stage/torromail-mcp" --list-tools > /dev/null

if [[ "$notarize" != "0" ]]; then
    echo "==> Submitting to Apple's notary service"
    submission="dist/$name-notarize.zip"
    rm -f "$submission"
    /usr/bin/ditto -c -k --keepParent "$stage" "$submission"
    result="$(xcrun notarytool submit "$submission" \
        --apple-id "$APPLE_ID" \
        --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_SPECIFIC_PASSWORD" \
        --wait --timeout 30m --output-format json)"
    rm -f "$submission"
    echo "$result"
    status="$(printf '%s' "$result" | /usr/bin/python3 -c 'import json, sys; print(json.load(sys.stdin).get("status", ""))')"
    if [[ "$status" != "Accepted" ]]; then
        echo "error: notarization ended with status '$status'" >&2
        exit 1
    fi
fi

cp README.md LICENSE-MIT LICENSE-APACHE "$stage/"
tar -C dist -czf "dist/$name.tar.gz" "$name"
(cd dist && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
rm -rf "$stage"
echo "==> Done: dist/$name.tar.gz"
