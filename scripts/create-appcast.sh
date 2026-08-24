#!/usr/bin/env bash
# Signs one TorroMail DMG with Sparkle's Ed25519 key and writes the appcast
# published beside it as a GitHub Release asset.
#
# Usage:
#   create-appcast.sh <dmg> <version> <release-notes-url> <output> [notes.md]
#
# Required env:
#   SPARKLE_ED_PRIVATE_KEY   private key exported by Sparkle generate_keys -x
# Optional env:
#   SIGN_UPDATE             explicit path to Sparkle's sign_update tool

set -euo pipefail

dmg_path="${1:?DMG path required}"
version="${2:?version required}"
release_notes_url="${3:?release notes URL required}"
output_path="${4:?output appcast path required}"
release_notes_file="${5:-}"
: "${SPARKLE_ED_PRIVATE_KEY:?SPARKLE_ED_PRIVATE_KEY must be set}"

if [[ ! -f "$dmg_path" ]]; then
    echo "error: DMG not found: $dmg_path" >&2
    exit 1
fi
if [[ -n "$release_notes_file" && ! -f "$release_notes_file" ]]; then
    echo "error: release notes not found: $release_notes_file" >&2
    exit 1
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -z "${SIGN_UPDATE:-}" ]]; then
    # Prune old_dsa_scripts: it holds a same-named legacy tool that emits a DSA
    # signature, which Sparkle 2 rejects. Only the Ed25519 tool may match.
    SIGN_UPDATE="$(find "$repo_root/apps/TorroMailApp/.build" \
        -path '*/old_dsa_scripts' -prune -o \
        -type f -name sign_update -print 2>/dev/null | head -1 || true)"
fi
if [[ ! -x "${SIGN_UPDATE:-}" ]]; then
    echo "error: Sparkle sign_update was not found under apps/TorroMailApp/.build" >&2
    exit 1
fi

key_file="$(mktemp)"
trap 'rm -f "$key_file"' EXIT
printf '%s' "$SPARKLE_ED_PRIVATE_KEY" > "$key_file"
chmod 600 "$key_file"

signature_line="$("$SIGN_UPDATE" -f "$key_file" "$dmg_path")"
ed_signature="$(printf '%s' "$signature_line" | sed -nE 's/.*sparkle:edSignature="([^"]+)".*/\1/p')"
dmg_length="$(printf '%s' "$signature_line" | sed -nE 's/.*length="([0-9]+)".*/\1/p')"
if [[ -z "$ed_signature" || -z "$dmg_length" ]]; then
    echo "error: could not parse sign_update output: $signature_line" >&2
    exit 1
fi

release_notes_md=""
if [[ -n "$release_notes_file" ]]; then
    release_notes_md="$(cat "$release_notes_file")"
fi

dmg_name="$(basename "$dmg_path")"
pub_date="$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')"
OUTPUT_PATH="$output_path" \
VERSION="$version" \
RELEASE_NOTES_URL="$release_notes_url" \
RELEASE_NOTES_MD="$release_notes_md" \
DMG_URL="https://github.com/mahype/TorroMail/releases/download/v${version}/${dmg_name}" \
DMG_LENGTH="$dmg_length" \
DMG_ED_SIGNATURE="$ed_signature" \
PUB_DATE="$pub_date" \
python3 "$repo_root/scripts/_appcast_create.py"

echo "Appcast: $output_path"
