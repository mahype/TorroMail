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
7. Publishes a GitHub Release with auto-generated notes and attaches the DMG
   and checksum file.

TorroMail has no in-app auto-update (no Sparkle/appcast): distribution is the
notarized DMG on the GitHub Releases page.

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

Exporting the certificate: in **Keychain Access**, find *Developer ID
Application: … (TEAMID)*, right-click → Export, choose Personal Information
Exchange (`.p12`), set a password → that becomes `MACOS_CERTIFICATE_PASSWORD`.

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
./scripts/build-macos-app.sh
./scripts/codesign-macos.sh
./scripts/build-dmg.sh
./scripts/smoke-test-dmg.sh dist/TorroMail-*.dmg
```

For the fast day-to-day dev bundle (debug, host-arch, ad-hoc), use
[scripts/make-app-bundle.sh](../scripts/make-app-bundle.sh) instead.
