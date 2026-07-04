# DonnyMail Account-Centered UI Design

Status: Superseded historical design.

This document predates the current product clarification. DonnyMail is an
MCP-first local mail access layer, not a general-purpose mail client. The native
macOS app remains limited to setup, consent, MCP lifecycle, status, audit, and
diagnostics. Do not use this document to justify adding an inbox, message
reader, thread browser, or daily-use mail UI.

## Decision

DonnyMail will use the account-centered layout from mockup option A. The primary entry point is the list of mail accounts. Selecting an account opens all account-specific configuration in one detail area. Global settings stay secondary and separate.

## Product Principle

The user should not need to understand MCP internals, config files, cache engines, or permission models before adding and using a mail account. The first visible workflow is:

1. Add or select an email account.
2. Confirm connection and authentication status.
3. Review simple recommended access.
4. Open deeper sections only when needed.

The interface should show safe defaults first and reveal advanced controls progressively.

## Main Navigation

The app shell uses a two-level structure:

- Account list: all configured email accounts plus an add-account action.
- Secondary global items: AI Clients, General Settings, Audit Log, Diagnostics.

The previous global split into Accounts, Permissions, Search & Cache, MCP Clients, Pending Actions, Audit Log, and Diagnostics is replaced. Permissions, search/cache, mailboxes, and pending actions become account detail sections.

## Account Detail Sections

Each account detail view contains these sections:

- Overview: connection summary, provider, authentication state, MCP availability, recent account-specific activity, and pending confirmations.
- Connection: provider, IMAP/SMTP or provider API status, OAuth/password method, autodiscovery, test connection, and Keychain state.
- Permissions: readable switches grouped by task: reading, attachments, drafts, sending, marking, moving, deleting, permanent deletion.
- Search & Cache: metadata/header cache, body cache, full-text index, attachment index, selected folders, storage size, last sync, rebuild/delete index actions.
- Mailboxes: included folders, excluded folders, sent/archive/trash mapping.
- Pending Actions: account-scoped confirmation queue for send/move/delete with preview and expiration.
- Advanced: provider-specific settings, debug identifiers, import/export for support, and future adapter options.

The top account page should not show every setting at once. It shows status summaries and a small set of next actions. Deeper panels carry detailed controls.

## Global Sections

Global sections are limited to settings that are not naturally owned by one mail account:

- AI Clients: configured MCP clients, client-specific authentication/approval variation, snippets or automatic installation, and the actual local process path.
- General Settings: whether DonnyMail launches the local MCP server with the app, startup behavior, local IPC transport, update policy, and app-wide privacy defaults.
- Audit Log: cross-account activity with filtering by account, client, tool, and result.
- Diagnostics: health checks, logs, export diagnostics, local daemon status, and adapter state.

## MCP Server Lifecycle

The local MCP server should start and stop with the DonnyMail app by default. Users should not need to run `cargo run -p donnymail-mcp` or any separate process in normal use.

Development and diagnostics may still expose CLI commands, but the documented user path is:

- Start DonnyMail.
- DonnyMail starts or supervises the local MCP server.
- The GUI shows whether the server is running and which executable is used.
- AI client setup points to the supervised local process.

## Progressive Disclosure

The visual hierarchy should be quiet and utility-focused:

- Account status and safe defaults are visible immediately.
- Risky abilities such as send, delete, permanent delete, body cache, attachment index, and full-text body index require explicit deeper interaction.
- Advanced provider and diagnostics controls are not shown on the first account overview.
- Confirmation dialogs show concrete previews instead of abstract permission names.

## Safety Rules

Account setup, secret changes, OAuth setup, permission edits, and cache/index enablement remain GUI-only. MCP tools can read within policy, search, prepare risky actions, and expose read-only admin state. MCP must not provide free-form tools to add accounts, change secrets, or grant itself rights.

## Implementation Scope

The next implementation step is to refactor the SwiftUI app structure. It should preserve the existing Rust core and MCP test contracts, then add SwiftUI tests or stable model tests when the app model is split into testable units.

The first implementation pass should not build real IMAP, SMTP, or OAuth adapters. It should implement the correct user-facing structure and include lightweight MCP process supervision so the app can start a local `donnymail-mcp` executable when available.

## Acceptance Criteria

- The first screen is an account list with a selected account detail view.
- Permissions, search/cache, mailboxes, and pending actions are reachable inside the selected account.
- AI client authentication variations are separate from account configuration.
- The UI communicates that the MCP server starts with DonnyMail in normal use.
- The interface avoids presenting all advanced settings at once.
- Existing Rust tests still pass.
- The SwiftUI package still builds.
