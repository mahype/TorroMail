# Releasing

This document is for maintainers. It describes how TorroMail is versioned,
built, signed, notarized, and published as a downloadable `.dmg`.

## TL;DR

```bash
# 1. Bump the version in Cargo.toml ([workspace.package] version = "…")
# 2. Commit, then tag from that commit and push
git commit -am "release: v0.2.0"
git tag v0.2.0
git push origin main v0.2.0
# 3. Watch the Release workflow on GitHub Actions — it builds, signs, notarizes,
#    and attaches the DMG to a new GitHub Release.
```

## Versioning

TorroMail follows **SemVer**: `MAJOR.MINOR.PATCH`, optionally `-rc.N` for
pre-releases.

The version lives in one place: `version` under `[workspace.package]` in the
root [Cargo.toml](../Cargo.toml). All crates inherit it via
`version.workspace = true`. The release workflow refuses to run if the pushed
tag does not match this value.

The build script injects the version into the app bundle's `Info.plist`
(`CFBundleShortVersionString` / `CFBundleVersion`) at build time.

## Release tag format

- Stable: `v0.2.0`, `v0.2.1`, `v1.0.0`
- Release candidate: `v0.2.0-rc.1`
- Any tag matching `v*` triggers [.github/workflows/release.yml](../.github/workflows/release.yml).

## What the release workflow does

On push of a `v*` tag, the GitHub Actions release workflow (`macos-14` runner):

1. Verifies the tag matches the `Cargo.toml` workspace version.
2. Builds `torromail-mcp` for `aarch64` and `x86_64` and lipos them universal.
3. Builds a universal Swift executable and assembles `dist/TorroMail.app`
   (bundled MCP server, icon, localization, version-stamped `Info.plist`).
4. Signs the bundle with the **Developer ID Application** certificate under the
   hardened runtime, notarizes it with Apple, and staples the ticket.
5. Packages a drag-to-Applications `.dmg`, signs + notarizes + staples it, and
   writes `SHA256SUMS.txt`.
6. Mounts the DMG and smoke-tests codesign / Gatekeeper / stapled ticket.
7. Signs the DMG with TorroMail's Sparkle Ed25519 key and creates
   `appcast.xml` with the generated release notes.
8. Publishes one GitHub Release containing the DMG, checksum, and appcast.
   Installed apps poll the stable
   `releases/latest/download/appcast.xml` address, so the feed and the file it
   advertises become visible together.

## Required GitHub Actions secrets

Set these in the repository settings (**Settings → Secrets and variables →
Actions**). Without them the workflow fails at the signing step.

| Secret | What it is |
| --- | --- |
| `MACOS_CERTIFICATE_P12` | Base64 of your exported **Developer ID Application** `.p12` (`base64 -i cert.p12 \| pbcopy`). |
| `MACOS_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12`. |
| `APPLE_ID` | Apple ID email tied to your developer account. |
| `APPLE_TEAM_ID` | 10-character Apple Developer Team ID. |
| `APPLE_APP_SPECIFIC_PASSWORD` | App-specific password from appleid.apple.com → App-Specific Passwords (used for notarytool). |
| `SPARKLE_ED_PUBLIC_KEY` | Public Ed25519 key printed by Sparkle's `generate_keys`; injected into the release app's `Info.plist`. |
| `SPARKLE_ED_PRIVATE_KEY` | Private Ed25519 key exported once with `generate_keys -x`; signs each update and must never be committed. |

Exporting the certificate: in **Keychain Access**, find *Developer ID
Application: … (TEAMID)*, right-click → Export, choose Personal Information
Exchange (`.p12`), set a password → that becomes `MACOS_CERTIFICATE_PASSWORD`.

### One-time Sparkle key setup

Use a TorroMail-specific key pair. After resolving the Swift package, Sparkle's
key tool is available inside the package artifact:

```bash
swift package --package-path apps/TorroMailApp resolve
GEN="$(find apps/TorroMailApp/.build -type f -name generate_keys | head -1)"
"$GEN" --account torromail
```

The command stores the private key in the macOS keychain and prints the public
key. Add that printed value as `SPARKLE_ED_PUBLIC_KEY` in the TorroMail GitHub
repository (`gh secret set SPARKLE_ED_PUBLIC_KEY`), then export the private key
directly into its matching secret without placing it in the repository:

```bash
private_key="$(mktemp)"
rm -f "$private_key"
"$GEN" --account torromail -x "$private_key"
gh secret set SPARKLE_ED_PRIVATE_KEY < "$private_key"
rm -P "$private_key"
```

Back the private key up in the maintainer's password manager. If it is lost,
Sparkle can rotate the Ed25519 key through a DMG signed with the same Apple
Developer ID certificate, but that recovery is more involved than restoring a
backup. The public and private keys are separate from the Apple Developer ID
certificate; both signature layers are required.

## Building locally

The scripts work on a developer machine too. Full Xcode is required for a
universal binary; with only the Command Line Tools they build host-arch only.

```bash
# Unsigned / ad-hoc — runs on this machine, not distributable
./scripts/build-macos-app.sh
./scripts/build-dmg.sh            # needs: brew install create-dmg

# Signed + notarized (needs the same values as the CI secrets, in your env)
export MACOS_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_TEAM_ID="XXXXXXXXXX"
export APPLE_APP_SPECIFIC_PASSWORD="abcd-efgh-ijkl-mnop"
export SPARKLE_ED_PUBLIC_KEY="base64-public-key"
./scripts/build-macos-app.sh
./scripts/codesign-macos.sh
./scripts/build-dmg.sh
./scripts/smoke-test-dmg.sh dist/TorroMail-*.dmg
```

Local builds omit and disable the updater when `SPARKLE_ED_PUBLIC_KEY` is not
set. CI passes `REQUIRE_SPARKLE_KEY=1`, so a release cannot silently ship in
that state.

For the fast day-to-day dev bundle (debug, host-arch, ad-hoc), use
[scripts/make-app-bundle.sh](../scripts/make-app-bundle.sh) instead.
