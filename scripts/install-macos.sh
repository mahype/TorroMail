#!/usr/bin/env bash
# Builds the TorroMail terminal programs on a Mac and installs them for the
# current user:
#
#   torromail      the terminal control surface
#   torromail-mcp  the MCP server assistants spawn
#
# Both land in $PREFIX/bin (default ~/.local), side by side — the surface finds
# the server next to itself, and writes that absolute path into an assistant's
# configuration when you connect one. No root needed.
#
#   scripts/install-macos.sh              build and install
#   scripts/install-macos.sh --uninstall  remove the two programs again
#   PREFIX=/usr/local scripts/install-macos.sh
#
# The copies are signed with your Apple Development or Developer ID identity
# (or CODESIGN_IDENTITY). That is what lets them share keychain items with the
# TorroMail app without a dialog: the items are scoped to the signing team.
# Unsigned, they could read none of the app's passwords.
#
# Your accounts, the policy document and the logs live in
# ~/Library/Application Support/TorroMail and are never touched here.
set -euo pipefail

prefix="${PREFIX:-$HOME/.local}"
bin="$prefix/bin"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$bin/torromail" "$bin/torromail-mcp"
    echo "Removed torromail and torromail-mcp from $bin."
    echo "Your data in ~/Library/Application Support/TorroMail was left alone."
    echo "Assistants you connected from the terminal still name the removed server"
    echo "in their configuration — reconnect them from the TorroMail app, or remove"
    echo "the \"torromail\" entry from their MCP settings by hand."
    exit 0
fi

if ! command -v cargo >/dev/null; then
    echo "cargo was not found. Install Rust first: https://rustup.rs" >&2
    exit 1
fi

identity="${CODESIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Developer ID Application|Apple Development/ { print $2; exit }')}"

echo "Building (release)…"
cargo build --release --manifest-path "$root/Cargo.toml" -p torromail-mcp -p torromail-tui

mkdir -p "$bin"
for program in torromail torromail-mcp; do
    install -m755 "$root/target/release/$program" "$bin/$program"
    if [[ -n "$identity" ]]; then
        codesign --force --sign "$identity" "$bin/$program"
    fi
done
if [[ -n "$identity" ]]; then
    echo "Signed: $identity"
else
    echo "warning: no codesigning identity found, so the programs are unsigned." >&2
    echo "         They cannot read passwords the TorroMail app stored, and the app" >&2
    echo "         cannot read theirs. Set CODESIGN_IDENTITY, or use a release build." >&2
fi

echo
echo "Installed $("$bin/torromail" --version) to $bin."
case ":$PATH:" in
    *":$bin:"*) echo "Start it with: torromail" ;;
    *) echo "$bin is not on your PATH — start it with: $bin/torromail" ;;
esac
