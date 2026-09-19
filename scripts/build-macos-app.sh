#!/usr/bin/env bash
# Builds a release TorroMail.app bundle at dist/TorroMail.app.
#
# On a machine with full Xcode the bundle is universal (arm64 + x86_64); with
# only the Command Line Tools it falls back to the host architecture. By default
# the bundle is ad-hoc signed — good enough to run locally. For a signed +
# notarized release, chain this with scripts/codesign-macos.sh and
# scripts/build-dmg.sh; see docs/RELEASING.md.
#
# Unlike the dev helper scripts/make-app-bundle.sh (debug, host-arch, fast),
# this produces the artifact shipped by the release workflow.
#
# Environment:
#   VERSION              Overrides version derived from `git describe`.
#                        Defaults to `git describe --tags --always --dirty` with
#                        the leading `v` stripped. Outside a git checkout, falls
#                        back to the Cargo.toml workspace version.
#   MACOS_SIGN_IDENTITY  When set, sign with hardened runtime instead of ad-hoc.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

pkg="apps/TorroMailApp"
entitlements="$pkg/Resources/TorroMail.entitlements"
info_plist_src="$pkg/Resources/Info.plist"

if [[ "${REQUIRE_SPARKLE_KEY:-0}" == "1" && -z "${SPARKLE_ED_PUBLIC_KEY:-}" ]]; then
    echo "error: SPARKLE_ED_PUBLIC_KEY is required for a release build" >&2
    exit 1
fi
if [[ -n "${SPARKLE_ED_PUBLIC_KEY:-}" ]]; then
    if ! sparkle_key_bytes="$(printf '%s' "$SPARKLE_ED_PUBLIC_KEY" | base64 --decode 2>/dev/null | wc -c | tr -d ' ')"; then
        sparkle_key_bytes="invalid"
    fi
    if [[ "$sparkle_key_bytes" != "32" ]]; then
        echo "error: SPARKLE_ED_PUBLIC_KEY must be a base64-encoded 32-byte Ed25519 key" >&2
        exit 1
    fi
fi

# --- Version ------------------------------------------------------------------

# No `--always`: on a repo with no tags yet, `git describe --always` returns a
# bare commit hash, which would land in CFBundleVersion as a non-version string.
# Letting describe fail here falls through to the Cargo.toml version instead.
if [[ -z "${VERSION:-}" ]]; then
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        VERSION="$(git describe --tags --match 'v*' --dirty 2>/dev/null | sed 's/^v//' || true)"
    fi
fi
if [[ -z "${VERSION:-}" ]]; then
    VERSION="$(awk -F'"' '/^version/ {print $2; exit}' Cargo.toml)"
fi
export VERSION
echo "==> Building TorroMail $VERSION"

# --- Detect whether we can build universal (requires full Xcode) --------------

xcode_dev_path="$(xcode-select -p 2>/dev/null || true)"
# A full Xcode developer dir ends in `.app/Contents/Developer` (including the
# versioned /Applications/Xcode_16.app that GitHub-hosted runners use). Command
# Line Tools live at /Library/Developer/CommandLineTools and cannot build
# universal binaries.
if [[ "$xcode_dev_path" == *.app/Contents/Developer ]]; then
    build_universal=true
else
    build_universal=false
    echo "==> NOTE: Command Line Tools detected (no full Xcode at $xcode_dev_path)."
    echo "         Building host-architecture only. For a universal release"
    echo "         artifact, install Xcode and run"
    echo "         \`sudo xcode-select -s /Applications/Xcode.app\`."
fi

native_arch="$(uname -m)"
case "$native_arch" in
    arm64)   native_rust_target="aarch64-apple-darwin" ;;
    x86_64)  native_rust_target="x86_64-apple-darwin" ;;
    *)       echo "error: unsupported host architecture $native_arch" >&2; exit 1 ;;
esac

# --- Rust MCP server binary ---------------------------------------------------
# torromail-mcp is a standalone executable that the app supervises over stdio;
# it is copied into the bundle, not linked into the Swift binary.

if $build_universal; then
    echo "==> Building torromail-mcp for aarch64-apple-darwin"
    cargo build --release --target aarch64-apple-darwin -p torromail-mcp
    echo "==> Building torromail-mcp for x86_64-apple-darwin"
    cargo build --release --target x86_64-apple-darwin -p torromail-mcp
    echo "==> Lipo'ing universal torromail-mcp"
    mkdir -p target/universal/release
    lipo -create \
        target/aarch64-apple-darwin/release/torromail-mcp \
        target/x86_64-apple-darwin/release/torromail-mcp \
        -output target/universal/release/torromail-mcp
    mcp_bin="target/universal/release/torromail-mcp"
else
    echo "==> Building torromail-mcp for $native_rust_target"
    cargo build --release --target "$native_rust_target" -p torromail-mcp
    mcp_bin="target/$native_rust_target/release/torromail-mcp"
fi
lipo -info "$mcp_bin"

# --- Swift executable ---------------------------------------------------------

if $build_universal; then
    echo "==> Building universal Swift executable (arm64 + x86_64)"
    swift build -c release --arch arm64 --arch x86_64 \
        --package-path "$pkg" --scratch-path "$pkg/.build"
else
    echo "==> Building Swift executable ($native_arch only)"
    swift build -c release \
        --package-path "$pkg" --scratch-path "$pkg/.build"
fi
bin_dir="$(swift build -c release \
    $($build_universal && echo --arch arm64 --arch x86_64) \
    --package-path "$pkg" --scratch-path "$pkg/.build" --show-bin-path)"
swift_build_bin="$bin_dir/TorroMailApp"

if [[ ! -f "$swift_build_bin" ]]; then
    echo "error: Swift build did not produce $swift_build_bin" >&2
    exit 1
fi
lipo -info "$swift_build_bin" || true

# Fail loudly if a universal build was requested but the binary is not fat —
# an arm64-only artifact would refuse to launch on Intel Macs.
if $build_universal; then
    archs="$(lipo -archs "$swift_build_bin" 2>/dev/null || true)"
    if [[ "$archs" != *arm64* || "$archs" != *x86_64* ]]; then
        echo "error: universal build requested but binary archs are '$archs'" >&2
        echo "       expected both arm64 and x86_64" >&2
        exit 1
    fi
    echo "==> Verified universal binary: $archs"
fi

# --- Assemble .app bundle -----------------------------------------------------

app="dist/TorroMail.app"
echo "==> Assembling $app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp "$swift_build_bin" "$app/Contents/MacOS/TorroMail"

# The app ships its own MCP server so launches never depend on cwd or PATH.
cp "$mcp_bin" "$app/Contents/MacOS/torromail-mcp"
chmod +x "$app/Contents/MacOS/torromail-mcp"

# SwiftPM resource bundle (localization tables, bundled font) — the app resolves
# these via Bundle.module, which points at this bundle inside Contents/Resources.
cp -R "$bin_dir/TorroMailApp_TorroMailApp.bundle" "$app/Contents/Resources/"

cp "$pkg/Icon/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"

# SwiftPM links the executable against @rpath/Sparkle.framework but does not
# copy binary targets into a hand-assembled .app. Embed the universal framework
# and give dyld the conventional app-bundle search path.
sparkle_framework_src="$pkg/.build/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64/Sparkle.framework"
if [[ ! -d "$sparkle_framework_src" ]]; then
    echo "error: Sparkle.framework not found at $sparkle_framework_src" >&2
    echo "       run 'swift package --package-path $pkg resolve' first" >&2
    exit 1
fi
echo "==> Embedding Sparkle.framework"
mkdir -p "$app/Contents/Frameworks"
cp -R "$sparkle_framework_src" "$app/Contents/Frameworks/"
if ! otool -l "$app/Contents/MacOS/TorroMail" | grep -Fq '@executable_path/../Frameworks'; then
    install_name_tool -add_rpath "@executable_path/../Frameworks" "$app/Contents/MacOS/TorroMail"
fi

# --- Info.plist with injected version ----------------------------------------

cp "$info_plist_src" "$app/Contents/Info.plist"
# Strip any `git describe` suffix (e.g. 0.1.0-4-g1a06bd2[-dirty]) from the
# machine-comparable CFBundleVersion; keep the descriptive string for display.
BUNDLE_VERSION="$(printf '%s' "$VERSION" | sed -E 's/-[0-9]+-g[0-9a-f]+(-dirty)?$//; s/-dirty$//')"
/usr/libexec/PlistBuddy \
    -c "Set :CFBundleShortVersionString $VERSION" \
    -c "Set :CFBundleVersion $BUNDLE_VERSION" \
    "$app/Contents/Info.plist"

# Every shipped updater needs its own Ed25519 public key. Local ad-hoc release
# builds may omit it and remain runnable, but CI refuses to publish such a
# bundle. Keeping the public key in a secret lets the one-time key ceremony stay
# outside the repository alongside its private half.
if [[ -n "${SPARKLE_ED_PUBLIC_KEY:-}" ]]; then
    /usr/libexec/PlistBuddy \
        -c "Set :SUPublicEDKey $SPARKLE_ED_PUBLIC_KEY" \
        "$app/Contents/Info.plist"
else
    /usr/libexec/PlistBuddy \
        -c "Add :TorroMailDisableUpdates bool true" \
        "$app/Contents/Info.plist"
    echo "==> note: Sparkle disabled (SPARKLE_ED_PUBLIC_KEY is unset)"
fi

# --- Sign ---------------------------------------------------------------------
# The MCP server is a second Mach-O in Contents/MacOS; `codesign --deep` does
# not reliably treat it as nested code, so sign it explicitly before the bundle.

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    echo "==> Signing with \"$MACOS_SIGN_IDENTITY\" (hardened runtime)"
    codesign --force --timestamp --options=runtime \
        --sign "$MACOS_SIGN_IDENTITY" \
        "$app/Contents/MacOS/torromail-mcp"
    codesign --force --deep --timestamp --options=runtime \
        --entitlements "$entitlements" \
        --sign "$MACOS_SIGN_IDENTITY" \
        "$app"
else
    echo "==> Ad-hoc signing (MACOS_SIGN_IDENTITY unset)"
    codesign --force --sign - "$app/Contents/MacOS/torromail-mcp"
    codesign --force --deep --sign - \
        --entitlements "$entitlements" \
        "$app"
fi

codesign --verify --deep --strict --verbose=2 "$app"

echo "==> Done: $app"
