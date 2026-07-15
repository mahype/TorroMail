# Agent Guide

This file is the first stop for agents working in this repository.

## Product Intent

TorroMail is a local mail access layer for agents. It retrieves mail from
configured accounts and exposes safe, policy-controlled capabilities through an
MCP server.

The project is open-source and open-interface-first. The MCP contract, local
permission model, and documentation are the primary product surface.

## Important Boundary

TorroMail is not a general-purpose mail client.

Do not build:

- An inbox UI for humans to browse mail.
- A message reader or thread browser as a daily-use mail surface.
- Manual mail triage workflows.
- A full compose-and-send user workflow.
- General mail-client features that compete with Apple Mail, Spark, Mimestream,
  Airmail, or similar apps.

The native macOS app is allowed to exist, but its role is setup and control:

- Account setup.
- OAuth and credentials flow.
- Permission and policy review.
- Cache and index settings.
- MCP server lifecycle and client setup.
- Pending action approval.
- Audit log and diagnostics.

If UI work is requested, keep it clear, minimal, and operational. It should feel
like a precise control surface, not like a mail inbox.

## UI Principles

- Decisions, not options: every visible control is either a user decision or a
  required action. If the user cannot decide or act on it, it does not belong
  in the UI.
- Status only on exception: healthy state stays quiet (at most a colored dot);
  errors and pending approvals surface where they occur.
- Technical details (transport, executable paths, internal queue or module
  state) belong in logs, never in the UI. There is no Diagnostics screen.
- The sidebar stays minimal: accounts (name + email only), Settings, Log.
- User-facing strings are localized. English is the development language;
  German lives in `apps/TorroMailApp/Sources/TorroMailApp/Resources/de.lproj`.

## Design Guidelines

Brand and design assets live in <https://github.com/mahype/torro-design>
(private repo — use authenticated `gh` access). Key facts:

- Core color: Torro Rot `#D50C0C` (dark variant `#A50A0A` for gradients and
  hover, black `#0E0E0F`, silver `#C4C3C3`, white).
- House font: Frutiger LT (95 UltraBlack for logo/headlines, 65 Bold for
  subheads, 55 Roman for body). Secondary print font: Minion Pro.
- Signet: two inward-facing horns; mascot: the fighting bull.
- Machine-readable tokens: `tokens/tokens.json` and `tokens/colors.css`;
  interactive guide with do/don'ts: `design-guide.html`.

Follow these for brand moments (icon, logo, accent color); native macOS
semantic colors and system typography remain the default for standard
controls per the UI principles above.

In this repository: the app icon (horns square per the guide) lives in
`apps/TorroMailApp/Icon/` (`AppIcon.svg` source, generated `AppIcon.icns`),
the accent color is `Color.torroRed` in the app target, and
`scripts/make-app-bundle.sh` builds a runnable dev bundle including icon and
localization.

## Architecture

- `crates/torromail-core`: portable Rust domain model for accounts, policies,
  cache defaults, pending actions, and search result sets.
- `crates/torromail-mcp`: explicit MCP tool catalog and stdio line server
  facade.
- `apps/TorroMailApp`: SwiftUI macOS configuration/control app.
- `docs`: architecture notes, product decisions, and implementation plans.

Core rule: account setup, secret changes, OAuth setup, and permission edits stay
outside MCP tools. MCP clients can search and read within policy, prepare risky
actions, and inspect read-only admin state.

## Current MCP Tool Surface

Read/search tools:

- `mail_search`
- `mail_refine_search`
- `mail_get_message`
- `mail_get_thread`
- `mail_list_mailboxes`

Direct write tools (no approval, gated by the mark permission):

- `mail_mark`

Prepared action tools:

- `mail_create_draft`
- `mail_prepare_send`
- `mail_prepare_move`
- `mail_prepare_delete`
- `mail_confirm_action`

Read-only admin tools:

- `mail_list_accounts`
- `mail_get_policy`
- `mail_get_cache_status`

## Verification

Run the relevant checks before claiming work is complete:

```sh
cargo test
cargo build -p torromail-mcp
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

For documentation-only changes, at minimum read the changed Markdown files and
check that they do not contradict this product boundary.

## Historical Notes

Older specs may discuss an account-centered SwiftUI interface. Treat those as
historical implementation notes unless they explicitly say they are current. The
current product direction is MCP-first mail access, with the native app limited
to setup, consent, status, and diagnostics.
