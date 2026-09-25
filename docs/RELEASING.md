# Releasing

This document is for maintainers. It describes how TorroMail is versioned,
built, signed, notarized, and published: the Mac app as a downloadable `.dmg`,
the terminal programs for macOS (a signed, notarized universal tarball), Linux
(tarballs and the `torromail-bin` AUR package) and Windows (a zip) — all in
one GitHub Release per version.

## TL;DR

```bash
# 1. Bump the version in Cargo.toml ([workspace.package] version = "…")
# 2. Commit, then tag from that commit and push
git commit -am "release: v0.2.0"
git tag v0.2.0
git push origin main v0.2.0
# 3. Watch the Release workflow on GitHub Actions — it builds every platform,
#    assembles the release as a draft and publishes it once nothing is missing.
```

Unsure about a build? Tag a release candidate first (`v0.2.0-rc.1`, same
`Cargo.toml` version): it is built and signed exactly the same but published as
a pre-release, which no installed app is ever offered.

## Versioning

TorroMail follows **SemVer**: `MAJOR.MINOR.PATCH`, optionally `-rc.N` for
pre-releases.

The version lives in one place: `version` under `[workspace.package]` in the
root [Cargo.toml](../Cargo.toml). All crates inherit it via
`version.workspace = true`. The release workflow refuses to run if the pushed
tag does not match this value (a `-rc.N` suffix aside).

The build script injects the version into the app bundle's `Info.plist`
(`CFBundleShortVersionString` / `CFBundleVersion`) at build time.

## Release tag format

- Stable: `v0.2.0`, `v0.2.1`, `v1.0.0`
- Release candidate: `v0.2.0-rc.1`
- Any tag matching `v*` triggers [.github/workflows/release.yml](../.github/workflows/release.yml).

## What the release workflow does

On push of a `v*` tag, [release.yml](../.github/workflows/release.yml) first
verifies that the tag matches the `Cargo.toml` workspace version, then runs two
halves side by side.

**macOS** (`macos-14` runner):

1. Builds `torromail-mcp` for `aarch64` and `x86_64` and lipos them universal.
2. Builds a universal Swift executable and assembles `dist/TorroMail.app`
   (bundled MCP server, icon, localization, version-stamped `Info.plist`).
3. Signs the bundle with the **Developer ID Application** certificate under the
   hardened runtime, notarizes it with Apple, and staples the ticket.
4. Packages a drag-to-Applications `.dmg`, signs + notarizes + staples it, and
   writes `SHA256SUMS.txt`.
5. Mounts the DMG and smoke-tests codesign / Gatekeeper / stapled ticket.
6. Signs the DMG with TorroMail's Sparkle Ed25519 key and creates
   `appcast.xml` with the generated release notes.
7. Builds the terminal programs `torromail` and `torromail-mcp` universal,
   signs them with the same Developer ID under the hardened runtime, has them
   notarized and packs `torromail-<version>-universal-macos.tar.gz`
   ([scripts/build-cli-macos.sh](../scripts/build-cli-macos.sh)). The shared
   Team ID is what lets them and the app read one another's keychain items;
   the script refuses a signature without one. A bare executable cannot carry
   a stapled ticket, so Gatekeeper checks the notarization online on first
   start.

**Linux** ([build-linux.yml](../.github/workflows/build-linux.yml)):

1. Builds `torromail` and `torromail-mcp` for `x86_64` and `aarch64` on the
   oldest runner image on offer (glibc 2.35) and packs a tarball each.
2. Fills in the [PKGBUILD](../packaging/aur/torromail-bin/PKGBUILD) — version,
   release tag, checksums — then builds, installs and runs the package in a
   clean Arch container. `PKGBUILD` and `.SRCINFO` become
   `torromail-bin-aur.tar.gz`; its contents are what is pushed to the AUR.

**Windows** (`windows-latest` runner): builds `torromail.exe` and
`torromail-mcp.exe` for `x86_64` and packs them into a zip. The programs are
not code-signed, so SmartScreen warns on first start; the platform counts as
experimental until it has been used on a real machine — CI builds it and runs
the test suite there, no more.

**Publish**, only once every part succeeded: the release is created as a
**draft** with every file attached, and then published. Installed Mac apps poll
`releases/latest/download/appcast.xml`, and a draft is nobody's "latest" — so
the feed, the file it advertises and the Linux downloads become visible
together, and a build that failed halfway ships nothing. A `-rc.N` tag is
published as a pre-release and never becomes "latest".

### Linux-only releases

A fix that does not concern the Mac app can go out alone: a tag of the form
`linux-v<version>-<build>` (`linux-v0.10.0-2`) runs
[release-linux.yml](../.github/workflows/release-linux.yml), which builds the
same Linux half and publishes it with `--latest=false`, so the Mac updater
never looks at it. Started by hand, or by a pull request that touches the
packaging, that workflow is a dry run and publishes nothing.

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
