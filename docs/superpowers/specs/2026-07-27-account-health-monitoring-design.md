# Account Health Monitoring

Status: current. Written 2026-07-27.

## The problem

An account's dot is green because the account worked once, during setup. It
never turns red on its own.

`MailAccount` persists its connection state as a single boolean `isVerified`,
and `restoredState` turns that back into `.connected` on every launch. Nothing
re-verifies it: `AccountCheck.run` — which spawns the MCP binary's
`--check-account` and is the only code that touches a real mailbox on the app's
behalf — runs only when the user presses the test button. Change the mailbox
password at the provider, and TorroMail keeps claiming everything is fine until
an assistant fails and the user goes looking.

The MCP server already knows better. When a tool call fails on authentication
it writes `"result":"error"` into `audit.jsonl` and moves on; the app reads that
file for the activity view but derives no account state from it.

## What this adds

Continuous, honest per-account health: checked periodically, checked when the
server starts, updated for free whenever a real tool call succeeds or fails,
and surfaced as a desktop notification the moment an account breaks.

Out of scope: fetching mail, warming the cache, or any local copy of messages.
The check proves login and lists mailboxes — the same thing `--check-account`
does today. TorroMail is not a mail client, and a health check must not become
a sync loop. The menu bar icon does not change; that is separate work.

## Architecture

### `health.jsonl` is the source of truth

A sibling of `audit.jsonl` and `connections.jsonl` in
`~/Library/Application Support/TorroMail/`. Append-only JSON Lines, watched by
the app the way `AuditWatcher` watches the audit log — polling size and
modification date, since the file may not exist yet.

Unlike `audit.jsonl`, which the app only ever reads, this file has two writers:
the server and the app. Both append single `\n`-terminated lines with one
`write` on a file opened for append, which is atomic enough for lines this
short; both treat failure to write as nothing more than a missed sample.

One record per check, using the same epoch `ts` field as its two siblings
rather than a second time format:

```json
{"ts":1785319503.0,"account":"work","outcome":"ok","source":"periodic","detail":"login ok, 12 mailboxes visible"}
```

| Field | Values |
| --- | --- |
| `ts` | seconds since the Unix epoch, as in `audit.jsonl` |
| `account` | account id as it appears in the policy document |
| `outcome` | `ok`, `rejected`, `unreachable` |
| `source` | `periodic`, `server-start`, `tool-call`, `manual` |
| `detail` | human-readable; the message shown in the UI and the notification |

A line that does not decode is skipped rather than taken as fatal, exactly as
`AuditLog.load` skips a half-written trailing line. A record whose `outcome` is
absent or unrecognised is skipped the same way; an unrecognised `source` is
kept, since nothing derives from it.

Nothing trims the file — neither `audit.jsonl` nor `connections.jsonl` is
trimmed today, and a health record is smaller than an audit line.

Readers retain the last 20 records **per account**, not the last N lines of the
file. The distinction is not academic: with a global tail, one long assistant
session against a busy account crowds every other account's records off the
end, and an account with no records falls back to the stored `isVerified` bit —
which is the stale-green bug this document exists to remove, reappearing only
under load.

Write volume is held down at the source instead of by a reading trick. A
success is not recorded if the account's newest record is a success less than a
minute old: health is a status, and `audit.jsonl` already holds the transcript
of every call. Failures are never throttled — the first one is the entire point,
and it must reach the app on the call it happened.

`isVerified` in the stored app state stays on disk and keeps its meaning — the
state an account had when the app last quit — but it no longer drives the dot
once `health.jsonl` has a record for that account. It is the value shown for
the seconds between launch and the first check.

### Classification happens in Rust, at one place

Today every failure is `CoreError::ProviderFailure(String)`, so telling a wrong
password from a dead network would mean matching on message text. Instead:

- Add `CoreError::CredentialRejected(String)`.
- Return it from `ImapClient::authenticate` — both the `LOGIN` arm and the
  rejection path in `authenticate_xoauth2`.
- Everything before authentication — TCP connect, TLS setup, the IMAP greeting,
  `STARTTLS` — keeps returning `ProviderFailure`.

Reaching the login command is necessary but not sufficient. A server that
refuses one is not necessarily saying the password is wrong, and treating every
refusal as though it were would fire a first-strike red dot and a notification
at a TLS misconfiguration or a rate limit. The refusal's RFC 5530 response code
decides:

| Answer | Reading |
| --- | --- |
| `NO [AUTHENTICATIONFAILED]`, `[AUTHORIZATIONFAILED]`, `[EXPIRED]` | `CredentialRejected` |
| `NO` with no response code | `CredentialRejected` — the common shape, and no server says it about anything else |
| `NO [UNAVAILABLE]`, `[PRIVACYREQUIRED]`, `[SERVERBUG]`, `[CONTACTADMIN]` | `ProviderFailure` — temporary, or a wrong transport setting |
| any tagged `BAD` | `ProviderFailure` — a syntax or protocol fault, never a password |

`PRIVACYREQUIRED` matters more here than its rarity suggests: this crate lets
`ConnectionSecurity` be chosen independently of the port, so a wrongly
configured account produces exactly that refusal, and blaming the password for
it would send the user to fix the one thing that is not broken.

The refusal travels as a small struct — kind, response code, text — rather than
a string, so this table lives in one function instead of being restated at each
throw site. Two details ride along with it: any untagged `* NO [ALERT] …` line
preceding the refusal is carried into the message, because that is where Gmail
and Yahoo put the sentence the user actually needs ("Application-specific
password required: …"); and on the login path the secret is redacted from the
text before it goes anywhere, since a server that echoes the offending command
in a `BAD` would otherwise write the password into `health.jsonl`.

`check_account` maps its result accordingly, and reports the outcome rather
than a bare `Result<String, String>`:

```rust
pub enum HealthOutcome { Ok, Rejected, Unreachable }
```

The `--check-account` flag keeps its exit-status contract (0 = ok) so nothing
that shells out to it breaks. On failure its stderr becomes two parts: the
first line is the outcome word alone (`rejected` or `unreachable`), everything
after it is the detail. `AccountCheck.run` reads the first line to classify and
the remainder as the message — today it takes the whole of stderr as the
message, so that split is the one change on the Swift side.

### Four writers

| Trigger | Who | Notes |
| --- | --- | --- |
| Every 15 minutes | App | `AccountHealthMonitor`, a `DispatchSourceTimer` on a utility queue, modelled on `AuditWatcher`. Accounts are checked serially, not in parallel — a handful of logins should not arrive as a burst. |
| App launch, and wake from sleep | App | Same monitor, immediate run. Wake is `NSWorkspace.shared.notificationCenter` observing `didWakeNotification`. |
| MCP server start | Server | One check per configured account as the server comes up, **debounced**: an account with a `health.jsonl` record younger than 5 minutes is skipped. Without this, five paired clients each spawning a server means five logins per account per launch. |
| Any tool call | Server | `record_health`, a sibling of `record_audit`. `CredentialRejected` → `rejected`; a provider failure while connecting → `unreachable`; a successful call → `ok`. Costs no extra login. |

The interval is fixed at 15 minutes. It is not a setting: the user cannot make
a better decision about it than we can, and AGENTS.md says a control that is
not a decision does not belong in the UI.

`AccountCheck.run` currently waits on the subprocess forever. It gets a
30-second timeout; passing it terminates the process and counts as
`unreachable`.

### Deriving the dot

A pure function over the records for one account, newest last. No counter is
persisted anywhere — the file is the state:

- Last record `ok` → `.connected`.
- Last record `rejected` → `.failed(detail)`, immediately.
- Last record `unreachable` → the state stays whatever the last non-`unreachable`
  record said, until three consecutive `unreachable` records; then
  `.failed(detail)`.
- No records at all → the restored `isVerified` state, as today.

The three-in-a-row rule is what keeps a train ride or a flaky café network from
turning every account red. A rejected password is not transient and gets no
grace.

`ConnectionState` keeps its four cases. The grace rule lives in the derivation,
not in the UI, so no new visible state is introduced.

## User-facing behaviour

### Notifications

`UNUserNotificationCenter`, authorization requested once on the monitor's first
run. Notifications fire on transition only:

- `.connected` → `.failed`: title is the account name, body is the detail
  (`"Der Server hat die Zugangsdaten abgelehnt."`, `"Seit 45 Minuten nicht
  erreichbar."`). Activating it opens that account's detail view.
- `.failed` → `.connected`: one quiet all-clear.

A persistently broken account does not re-notify. It stays red in the app,
which is where a standing problem belongs.

Denied authorization degrades silently to in-app status only. The app does not
ask again and does not nag.

### In the app

- Dashboard: a section that exists only while at least one account is broken,
  listing those accounts with their reason; each row opens the account. Today
  `model.health == .needsAttention` never reaches the dashboard at all.
- Account detail: the failure reason in plain language next to the existing
  `ConnectionStatusBadge`, plus "last checked …" as relative time. The healthy
  case shows the timestamp only — no new prose while things are fine.
- Strings are added to `de.lproj/Localizable.strings` alongside their English
  defaults.

## Components

| Unit | Responsibility |
| --- | --- |
| `CoreError::CredentialRejected` (core) | Distinguishes refused credentials from an unreachable server |
| `HealthOutcome` + `check_account` (mcp) | Classifies one login attempt |
| `record_health` (mcp) | Appends a record for a tool call's outcome; best-effort, never fails a call |
| server-start sweep (mcp) | Debounced check of every configured account |
| `HealthLog` (TorroMailKit) | Reads and appends `health.jsonl`; owns the derivation function |
| `HealthLogWatcher` (TorroMailKit) | Fires when the file grows, like `AuditWatcher` |
| `AccountHealthMonitor` (TorroMailKit) | The timer, the wake observer, the serial check loop, the 30-second timeout |
| `HealthNotifier` (TorroMailApp) | Transition detection and `UNUserNotificationCenter` |

The derivation is deliberately separate from the monitor: it is a pure function
over records, testable without a timer, a subprocess, or a mailbox.

## Testing

Rust:

- A tagged `NO` to `LOGIN` produces `CredentialRejected`; a tagged `NO` in the
  XOAUTH2 continuation exchange does too.
- `NO [UNAVAILABLE]` and a tagged `BAD` produce `ProviderFailure` — the
  classification table above, tested rather than assumed.
- A transport that fails to connect produces `ProviderFailure`, not
  `CredentialRejected` — and so does a transport that dies *after* the login
  command was sent, which is the seam the split exists to hold.
- `record_health` writes the right outcome for a rejected call, a transport
  failure, and a success.
- The server-start sweep skips an account whose newest record is 4 minutes old
  and checks one whose newest is 6 minutes old.
- A malformed or unwritable `health.jsonl` does not fail a tool call.

Swift (`TorroMailKitContract`):

- Derivation: empty file falls back to the restored state; a trailing `ok` is
  green; a trailing `rejected` is red immediately; one and two `unreachable`
  records preserve the prior state; three turn it red; an `ok` in between resets
  the count.
- Records with an unknown `outcome` or `source` are ignored, not fatal.
- Transition detection fires once per crossing, not once per check.

## Verification

```sh
cargo test
cargo build -p torromail-mcp
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Manual: point an account at a wrong password, wait for one tick, confirm the
notification arrives once, the dashboard section appears, and correcting the
password clears both.
