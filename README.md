# DonnyMail

DonnyMail is a local mail access layer for agents. It retrieves mail from configured
accounts and exposes mail capabilities through an MCP server. The project is
open-source and open-interface-first: the MCP contract and local safety model are
the primary product surface.

DonnyMail is not a general-purpose mail client. It does not provide an inbox UI
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

- `crates/donnymail-core`: portable Rust domain model for accounts, policies, cache defaults, pending actions, and reusable search result sets.
- `crates/donnymail-mcp`: explicit MCP tool surface and stdio line server facade.
- `apps/DonnyMailApp`: macOS SwiftUI configuration/control app. This is not a mail client UI.
- `docs`: architecture notes and product decisions.

## Agent Orientation

Start with `AGENTS.md` before changing the project. It summarizes the current
product intent, boundaries, architecture, and verification commands for agentic
workers.

## Verify

```sh
cargo test
cargo build -p donnymail-mcp
swift run --package-path apps/DonnyMailApp --scratch-path apps/DonnyMailApp/.build DonnyMailKitContract
swift build --package-path apps/DonnyMailApp --scratch-path apps/DonnyMailApp/.build
```

In development, build `donnymail-mcp` before launching the SwiftUI app if you want the app supervisor to start the MCP process automatically from `target/debug/donnymail-mcp`.
