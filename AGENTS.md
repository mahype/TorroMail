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

The catalog is the full planned surface; tools not yet implemented are in the
catalog on purpose and answer `-32000 tool not implemented yet` rather than
vanishing. Keep this list honest — a tool that looks available but is not is
worse than one that says so.

Read/search tools:

- `mail_search`
- `mail_get_message`
- `mail_list_mailboxes`
- `mail_refine_search`
- `mail_get_thread` — plain IMAP has no thread identity, so a thread is
  reconstructed from the `References`/`In-Reply-To` headers within one mailbox.

Direct write tools (no approval, gated by the mark permission):

- `mail_mark`

Prepared action tools:

- `mail_create_draft` — composes the message and appends it to the drafts
  folder with `\Draft`; gated by the drafts right. Optional attachments travel
  as standard padded base64 with a filename and optional media type; paths and
  URLs are never accepted. At most 20 attachments and 20 MiB of finished MIME
  data are allowed.
- `mail_prepare_move` — holds a move for confirmation; gated by the move right.
- `mail_prepare_delete` — holds a soft delete (move to Trash) or, with
  `permanent`, an expunge; gated by the matching right.
- `mail_prepare_send` — holds a send of a draft composed this session; gated
  by the send right. Submission goes over SMTP (implicit TLS on 465, STARTTLS
  otherwise); the account's SMTP facts travel in the policy document.
- `mail_confirm_action` — runs a prepared action once, matched by its code
  and inside its TTL, re-checking the policy at execution.

Prepared actions live in the server between prepare and confirm. The
confirmation code is the handshake tying a confirm to one preparation; the GUI
is the intended place for a human to read the preview and approve.

All fourteen catalog tools are implemented.

Read-only admin tools. These answer from the policy document alone and are
dispatched before any account lookup or connection — `mail_list_accounts` is
where an `account_id` is learned, so requiring one would lock every client out:

- `mail_list_accounts`
- `mail_get_policy`
- `mail_get_cache_status`

## Client Pairing

Every MCP client presents an access key via the `TORROMAIL_TOKEN` environment
variable in its own MCP config; the app writes it there on connect (or hands
it over in the manual snippet, masked on screen). The policy document carries
a `clients` allowlist of SHA-256 hashes — never a usable key; plaintext lives
only in the client's config and the app's keychain (`client-key-<id>` under
service `TorroMail`). The server checks the hash per tool call, so revoking in
the app bites running sessions. `initialize` and `tools/list` stay open; every
`tools/call` from an unpaired client answers `-32001` with instructions the
assistant can relay. A document *without* a `clients` key (pre-pairing or
hand-managed) enforces nothing; the app always publishes one. `--check-account`
sits behind the same gate — the app presents its own key (`torromail-app`).

Reading mail has no browsing tool by design. "What came in lately?" is
`mail_search` with an empty query: no filter, newest first, INBOX unless a
mailbox is named. Ordering follows IMAP UIDs — arrival at the server — not the
`Date` header, which is the sender's claim.

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
