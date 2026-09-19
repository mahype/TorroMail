# TorroMail

TorroMail is a local mail access layer for agents. It retrieves mail from configured
accounts and exposes mail capabilities through an MCP server. The project is
open-source and open-interface-first: the MCP contract and local safety model are
the primary product surface.

TorroMail is not a general-purpose mail client. It does not provide an inbox UI
for users to browse, read, or manage mail manually. Any native macOS app in this
repository is a configuration and control surface for account setup, permissions,
MCP lifecycle, pending action review, diagnostics, and local status.

The project deliberately keeps account setup, secrets, OAuth, and permission
changes out of MCP tools. MCP clients can search, read within policy, prepare
risky actions, and inspect read-only admin state. The native app owns
configuration and user confirmation.

## Product Boundaries

- Build the mail retrieval, policy, cache, and MCP interface first.
- Keep human-facing mail consumption out of scope: no inbox timeline, message
  reader, thread browser, or day-to-day mail workflow UI.
- Use the native app only for setup, consent, permissions, status, and
  diagnostics.
- Treat MCP tools as the supported interface for agents and integrations.
- Favor clear documentation and explicit contracts over hidden behavior.

## Layout

- `crates/torromail-core`: portable Rust domain model for accounts, policies, cache defaults, pending actions, and reusable search result sets.
- `crates/torromail-mcp`: explicit MCP tool surface and stdio line server facade.
- `crates/torromail-control`: the configuration model every surface shares — accounts and their state file, the policy document writer, MCP client setup, provider discovery, log readers, platform paths.
- `crates/torromail-tui`: terminal control surface (`torromail`), the Linux counterpart to the macOS app. Read-only for now. This is not a mail client UI either.
- `contracts`: behaviour pinned as shared cases. The Rust tests and the Swift contract suite run the same files, so the two implementations cannot drift apart silently.
- `apps/TorroMailApp`: macOS SwiftUI configuration/control app. This is not a mail client UI.
- `docs`: architecture notes and product decisions.

## Agent Orientation

Start with `AGENTS.md` before changing the project. It summarizes the current
product intent, boundaries, architecture, and verification commands for agentic
workers.

## Verify

```sh
cargo test
cargo build -p torromail-mcp
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

To try the terminal surface: `cargo run -p torromail-tui`. It reads the shared data directory —
`~/Library/Application Support/TorroMail` on macOS, `$XDG_STATE_HOME/torromail`
(default `~/.local/state/torromail`) elsewhere. Keys: `1`–`7` sections, arrows to select,
`tab` for account tabs, `q` to quit.

In development, build `torromail-mcp` before launching the SwiftUI app if you want the app supervisor to start the MCP process automatically from `target/debug/torromail-mcp`.

For a runnable dev bundle, use `scripts/make-app-bundle.sh`.

## Releases

Pushing a `v*` tag triggers [.github/workflows/release.yml](.github/workflows/release.yml),
which builds a universal `TorroMail.app`, signs and notarizes it, packages a
`.dmg`, signs a Sparkle appcast, and publishes all artifacts as one GitHub
Release. Installed copies can check that feed automatically or on demand. See
[docs/RELEASING.md](docs/RELEASING.md) for versioning rules, the required
signing secrets, and how to build a release locally.
