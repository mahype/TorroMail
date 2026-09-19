#!/usr/bin/env bash
# Builds TorroMail for Linux and installs it for the current user:
#
#   torromail      the terminal control surface
#   torromail-mcp  the MCP server assistants spawn
#
# Both land in $PREFIX/bin (default ~/.local), side by side — the surface finds
# the server next to itself, and writes that absolute path into an assistant's
# configuration when you connect one. No root needed.
#
#   scripts/install-linux.sh              build and install
#   scripts/install-linux.sh --uninstall  remove the two programs again
#   PREFIX=/usr/local scripts/install-linux.sh
#
# Your accounts, the policy document and the logs live in
# ${XDG_STATE_HOME:-~/.local/state}/torromail and are never touched here;
# passwords and keys live in your desktop's Secret Service.
set -euo pipefail

prefix="${PREFIX:-$HOME/.local}"
bin="$prefix/bin"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$bin/torromail" "$bin/torromail-mcp"
    echo "Removed torromail and torromail-mcp from $bin."
    echo "Your data in ${XDG_STATE_HOME:-$HOME/.local/state}/torromail was left alone."
    echo "Assistants you connected still name the removed server in their"
    echo "configuration — disconnect them in TorroMail before uninstalling, or"
    echo "remove the \"torromail\" entry from their MCP settings by hand."
    exit 0
fi

if ! command -v cargo >/dev/null; then
    echo "cargo was not found. Install Rust first: https://rustup.rs" >&2
    exit 1
fi
# Not fatal: the programs install fine without it, but no account can be
# added, because there would be nowhere to keep its password.
if ! command -v secret-tool >/dev/null; then
    echo "warning: secret-tool was not found. TorroMail keeps passwords in the" >&2
    echo "         desktop's Secret Service and needs it (Arch: pacman -S libsecret," >&2
    echo "         Debian/Ubuntu: apt install libsecret-tools), plus a running" >&2
    echo "         keyring such as gnome-keyring." >&2
fi

echo "Building (release)…"
cargo build --release --manifest-path "$root/Cargo.toml" -p torromail-mcp -p torromail-tui

install -Dm755 "$root/target/release/torromail" "$bin/torromail"
install -Dm755 "$root/target/release/torromail-mcp" "$bin/torromail-mcp"

echo
echo "Installed $("$bin/torromail" --version) to $bin."
case ":$PATH:" in
    *":$bin:"*) echo "Start it with: torromail" ;;
    *) echo "$bin is not on your PATH — start it with: $bin/torromail" ;;
esac
