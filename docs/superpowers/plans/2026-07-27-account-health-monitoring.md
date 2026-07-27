# Account Health Monitoring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An account's status dot tells the truth continuously — refused credentials turn it red within minutes and raise a desktop notification, instead of staying green forever because setup once succeeded.

**Architecture:** A new append-only `health.jsonl` beside `audit.jsonl` becomes the single source of truth. The Rust side learns to distinguish refused credentials from an unreachable server (a new `CoreError::CredentialRejected`, thrown at exactly one place) and appends a record on every tool call and at server start. The app appends records from a 15-minute timer, watches the file, and derives each account's `ConnectionState` from the tail of the log with a three-strikes grace period for transport failures.

**Tech Stack:** Rust (workspace crates `torromail-core`, `torromail-mcp`; no async runtime, no external test framework — integration tests in `crates/*/tests/`), Swift 6 / SwiftUI (`apps/TorroMailApp`, targets `TorroMailKit` and `TorroMailApp`, contract test `TorroMailKitContract` is a plain executable asserting via `require(...)`).

**Spec:** `docs/superpowers/specs/2026-07-27-account-health-monitoring-design.md`

---

## File Structure

| File | Change | Responsibility |
| --- | --- | --- |
| `crates/torromail-core/src/lib.rs` | Modify | Add `CoreError::CredentialRejected` and its `Display` arm |
| `crates/torromail-core/src/imap_provider.rs` | Modify | Split `command` so a spoken refusal is distinguishable from a transport error; throw `CredentialRejected` from `authenticate` |
| `crates/torromail-core/tests/imap_contract.rs` | Modify | Classification tests against the existing scripted transport |
| `crates/torromail-mcp/src/health.rs` | Create | `HealthOutcome`, the record shape, appending, reading the tail, the debounce decision |
| `crates/torromail-mcp/src/lib.rs` | Modify | `health_path`, `ToolFailure::health_outcome`, recording from `run_with_connection`, `check_account` returning an outcome, the server-start sweep |
| `crates/torromail-mcp/src/main.rs` | Modify | `--check-account` stderr contract; run the sweep before the stdio loop |
| `crates/torromail-mcp/tests/tool_contract.rs` | Modify | Health records written from real tool-call outcomes |
| `apps/TorroMailApp/Sources/TorroMailKit/HealthLog.swift` | Create | `HealthOutcome`, `HealthRecord`, reading/appending `health.jsonl`, the pure derivation, and the pure transition rule |
| `apps/TorroMailApp/Sources/TorroMailKit/AccountHealthMonitor.swift` | Create | The 15-minute timer, the wake observer, the serial check loop |
| `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift` | Modify | `AccountCheck` gains an outcome and a timeout; `TorroMailModel.applyHealth` |
| `apps/TorroMailApp/Sources/TorroMailApp/HealthNotifier.swift` | Create | Delivery only: `UNUserNotificationCenter` plumbing and the one-time authorization ask |
| `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift` | Modify | Wire watcher, monitor and notifier; the dashboard section; "last checked" |
| `apps/TorroMailApp/Sources/TorroMailApp/Resources/de.lproj/Localizable.strings` | Modify | German for the new strings |
| `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift` | Modify | Derivation rules |

`HealthLog.swift` and `AccountHealthMonitor.swift` are new files rather than more of `TorroMailKit.swift`, which is already 2810 lines. The spec named a `HealthLogWatcher`; there is no such class in the end — `AuditWatcher` already takes a `url:` and `connectionWatcher` in `TorroMailApp.swift:455` already reuses it for `connections.jsonl`. A third instance pointed at `health.jsonl` is the DRY answer.

---

### Task 1: Tell a refused login from an unreachable server

Today every IMAP failure is `CoreError::ProviderFailure(String)`, so nothing downstream can tell a wrong password from a dead network without matching on message text. The refusal has to become its own error at the one place the server speaks it.

> **Amended during review.** The steps below classify *every* tagged non-OK answer to `LOGIN` as a credential rejection. That is too coarse: `NO [UNAVAILABLE]` is a temporary backend failure, `NO [PRIVACYREQUIRED]` is a wrong transport setting, and a tagged `BAD` is a protocol fault — none of them is a wrong password, and each would fire a first-strike red dot and a notification. The implemented rule keys on the RFC 5530 response code and carries the refusal as a struct rather than a string; see the classification table in the spec. The refusal message also carries any preceding untagged `* NO [ALERT] …` line, and the secret is redacted from it on the login path. Read the shipped `crates/torromail-core/src/imap_provider.rs` rather than the code blocks below.

**Files:**
- Modify: `crates/torromail-core/src/lib.rs:17-56`
- Modify: `crates/torromail-core/src/imap_provider.rs:239-291`, `crates/torromail-core/src/imap_provider.rs:531-557`
- Test: `crates/torromail-core/tests/imap_contract.rs`

- [ ] **Step 1: Write the failing tests**

Append to `crates/torromail-core/tests/imap_contract.rs`. The file already has `line`, `SentLog`, `ScriptedTransport` and `login_script` helpers — use them, do not write new ones. Add `CoreError` to the existing `use torromail_core::{…}` list at the top of the file.

```rust
#[test]
fn a_refused_password_is_a_credential_rejection_not_a_transport_failure() {
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO [AUTHENTICATIONFAILED] Invalid credentials"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "wrong",
    );

    match result {
        Ok(_) => panic!("login must fail"),
        Err(CoreError::CredentialRejected(message)) => {
            assert!(message.contains("Invalid credentials"), "got: {message}");
        }
        Err(other) => panic!("a refused login must be CredentialRejected, got: {other:?}"),
    }
}

#[test]
fn a_refused_token_is_a_credential_rejection_too() {
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("+ eyJzdGF0dXMiOiI0MDEifQ=="),
        line("t1 NO Invalid credentials (Failure)"),
    ];
    let result = ImapClient::connect_with(
        ScriptedTransport::new(script, SentLog::default()),
        "me@gmail.com",
        "expired",
        ImapAuth::XOAuth2,
    );

    assert!(
        matches!(result, Err(CoreError::CredentialRejected(_))),
        "an expired token is a credential problem, not a transport one"
    );
}

#[test]
fn a_broken_greeting_stays_a_provider_failure() {
    // Nothing was refused — we never got far enough to say a password. This
    // is the case the three-strikes grace period exists for.
    let script = vec![line("* BYE server too busy")];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    );

    assert!(
        matches!(result, Err(CoreError::ProviderFailure(_))),
        "a failure before authentication must not accuse the credentials"
    );
}

#[test]
fn a_refused_command_after_login_is_not_a_credential_problem() {
    let mut script = login_script();
    script.push(line("t2 NO LIST failed"));
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds")
    .list_mailboxes();

    assert!(
        matches!(result, Err(CoreError::ProviderFailure(_))),
        "only the login command can produce a credential rejection"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p torromail-core --test imap_contract
```

Expected: compilation error, `no variant or associated item named 'CredentialRejected' found for enum 'CoreError'`.

- [ ] **Step 3: Add the error variant**

In `crates/torromail-core/src/lib.rs`, add to the `CoreError` enum right after `ProviderFailure(String)`:

```rust
    /// The server spoke a refusal to the login command itself: a wrong
    /// password, an expired app password, a rejected OAuth token. Kept apart
    /// from `ProviderFailure` because the two need opposite handling — this
    /// one will not fix itself, and retrying it is pointless.
    CredentialRejected(String),
```

And the matching arm in `impl Display for CoreError`, after the `ProviderFailure` arm:

```rust
            Self::CredentialRejected(message) => {
                write!(f, "credentials rejected: {message}")
            }
```

- [ ] **Step 4: Split the command send from the refusal it may answer with**

In `crates/torromail-core/src/imap_provider.rs`, replace the whole existing `fn command` (currently at lines 531-557) with these two functions:

```rust
    /// Sends `command` and reads until its tagged answer. The two failure
    /// shapes stay apart on purpose: the outer `Err` is transport — the
    /// connection broke on the way — while the inner `Err` is the server
    /// speaking a tagged `NO` or `BAD`. Only a refusal the server actually
    /// spoke can mean the credentials are wrong.
    fn command_answering(
        &mut self,
        command: &str,
    ) -> CoreResult<Result<Vec<ResponseLine>, String>> {
        self.next_tag += 1;
        let tag = format!("t{}", self.next_tag);
        self.transport.send_line(&format!("{tag} {command}"))?;

        let mut lines = Vec::new();
        loop {
            let mut text = self.transport.read_line()?;
            let mut literals = Vec::new();
            while let Some(count) = trailing_literal_size(&text) {
                literals.push(self.transport.read_bytes(count)?);
                let continuation = self.transport.read_line()?;
                text.push(' ');
                text.push_str(&continuation);
            }

            if let Some(rest) = text.strip_prefix(&format!("{tag} ")) {
                if rest.starts_with("OK") {
                    return Ok(Ok(lines));
                }
                return Ok(Err(rest.to_owned()));
            }
            lines.push(ResponseLine { text, literals });
        }
    }

    fn command(&mut self, command: &str) -> CoreResult<Vec<ResponseLine>> {
        self.command_answering(command)?.map_err(|refusal| {
            CoreError::ProviderFailure(format!("IMAP command refused: {refusal}"))
        })
    }
```

- [ ] **Step 5: Throw the new error from the two authentication paths**

In the same file, replace the body of `fn authenticate` (currently at line 239):

```rust
    fn authenticate(&mut self, username: &str, secret: &str, auth: ImapAuth) -> CoreResult<()> {
        match auth {
            ImapAuth::Password => {
                self.command_answering(&format!(
                    "LOGIN {} {}",
                    imap_quoted(username),
                    imap_quoted(secret)
                ))?
                .map_err(|refusal| {
                    CoreError::CredentialRejected(format!("IMAP rejected the login: {refusal}"))
                })?;
            }
            ImapAuth::XOAuth2 => self.authenticate_xoauth2(username, secret)?,
        }
        Ok(())
    }
```

And in `fn authenticate_xoauth2`, change the rejection at line 276 from `CoreError::ProviderFailure` to:

```rust
                return Err(CoreError::CredentialRejected(format!(
                    "IMAP rejected the access token: {rest}"
                )));
```

- [ ] **Step 6: Run the whole core suite**

```bash
cargo test -p torromail-core
```

Expected: PASS, including the two pre-existing tests `a_refused_login_surfaces_the_server_answer` and `a_rejected_token_gets_the_empty_reply_the_server_waits_for` — both assert on `error.to_string()` containing the server's words, and both new error messages still carry them.

- [ ] **Step 7: Check nothing else matched on the enum exhaustively**

```bash
cargo build --workspace
```

Expected: builds clean. `ToolFailure::is_connection` in `crates/torromail-mcp/src/lib.rs:1460` matches `ProviderFailure` specifically and is **left alone deliberately** — it decides whether to drop the pooled session and retry once. A rejected credential must not trigger a retry: repeating a bad login gains nothing and walks toward a provider lockout. If the build flags any other non-exhaustive match, add a `CredentialRejected` arm that behaves like the `ProviderFailure` one.

- [ ] **Step 8: Commit**

```bash
git add crates/torromail-core/src/lib.rs crates/torromail-core/src/imap_provider.rs crates/torromail-core/tests/imap_contract.rs
git commit -m "feat(core): tell a refused login from an unreachable server"
```

---

### Task 2: The health record and its file

A small module owning the record shape, the append, and reading the tail. Everything else in the Rust side calls into this.

**Files:**
- Create: `crates/torromail-mcp/src/health.rs`
- Modify: `crates/torromail-mcp/src/lib.rs` (add `mod health;` and re-export)
- Test: `crates/torromail-mcp/tests/health_log.rs` (create)

- [ ] **Step 1: Write the failing test**

Create `crates/torromail-mcp/tests/health_log.rs`:

```rust
//! The health log: what gets written, what gets read back, and when a fresh
//! record makes a check unnecessary.

use std::time::{SystemTime, UNIX_EPOCH};

use torromail_mcp::health::{self, HealthOutcome};

fn temp_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("torromail-health-{name}.jsonl"));
    let _ = std::fs::remove_file(&path);
    path
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[test]
fn an_appended_record_reads_back_with_its_fields() {
    let path = temp_path("roundtrip");
    health::append(&path, "work", HealthOutcome::Rejected, "tool-call", "nope");

    let records = health::load(&path);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].account, "work");
    assert_eq!(records[0].outcome, HealthOutcome::Rejected);
    assert_eq!(records[0].source, "tool-call");
    assert_eq!(records[0].detail, "nope");
}

#[test]
fn a_missing_file_reads_as_no_records() {
    assert!(health::load(&temp_path("absent")).is_empty());
}

#[test]
fn a_broken_line_is_skipped_rather_than_taken_as_fatal() {
    let path = temp_path("broken");
    health::append(&path, "work", HealthOutcome::Ok, "periodic", "fine");
    std::fs::write(
        &path,
        format!("{}{{ not json\n", std::fs::read_to_string(&path).unwrap()),
    )
    .unwrap();

    let records = health::load(&path);
    assert_eq!(records.len(), 1, "the good line survives its broken neighbour");
}

#[test]
fn a_fresh_record_makes_a_check_unnecessary() {
    let path = temp_path("debounce");
    health::append(&path, "work", HealthOutcome::Ok, "server-start", "fine");

    let records = health::load(&path);
    assert!(
        !health::needs_check(&records, "work", now(), 300),
        "a record written seconds ago is fresh enough"
    );
    assert!(
        health::needs_check(&records, "other", now(), 300),
        "an account with no record at all is always due"
    );
}

#[test]
fn a_stale_record_makes_a_check_due_again() {
    let path = temp_path("stale");
    health::append(&path, "work", HealthOutcome::Ok, "server-start", "fine");
    let records = health::load(&path);

    assert!(
        health::needs_check(&records, "work", now() + 400, 300),
        "past the window the account is due again"
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo test -p torromail-mcp --test health_log
```

Expected: FAIL, `unresolved import 'torromail_mcp::health'`.

- [ ] **Step 3: Write the module**

Create `crates/torromail-mcp/src/health.rs`:

```rust
//! The account health log: `health.jsonl`, a sibling of `audit.jsonl` and
//! `connections.jsonl` in the app's Application Support folder.
//!
//! Both the server and the app append to it, and the app derives every
//! account's status dot from its tail. Writing is best-effort throughout — a
//! mail action must never fail because a health line could not be written,
//! and a missed sample costs at most one interval.

use std::path::Path;

use serde_json::{json, Value};
use torromail_core::CoreError;

/// What one login attempt proved. Deliberately three-valued: "we could not
/// get there" and "it said no" need opposite handling, and collapsing them is
/// what made every account look green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthOutcome {
    Ok,
    Rejected,
    Unreachable,
}

impl HealthOutcome {
    /// The word on the wire — in `health.jsonl` and on `--check-account`'s
    /// stderr.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Rejected => "rejected",
            Self::Unreachable => "unreachable",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "ok" => Some(Self::Ok),
            "rejected" => Some(Self::Rejected),
            "unreachable" => Some(Self::Unreachable),
            _ => None,
        }
    }

    /// What a failure says about the account's health — `None` when it says
    /// nothing. A policy denial or a missing message is not a health signal,
    /// and recording one would turn an ordinary refusal into an alarm.
    #[must_use]
    pub fn from_error(error: &CoreError) -> Option<Self> {
        match error {
            CoreError::CredentialRejected(_) => Some(Self::Rejected),
            CoreError::ProviderFailure(_) => Some(Self::Unreachable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthRecord {
    pub ts: u64,
    pub account: String,
    pub outcome: HealthOutcome,
    pub source: String,
    pub detail: String,
}

/// How many lines from the end are read. Far more than any rule needs — the
/// app's three-strikes window is three — but cheap, and it keeps a long file
/// from being parsed in full on every tick.
const TAIL_LINES: usize = 200;

/// Append one record. Every error is swallowed: this is a sample, not a
/// transaction.
pub fn append(path: &Path, account: &str, outcome: HealthOutcome, source: &str, detail: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let line = json!({
        "ts": ts,
        "account": account,
        "outcome": outcome.as_str(),
        "source": source,
        "detail": detail,
    })
    .to_string();

    // Append mode is atomic per write for lines this short, so the app and
    // several server processes never interleave their records.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes());
    }
}

/// The tail of the log, oldest first. A missing file is no records; a line
/// that does not parse, or carries an outcome word we do not know, is skipped
/// rather than taken as fatal.
#[must_use]
pub fn load(path: &Path) -> Vec<HealthRecord> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    lines[start..]
        .iter()
        .filter_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            Some(HealthRecord {
                ts: value["ts"].as_u64()?,
                account: value["account"].as_str()?.to_owned(),
                outcome: HealthOutcome::parse(value["outcome"].as_str()?)?,
                source: value["source"].as_str().unwrap_or_default().to_owned(),
                detail: value["detail"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

/// Whether `account` is due for a check. Without this, every MCP client that
/// spawns its own server process would log in again at launch — five paired
/// clients means five logins per account, every time.
#[must_use]
pub fn needs_check(records: &[HealthRecord], account: &str, now: u64, window: u64) -> bool {
    !records
        .iter()
        .filter(|record| record.account == account)
        .any(|record| now.saturating_sub(record.ts) < window)
}
```

- [ ] **Step 4: Expose the module**

In `crates/torromail-mcp/src/lib.rs`, add near the other module declarations at the top of the file:

```rust
pub mod health;
```

- [ ] **Step 5: Run the tests**

```bash
cargo test -p torromail-mcp --test health_log
```

Expected: PASS, 5 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/torromail-mcp/src/health.rs crates/torromail-mcp/src/lib.rs crates/torromail-mcp/tests/health_log.rs
git commit -m "feat(mcp): the account health log"
```

---

### Task 3: Record health from every real tool call

`run_with_connection` is the single choke point every mailbox-touching tool passes through. It knows the account, whether the connection opened, and how the operation ended — so it is the one place that needs to learn about health. This costs no extra login.

**Files:**
- Modify: `crates/torromail-mcp/src/lib.rs:365-419` (the struct and its constructors), `crates/torromail-mcp/src/lib.rs:1456-1468` (`ToolFailure`), `crates/torromail-mcp/src/lib.rs:718-761` (`run_with_connection`)
- Test: `crates/torromail-mcp/tests/tool_contract.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/torromail-mcp/tests/tool_contract.rs`. The file already has `temp_policy_path` and a `with_connect_override` pattern — read the existing tests around `crates/torromail-mcp/tests/tool_contract.rs:1523` for the exact fixture style used in this file and match it. The three tests:

```rust
#[test]
fn a_rejected_login_during_a_tool_call_is_recorded_as_rejected() {
    let path = temp_policy_path("health-rejected");
    write_policy_with_one_account(&path, "work");
    let health = path.parent().unwrap().join("health.jsonl");
    let _ = std::fs::remove_file(&health);

    let server = LineMcpServer::with_connect_override(&path, false, |_| {
        Err(torromail_core::CoreError::CredentialRejected(
            "IMAP rejected the login: NO bad password".to_owned(),
        ))
    })
    .with_presented_token(Some(TEST_TOKEN));
    let _ = server.handle_line(&search_request("work"));

    let records = torromail_mcp::health::load(&health);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, HealthOutcome::Rejected);
    assert_eq!(records[0].source, "tool-call");
    assert_eq!(records[0].account, "work");
}

#[test]
fn an_unreachable_server_during_a_tool_call_is_recorded_as_unreachable() {
    let path = temp_policy_path("health-unreachable");
    write_policy_with_one_account(&path, "work");
    let health = path.parent().unwrap().join("health.jsonl");
    let _ = std::fs::remove_file(&health);

    let server = LineMcpServer::with_connect_override(&path, false, |_| {
        Err(torromail_core::CoreError::ProviderFailure(
            "connecting imap.example.com:993 failed".to_owned(),
        ))
    })
    .with_presented_token(Some(TEST_TOKEN));
    let _ = server.handle_line(&search_request("work"));

    let records = torromail_mcp::health::load(&health);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, HealthOutcome::Unreachable);
}

#[test]
fn a_policy_denial_records_no_health_at_all() {
    // The connection was fine; the tool was simply not allowed. Recording a
    // health record here would turn every permission refusal into an alarm.
    let path = temp_policy_path("health-denied");
    write_policy_with_one_account_denying_read(&path, "work");
    let health = path.parent().unwrap().join("health.jsonl");
    let _ = std::fs::remove_file(&health);

    let server = LineMcpServer::with_policy_path_and_fixtures(&path)
        .with_presented_token(Some(TEST_TOKEN));
    let _ = server.handle_line(&search_request("work"));

    assert!(
        torromail_mcp::health::load(&health)
            .iter()
            .all(|record| record.outcome != HealthOutcome::Rejected),
        "a denied capability is not a credential problem"
    );
}
```

Add `use torromail_mcp::health::HealthOutcome;` to the file's imports. If `write_policy_with_one_account`, `write_policy_with_one_account_denying_read`, `search_request` or `TEST_TOKEN` do not already exist under those names in this test file, use whatever the file's existing helpers are called — read the top of `crates/torromail-mcp/tests/tool_contract.rs` first and reuse them rather than adding near-duplicates.

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo test -p torromail-mcp --test tool_contract a_rejected_login_during_a_tool_call
```

Expected: FAIL — the health file is never written, so `records.len()` is 0.

- [ ] **Step 3: Give the server a health path**

In `crates/torromail-mcp/src/lib.rs`, add a field to `LineMcpServer` after `connections_path`:

```rust
    /// Sibling of the policy file: every tool call that touches a mailbox
    /// appends one JSONL line here, so the app's status dot reflects what
    /// actually happens rather than what happened once at setup. `None` in
    /// fixture mode, for the same reason as `audit_path`.
    health_path: Option<PathBuf>,
```

Set it to `None` in `fixture()` (alongside `audit_path: None`) and derive it in `with_policy_path`, beside the two existing siblings:

```rust
        let health_path = path.parent().map(|dir| dir.join("health.jsonl"));
```

then add `health_path,` to the struct literal in `with_policy_path`.

- [ ] **Step 4: Teach `ToolFailure` what it says about health**

In `crates/torromail-mcp/src/lib.rs`, add to `impl ToolFailure`, right after `is_connection`:

```rust
    /// What this failure says about the account's health, if anything. A
    /// malformed request or a policy denial says nothing — the connection was
    /// fine, the answer was simply no.
    fn health_outcome(&self) -> Option<HealthOutcome> {
        match self {
            Self::Core(error) => HealthOutcome::from_error(error),
            Self::InvalidParams(_) => None,
        }
    }
```

Add `use crate::health::{self, HealthOutcome};` to the file's imports.

- [ ] **Step 5: Add the recorder**

In `crates/torromail-mcp/src/lib.rs`, next to `record_audit`:

```rust
    /// Append one health record for `account_id`. Best-effort like the audit
    /// log — a mail action must never fail because its health line could not
    /// be written.
    fn record_health(&self, account_id: &AccountId, outcome: HealthOutcome, detail: &str) {
        let Some(path) = &self.health_path else {
            return;
        };
        health::append(path, account_id.as_str(), outcome, "tool-call", detail);
    }
```

- [ ] **Step 6: Record at the three points where health is knowable**

In `run_with_connection`, make these three edits. The connection-open failure at line 740:

```rust
            if stale {
                let provider = match self.open_connection(account_id, facts) {
                    Ok(provider) => provider,
                    Err(error) => {
                        if let Some(outcome) = HealthOutcome::from_error(&error) {
                            self.record_health(account_id, outcome, &error.to_string());
                        }
                        return json_rpc_error(id, -32000, &error.to_string());
                    }
                };
                pool.insert(account_id.clone(), PooledConnection { identity, provider });
            }
```

And the two arms of the `match run(...)`:

```rust
            match run(connection.provider.as_mut(), engine) {
                Ok(payload) => {
                    // A call that worked is the cheapest possible proof the
                    // account is healthy — no extra login needed.
                    self.record_health(account_id, HealthOutcome::Ok, "");
                    return json_rpc_text_result(id, &payload);
                }
                Err(failure) if failure.is_connection() && !rebuilt => {
                    // The session is suspect: drop it so the retry opens a
                    // fresh one, and do not loop forever. No record yet — the
                    // retry decides whether this was a stale socket or a real
                    // problem.
                    pool.remove(account_id);
                    rebuilt = true;
                }
                Err(failure) => {
                    if let Some(outcome) = failure.health_outcome() {
                        self.record_health(account_id, outcome, &failure.message());
                    }
                    return failure.into_response(id);
                }
            }
```

`failure.into_response(id)` consumes the failure, so read the message first. Add to `impl ToolFailure`:

```rust
    /// The human-readable reason, readable without consuming the failure.
    fn message(&self) -> String {
        match self {
            Self::InvalidParams(message) => message.clone(),
            Self::Core(error) => error.to_string(),
        }
    }
```

- [ ] **Step 7: Run the tests**

```bash
cargo test -p torromail-mcp
```

Expected: PASS, the three new tests included.

- [ ] **Step 8: Commit**

```bash
git add crates/torromail-mcp/src/lib.rs crates/torromail-mcp/tests/tool_contract.rs
git commit -m "feat(mcp): record account health from every mailbox tool call"
```

---

### Task 4: Check at server start, and report the outcome from `--check-account`

Two things the app cannot do for itself: prove an account works when a client started the server without the app running, and learn *why* a manual check failed.

**Files:**
- Modify: `crates/torromail-mcp/src/lib.rs:1348-1400` (`check_account`)
- Modify: `crates/torromail-mcp/src/main.rs:44-65`, `crates/torromail-mcp/src/main.rs:67-71`
- Test: `crates/torromail-mcp/tests/health_log.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/torromail-mcp/tests/health_log.rs`:

```rust
#[test]
fn the_sweep_skips_fresh_accounts_and_takes_stale_ones() {
    let path = temp_path("sweep");
    health::append(&path, "fresh", HealthOutcome::Ok, "server-start", "");
    let records = health::load(&path);

    let due: Vec<&str> = ["fresh", "stale"]
        .into_iter()
        .filter(|account| health::needs_check(&records, account, now(), health::SERVER_START_WINDOW))
        .collect();

    assert_eq!(due, ["stale"], "only the account without a fresh record is due");
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo test -p torromail-mcp --test health_log the_sweep_skips
```

Expected: FAIL, `no SERVER_START_WINDOW in torromail_mcp::health`.

- [ ] **Step 3: Name the window**

In `crates/torromail-mcp/src/health.rs`:

```rust
/// How recently an account must have been checked for a starting server to
/// leave it alone. Five minutes: long enough that a burst of client launches
/// produces one login per account, short enough that the first check after a
/// quiet period is genuinely current.
pub const SERVER_START_WINDOW: u64 = 300;
```

- [ ] **Step 4: Make `check_account` report an outcome**

In `crates/torromail-mcp/src/lib.rs`, change `check_account`'s signature and its three failure points. The credential resolution and connection lines currently read `.map_err(|error| error.to_string())?`; they become classified returns:

```rust
/// The connection check behind the app's "Test Connection" button and the
/// `--check-account` flag: resolve the secret, log in over TLS, count the
/// mailboxes. No mail content is touched. The outcome rides alongside the
/// message so the caller can tell a wrong password from a dead network
/// without reading prose.
pub fn check_account(
    account_id: &str,
    policy_path: Option<PathBuf>,
    presented_token: Option<&str>,
) -> (HealthOutcome, String) {
    match check_account_inner(account_id, policy_path, presented_token) {
        Ok(summary) => (HealthOutcome::Ok, summary),
        Err((outcome, message)) => (outcome, message),
    }
}

fn check_account_inner(
    account_id: &str,
    policy_path: Option<PathBuf>,
    presented_token: Option<&str>,
) -> Result<String, (HealthOutcome, String)> {
```

`check_account_inner` is the current body of `check_account` verbatim, with each of its eight early returns given an outcome. Configuration problems are all `Unreachable`: they are not the credentials, and the app must never accuse the password for something that is not it. In source order:

```rust
    let path = policy_path.ok_or((
        HealthOutcome::Unreachable,
        "no policy document path available".to_owned(),
    ))?;
    let text = std::fs::read_to_string(&path).map_err(|error| {
        (
            HealthOutcome::Unreachable,
            format!("policy document unreadable: {error}"),
        )
    })?;
    let document = parse_policy_document(&text)
        .map_err(|message| (HealthOutcome::Unreachable, message))?;

    if let Some(clients) = &document.clients {
        match presented_token.map(sha256_hex) {
            None => return Err((HealthOutcome::Unreachable, NOT_PAIRED.to_owned())),
            Some(presented) => {
                if !is_paired(clients, &presented) {
                    return Err((HealthOutcome::Unreachable, KEY_REJECTED.to_owned()));
                }
            }
        }
    }

    let accounts = document.accounts;
    let account = accounts
        .iter()
        .find(|account| {
            account
                .imap
                .as_ref()
                .is_some_and(|config| config.account_id.as_str() == account_id)
        })
        .ok_or((
            HealthOutcome::Unreachable,
            format!("no IMAP configuration for account {account_id}"),
        ))?;
    let config = account.imap.as_ref().ok_or((
        HealthOutcome::Unreachable,
        format!("no IMAP configuration for account {account_id}"),
    ))?;
```

The three that actually touch the mailbox classify by error instead:

```rust
    let secret = resolve_credential(config, account.oauth.as_ref())
        .map_err(|error| classify(&error))?;
    let provider =
        torromail_imap_tls::connect_account(config, &secret).map_err(|error| classify(&error))?;
    let mailboxes = provider
        .list_mailboxes(&config.account_id)
        .map_err(|error| classify(&error))?;

    Ok(format!("login ok, {} mailboxes visible", mailboxes.len()))
```

with, next to it:

```rust
/// A failure on the mailbox path, as an outcome and a message. Anything that
/// is not a spoken refusal counts as unreachable — the safe side, because an
/// unreachable account gets a grace period and a wrongly-accused password
/// does not.
fn classify(error: &CoreError) -> (HealthOutcome, String) {
    (
        HealthOutcome::from_error(error).unwrap_or(HealthOutcome::Unreachable),
        error.to_string(),
    )
}
```

- [ ] **Step 5: Add the server-start sweep**

In `crates/torromail-mcp/src/lib.rs`, as a free function beside `check_account`:

```rust
/// Check every configured account once as the server comes up, skipping any
/// that were checked in the last `SERVER_START_WINDOW` seconds. Without the
/// debounce, five paired clients each spawning their own server process would
/// mean five logins per account on every launch.
///
/// Best-effort and silent: a server must start and serve tools whatever the
/// mailboxes are doing.
pub fn sweep_account_health(policy_path: Option<PathBuf>, presented_token: Option<&str>) {
    let Some(path) = policy_path.clone() else {
        return;
    };
    let Some(health_path) = path.parent().map(|dir| dir.join("health.jsonl")) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(document) = parse_policy_document(&text) else {
        return;
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let records = health::load(&health_path);

    for account in &document.accounts {
        let Some(config) = account.imap.as_ref() else {
            continue;
        };
        let id = config.account_id.as_str().to_owned();
        if !health::needs_check(&records, &id, now, health::SERVER_START_WINDOW) {
            continue;
        }
        let (outcome, message) = check_account(&id, Some(path.clone()), presented_token);
        health::append(&health_path, &id, outcome, "server-start", &message);
    }
}
```

- [ ] **Step 6: Wire `main.rs`**

In `crates/torromail-mcp/src/main.rs`, replace the `--check-account` block (lines 47-65) with:

```rust
    if let Some(position) = arguments
        .iter()
        .position(|argument| argument == "--check-account")
    {
        let Some(account_id) = arguments.get(position + 1) else {
            eprintln!("--check-account needs an account id");
            std::process::exit(2);
        };
        let (outcome, message) =
            torromail_mcp::check_account(account_id, policy_path(), presented_token.as_deref());
        if outcome == torromail_mcp::health::HealthOutcome::Ok {
            println!("{message}");
            return Ok(());
        }
        // Two-part stderr: the outcome word on its own first line, the reason
        // after it. The app reads the first line to tell a wrong password
        // from a dead network without parsing prose; the exit status keeps
        // its old meaning for anything that only checks that.
        eprintln!("{}", outcome.as_str());
        eprintln!("{message}");
        std::process::exit(1);
    }
```

And before the stdio loop, after the server is built (line 71):

```rust
    // Prove the accounts before the first tool call needs them, so the app's
    // dots are current even when a client — not the app — started us.
    torromail_mcp::sweep_account_health(policy_path(), presented_token.as_deref());
```

- [ ] **Step 7: Run everything**

```bash
cargo test && cargo build -p torromail-mcp
```

Expected: PASS. Any other caller of `check_account` the compiler flags now gets a tuple instead of a `Result` — there is exactly one, in `main.rs`, already updated.

- [ ] **Step 8: Commit**

```bash
git add crates/torromail-mcp/src/lib.rs crates/torromail-mcp/src/main.rs crates/torromail-mcp/src/health.rs crates/torromail-mcp/tests/health_log.rs
git commit -m "feat(mcp): check accounts at server start and classify --check-account"
```

---

### Task 5: The Swift health log and the derivation rule

The heart of the feature on the app side, and deliberately a pure function: given the records and a fallback, what colour is the dot? No timer, no subprocess, no mailbox.

**Files:**
- Create: `apps/TorroMailApp/Sources/TorroMailKit/HealthLog.swift`
- Test: `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift`

- [ ] **Step 1: Write the failing test**

Append to `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift`. The file uses a top-level `require(_:_:)` helper — keep that style.

```swift
// MARK: - Account health

// The whole point of the feature: a green dot must mean "checked recently and
// fine", not "worked once during setup".

func healthRecord(
    _ account: String,
    _ outcome: HealthOutcome,
    secondsAgo: TimeInterval = 0,
    detail: String = ""
) -> HealthRecord {
    HealthRecord(
        accountID: account,
        at: Date().addingTimeInterval(-secondsAgo),
        outcome: outcome,
        source: "periodic",
        detail: detail
    )
}

require(
    HealthLog.derive(records: [], accountID: "work", fallback: .connected) == .connected,
    "with no records at all the stored state stands"
)
require(
    HealthLog.derive(
        records: [healthRecord("work", .ok)],
        accountID: "work",
        fallback: .needsTest
    ) == .connected,
    "a successful check is green whatever the stored state said"
)
require(
    HealthLog.derive(
        records: [healthRecord("work", .rejected, detail: "credentials rejected: NO")],
        accountID: "work",
        fallback: .connected
    ).isBroken,
    "refused credentials go red immediately — they will not fix themselves"
)
require(
    HealthLog.derive(
        records: [
            healthRecord("work", .ok, secondsAgo: 300),
            healthRecord("work", .unreachable, secondsAgo: 200),
            healthRecord("work", .unreachable, secondsAgo: 100)
        ],
        accountID: "work",
        fallback: .needsTest
    ) == .connected,
    "two unreachable checks are a flaky network, not a broken account"
)
require(
    HealthLog.derive(
        records: [
            healthRecord("work", .ok, secondsAgo: 400),
            healthRecord("work", .unreachable, secondsAgo: 300),
            healthRecord("work", .unreachable, secondsAgo: 200),
            healthRecord("work", .unreachable, secondsAgo: 100)
        ],
        accountID: "work",
        fallback: .needsTest
    ).isBroken,
    "three in a row is a problem worth showing"
)
require(
    HealthLog.derive(
        records: [
            healthRecord("work", .unreachable, secondsAgo: 500),
            healthRecord("work", .unreachable, secondsAgo: 400),
            healthRecord("work", .ok, secondsAgo: 300),
            healthRecord("work", .unreachable, secondsAgo: 200),
            healthRecord("work", .unreachable, secondsAgo: 100)
        ],
        accountID: "work",
        fallback: .needsTest
    ) == .connected,
    "a success in between resets the count"
)
require(
    HealthLog.derive(
        records: [healthRecord("other", .rejected)],
        accountID: "work",
        fallback: .connected
    ) == .connected,
    "another account's trouble is not this account's"
)

// A line whose outcome word we do not know is skipped, not fatal: an older or
// newer build writing a word this one has never heard of must not take the
// status display down with it.
let mixedLog = FileManager.default.temporaryDirectory
    .appendingPathComponent("torromail-contract-health.jsonl")
try? FileManager.default.removeItem(at: mixedLog)
FileManager.default.createFile(
    atPath: mixedLog.path,
    contents: Data(
        """
        {"ts":1,"account":"work","outcome":"quantum","source":"periodic","detail":""}
        {"ts":2,"account":"work","outcome":"ok","source":"periodic","detail":""}
        not json at all
        """.utf8
    )
)
require(
    HealthLog.load(from: mixedLog).count == 1,
    "an unknown outcome and a broken line are both skipped, and the good line survives"
)

// Notifications follow crossings, not states — otherwise a broken account
// announces itself every fifteen minutes until the user stops reading.
let brokenAccount = MailAccount(
    id: "work",
    name: "Work",
    email: "work@example.com",
    provider: .imapSmtp,
    loginMethod: .password,
    username: "work@example.com",
    connectionState: .failed("credentials rejected: NO")
)
let healthyAccount = MailAccount(
    id: "work",
    name: "Work",
    email: "work@example.com",
    provider: .imapSmtp,
    loginMethod: .password,
    username: "work@example.com",
    connectionState: .connected
)
require(
    HealthLog.transitions(previous: ["work": false], accounts: [brokenAccount]).count == 1,
    "going from healthy to broken is worth saying once"
)
require(
    HealthLog.transitions(previous: ["work": true], accounts: [brokenAccount]).isEmpty,
    "a still-broken account says nothing further"
)
require(
    HealthLog.transitions(previous: ["work": true], accounts: [healthyAccount])
        == [.recovered(accountID: "work")],
    "recovery gets its own quiet all-clear"
)
require(
    HealthLog.transitions(previous: [:], accounts: [brokenAccount]).isEmpty,
    "an account seen for the first time has not crossed anything"
)
```

- [ ] **Step 2: Run it to verify it fails**

```bash
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
```

Expected: FAIL, `cannot find 'HealthLog' in scope`.

- [ ] **Step 3: Write the module**

Create `apps/TorroMailApp/Sources/TorroMailKit/HealthLog.swift`:

```swift
import Foundation

/// What one login attempt proved. Three-valued because "we could not get
/// there" and "it said no" need opposite handling: a wrong password will not
/// fix itself, a train tunnel will.
public enum HealthOutcome: String, Hashable, Sendable {
    case ok
    case rejected
    case unreachable
}

/// One check, as the server or the app recorded it.
public struct HealthRecord: Hashable, Sendable {
    public var accountID: String
    public var at: Date
    public var outcome: HealthOutcome
    /// `periodic`, `server-start`, `tool-call` or `manual` — kept for the log,
    /// nothing derives from it.
    public var source: String
    public var detail: String

    public init(
        accountID: String,
        at: Date,
        outcome: HealthOutcome,
        source: String = "",
        detail: String = ""
    ) {
        self.accountID = accountID
        self.at = at
        self.outcome = outcome
        self.source = source
        self.detail = detail
    }
}

/// Reads and appends `health.jsonl` — the log the MCP server and the app both
/// write, and the only thing that decides an account's status dot. Sibling of
/// `audit.jsonl`, and shares its never-throw contract: a missing file or a
/// half-written trailing line yields what it can rather than taking the
/// window down.
public enum HealthLog {
    /// How many consecutive unreachable checks it takes before an account is
    /// called broken. Below this it keeps whatever it was, because a flaky
    /// network is not a credential problem and a false red teaches people to
    /// ignore the dot.
    public static let unreachableGrace = 3

    /// `~/Library/Application Support/TorroMail/health.jsonl` — beside the
    /// policy document and the two logs the server already writes.
    public static func defaultURL(fileManager: FileManager = .default) throws -> URL {
        try fileManager
            .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("TorroMail", isDirectory: true)
            .appendingPathComponent("health.jsonl")
    }

    /// The tail of the log, oldest first. Reads the last `limit` lines: far
    /// more than the three the grace rule needs, and cheap enough to do on
    /// every file change.
    public static func load(
        limit: Int = 200,
        from url: URL? = nil,
        fileManager: FileManager = .default
    ) -> [HealthRecord] {
        guard let target = try? url ?? defaultURL(fileManager: fileManager),
              let text = try? String(contentsOf: target, encoding: .utf8) else {
            return []
        }
        return text
            .split(separator: "\n", omittingEmptySubsequences: true)
            .suffix(limit)
            .compactMap { line -> HealthRecord? in
                guard let data = line.data(using: .utf8),
                      let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let ts = raw["ts"] as? TimeInterval,
                      let account = raw["account"] as? String,
                      let word = raw["outcome"] as? String,
                      let outcome = HealthOutcome(rawValue: word) else {
                    return nil
                }
                return HealthRecord(
                    accountID: account,
                    at: Date(timeIntervalSince1970: ts),
                    outcome: outcome,
                    source: raw["source"] as? String ?? "",
                    detail: raw["detail"] as? String ?? ""
                )
            }
    }

    /// Append one record. Best-effort: a missed sample costs at most one
    /// interval, and nothing the user does should fail over a log line.
    public static func append(
        _ record: HealthRecord,
        to url: URL? = nil,
        fileManager: FileManager = .default
    ) {
        guard let target = try? url ?? defaultURL(fileManager: fileManager) else { return }
        let payload: [String: Any] = [
            "ts": record.at.timeIntervalSince1970,
            "account": record.accountID,
            "outcome": record.outcome.rawValue,
            "source": record.source,
            "detail": record.detail
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              var line = String(data: data, encoding: .utf8) else {
            return
        }
        line.append("\n")

        if !fileManager.fileExists(atPath: target.path) {
            try? Data().write(to: target)
        }
        guard let handle = try? FileHandle(forWritingTo: target) else { return }
        defer { try? handle.close() }
        _ = try? handle.seekToEnd()
        try? handle.write(contentsOf: Data(line.utf8))
    }

    /// The status one account's records add up to.
    ///
    /// `fallback` is what the account carried before any record existed — the
    /// state restored from disk at launch. It is also what an unreachable
    /// streak too short to count falls back to, which is the whole grace rule:
    /// nothing changes until we are sure.
    public static func derive(
        records: [HealthRecord],
        accountID: String,
        fallback: ConnectionState,
        grace: Int = unreachableGrace
    ) -> ConnectionState {
        let mine = records
            .filter { $0.accountID == accountID }
            .sorted { $0.at < $1.at }
        guard let last = mine.last else { return fallback }

        switch last.outcome {
        case .ok:
            return .connected
        case .rejected:
            return .failed(last.detail.isEmpty ? "credentials rejected" : last.detail)
        case .unreachable:
            let streak = mine.reversed().prefix { $0.outcome == .unreachable }.count
            guard streak >= grace else {
                // Not yet convinced. Whatever the last real verdict was still
                // stands — a couple of missed checks is a network, not an
                // account.
                let settled = mine.last { $0.outcome != .unreachable }
                switch settled?.outcome {
                case .ok: return .connected
                case .rejected: return .failed(settled?.detail ?? "credentials rejected")
                default: return fallback
                }
            }
            return .failed(last.detail.isEmpty ? "server not reachable" : last.detail)
        }
    }

    /// When each account was last checked, for the "last checked …" line in
    /// the account detail.
    public static func lastChecked(records: [HealthRecord]) -> [String: Date] {
        var latest: [String: Date] = [:]
        for record in records where (latest[record.accountID] ?? .distantPast) < record.at {
            latest[record.accountID] = record.at
        }
        return latest
    }
}

/// A crossing between healthy and broken — the only thing worth a
/// notification. A standing problem is not a crossing: it stays red in the
/// app, which is where a standing problem belongs.
public enum HealthTransition: Hashable, Sendable {
    case broke(accountID: String, reason: String)
    case recovered(accountID: String)
}

extension HealthLog {
    /// Each account's broken-ness, the shape `transitions` compares against.
    public static func brokenness(accounts: [MailAccount]) -> [String: Bool] {
        Dictionary(
            accounts.map { ($0.id, $0.connectionState.isBroken) },
            uniquingKeysWith: { first, _ in first }
        )
    }

    /// Which accounts crossed since `previous` was taken.
    ///
    /// Pure on purpose: deciding what counts as a crossing is the part worth
    /// testing, and it should not need a notification centre to exercise. An
    /// account missing from `previous` is new and yields nothing — it has not
    /// crossed anything, it has only appeared.
    public static func transitions(
        previous: [String: Bool],
        accounts: [MailAccount]
    ) -> [HealthTransition] {
        accounts.compactMap { account in
            guard let was = previous[account.id] else { return nil }
            let broken = account.connectionState.isBroken
            guard was != broken else { return nil }
            if broken {
                let reason: String
                if case let .failed(message) = account.connectionState, !message.isEmpty {
                    reason = message
                } else {
                    reason = ""
                }
                return .broke(accountID: account.id, reason: reason)
            }
            return .recovered(accountID: account.id)
        }
    }
}
```

- [ ] **Step 4: Run the contract test**

```bash
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
```

Expected: exits 0, no "Contract failed" output.

- [ ] **Step 5: Commit**

```bash
git add apps/TorroMailApp/Sources/TorroMailKit/HealthLog.swift apps/TorroMailApp/Tests/TorroMailKitContract/main.swift
git commit -m "feat(app): the health log and the rule that derives a status dot"
```

---

### Task 6: A check that classifies and cannot hang

`AccountCheck.run` currently returns a bare `ConnectionState` and waits on the subprocess forever. Both have to change before a timer may call it every fifteen minutes.

**Files:**
- Modify: `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift:522-574`
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift:1405-1411`
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/AccountSetupWizard.swift` (the `AccountCheck.run` call site — find it with the grep in Step 4)

- [ ] **Step 1: Add the result type**

In `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift`, immediately above `public enum AccountCheck`:

```swift
/// What one run of the connection check found: the classified outcome, the
/// reason in words, and the state a caller acting on this single check alone
/// should show.
public struct AccountCheckResult: Hashable, Sendable {
    public var outcome: HealthOutcome
    public var detail: String

    public init(outcome: HealthOutcome, detail: String) {
        self.outcome = outcome
        self.detail = detail
    }

    /// For the manual test button: the user asked right now, so any failure
    /// is worth showing right now. The grace period for unreachable servers
    /// belongs to the background monitor, which has a log to judge from.
    public var state: ConnectionState {
        outcome == .ok ? .connected : .failed(detail)
    }
}
```

- [ ] **Step 2: Rewrite `AccountCheck.run`**

Replace the body of `AccountCheck.run` in the same file. Everything up to `try process.run()` stays exactly as it is; the return type and everything from `process.waitUntilExit()` onward change:

```swift
    public static func run(
        accountID: String,
        executableName: String,
        policyURL: URL? = nil,
        timeout: TimeInterval = 30
    ) -> AccountCheckResult {
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        guard let command = locator.resolve() else {
            return AccountCheckResult(outcome: .unreachable, detail: "MCP executable not found")
        }

        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments + ["--check-account", accountID]
        var environment = ProcessInfo.processInfo.environment
        if let url = policyURL ?? (try? PolicyDocument.defaultURL()) {
            environment["TORROMAIL_POLICY_PATH"] = url.path
        }
        // The check logs into the real mailbox, so it sits behind the same
        // pairing gate as the tools — the app presents its own key.
        if let appToken = try? MCPClientKeyStore.appToken() {
            environment["TORROMAIL_TOKEN"] = appToken
        }
        process.environment = environment
        let errorPipe = Pipe()
        process.standardOutput = Pipe()
        process.standardError = errorPipe

        let finished = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in finished.signal() }
        do {
            try process.run()
        } catch {
            return AccountCheckResult(outcome: .unreachable, detail: error.localizedDescription)
        }

        // A hung TLS handshake must not wedge the monitor's queue forever.
        // A check that never answers is a server we could not reach, which is
        // exactly what the grace period is for.
        if finished.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            return AccountCheckResult(
                outcome: .unreachable,
                detail: "the connection check timed out"
            )
        }

        if process.terminationStatus == 0 {
            return AccountCheckResult(outcome: .ok, detail: "")
        }

        // Two-part stderr, as `main.rs` writes it: the outcome word alone on
        // the first line, the reason after it. A binary older than that
        // contract writes only prose, whose first line parses as no outcome —
        // and unreachable is the safe reading, because it earns a grace
        // period rather than accusing the password.
        let text = String(
            data: errorPipe.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        )?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        var lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        let outcome: HealthOutcome
        if let first = lines.first, let parsed = HealthOutcome(rawValue: first) {
            outcome = parsed
            lines.removeFirst()
        } else {
            outcome = .unreachable
        }
        let detail = lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return AccountCheckResult(
            outcome: outcome,
            detail: detail.isEmpty ? "connection check failed" : detail
        )
    }
```

- [ ] **Step 3: Update the manual test button**

In `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`, in `testConnection()`, replace the detached task body:

```swift
        Task.detached(priority: .userInitiated) {
            let result = AccountCheck.run(accountID: accountID, executableName: executable)
            // The user pressed the button, so this check is the record — the
            // monitor's file is where every check lands.
            HealthLog.append(
                HealthRecord(
                    accountID: accountID,
                    at: Date(),
                    outcome: result.outcome,
                    source: "manual",
                    detail: result.detail
                )
            )
            await MainActor.run {
                account.connectionState = result.state
                isCheckingConnection = false
            }
        }
```

- [ ] **Step 4: Update the wizard call site**

```bash
grep -n "AccountCheck.run" apps/TorroMailApp/Sources/TorroMailApp/AccountSetupWizard.swift
```

The wizard proves a candidate account against a throwaway policy document. Wherever it assigns the returned value to a `ConnectionState`, append `.state` to the call — the wizard is a manual check and wants the same immediate verdict as the button. Do **not** append a `HealthLog` record there: the account does not exist yet, and a record naming an id that never joins the list would be a permanent ghost in the log.

- [ ] **Step 5: Build**

```bash
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Expected: builds clean. If the compiler reports a `Sendable` complaint about capturing `account` in the detached task, that is pre-existing code shape — leave it as the surrounding code has it.

- [ ] **Step 6: Commit**

```bash
git add apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift apps/TorroMailApp/Sources/TorroMailApp/AccountSetupWizard.swift
git commit -m "feat(app): the connection check reports why it failed and cannot hang"
```

---

### Task 7: The monitor

The timer that makes all of this continuous. It writes to `health.jsonl` and nothing else — the watcher and the model pick it up from there, so the monitor never touches the UI.

**Files:**
- Create: `apps/TorroMailApp/Sources/TorroMailKit/AccountHealthMonitor.swift`

- [ ] **Step 1: Write the file**

Mirror `AuditWatcher` (`apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift:1724`) exactly in shape — a plain `final class` holding a `DispatchSourceTimer`, started and stopped by the app. That pattern already compiles under this package's Swift 6 settings; deviating from it is how strict-concurrency errors appear.

```swift
import Foundation

#if canImport(AppKit)
import AppKit
#endif

/// Checks every account on a timer and writes what it finds to
/// `health.jsonl`. Nothing here touches the UI: the app watches that file the
/// same way it watches the audit log, so one path updates the dots no matter
/// who wrote the record — this monitor, a tool call, or a starting server.
///
/// Checks run one after another rather than at once. A handful of accounts
/// arriving as a burst of simultaneous logins is exactly the kind of traffic
/// providers throttle.
public final class AccountHealthMonitor {
    /// Fifteen minutes. Not a setting: the user cannot make a better decision
    /// about it than we can, and a control that is not a decision does not
    /// belong in the UI.
    public static let defaultInterval: TimeInterval = 900

    private let interval: TimeInterval
    private let queue = DispatchQueue(label: "de.torro.mail.health", qos: .utility)
    private var timer: DispatchSourceTimer?
    private var wakeObserver: NSObjectProtocol?

    /// What to check, as last published from the main actor. Guarded by
    /// `queue`, which is also where the checks run — so a tick never reads
    /// half of an update.
    ///
    /// A snapshot rather than a closure back into the model on purpose: the
    /// timer fires on a background queue, and reaching into main-actor state
    /// from there is a runtime trap, not a compile error.
    private var accountIDs: [String] = []
    private var executableName = ""

    public init(interval: TimeInterval = defaultInterval) {
        self.interval = interval
    }

    /// Publish what the next tick should check. Called from the main actor
    /// whenever the accounts or the configured executable change, including
    /// once before `start()`.
    public func update(accountIDs: [String], executableName: String) {
        queue.async {
            self.accountIDs = accountIDs
            self.executableName = executableName
        }
    }

    /// Starts the timer and checks immediately: a status the app shows at
    /// launch should be from seconds ago, not from whenever it last quit.
    public func start() {
        stop()
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: interval)
        timer.setEventHandler { [weak self] in
            self?.runOnce(source: "periodic")
        }
        timer.resume()
        self.timer = timer

        #if canImport(AppKit)
        // Waking from sleep is the other moment the stored status is most
        // likely stale — and the moment the network is least likely to be up
        // yet, which is what the grace period covers.
        wakeObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification,
            object: nil,
            queue: nil
        ) { [weak self] _ in
            self?.queue.async { self?.runOnce(source: "periodic") }
        }
        #endif
    }

    public func stop() {
        timer?.cancel()
        timer = nil
        #if canImport(AppKit)
        if let wakeObserver {
            NSWorkspace.shared.notificationCenter.removeObserver(wakeObserver)
        }
        wakeObserver = nil
        #endif
    }

    deinit { stop() }

    /// One pass over every account. Called on `queue`, so the checks are
    /// serial by construction and the snapshot is safe to read.
    private func runOnce(source: String) {
        let executable = executableName
        guard !executable.isEmpty else { return }
        for accountID in accountIDs {
            let result = AccountCheck.run(accountID: accountID, executableName: executable)
            HealthLog.append(
                HealthRecord(
                    accountID: accountID,
                    at: Date(),
                    outcome: result.outcome,
                    source: source,
                    detail: result.detail
                )
            )
        }
    }
}
```

- [ ] **Step 2: Build**

```bash
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add apps/TorroMailApp/Sources/TorroMailKit/AccountHealthMonitor.swift
git commit -m "feat(app): check every account on a timer"
```

---

### Task 8: Let the model follow the log

**Files:**
- Modify: `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift` (add to the `TorroMailModel` extension near `reloadAudit`, around line 2679)
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift:445-496`

- [ ] **Step 1: Add the model method**

In `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift`, in the `extension TorroMailModel` beside `reloadAudit`:

```swift
    /// Re-derive every account's connection state from `health.jsonl` — after
    /// the watcher reports the file grew, or at launch.
    ///
    /// Only genuinely changed states are assigned. `accounts` is observed:
    /// writing an identical value would republish the policy document and
    /// rewrite the state file on every quiet tick, four times an hour, forever.
    public func applyHealth(from url: URL? = nil) {
        let records = HealthLog.load(from: url)
        guard !records.isEmpty else { return }
        for index in accounts.indices {
            let derived = HealthLog.derive(
                records: records,
                accountID: accounts[index].id,
                fallback: accounts[index].connectionState
            )
            if accounts[index].connectionState != derived {
                accounts[index].connectionState = derived
            }
        }
        lastHealthCheck = HealthLog.lastChecked(records: records)
    }
```

And a published property on `TorroMailModel`, beside `audit` (around line 2566):

```swift
    /// When each account was last checked, for the account detail's "last
    /// checked …" line. Not persisted: it is a fact about the log, and the log
    /// is on disk.
    @Published public var lastHealthCheck: [String: Date] = [:]
```

- [ ] **Step 2: Wire the app**

In `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`, add beside the two existing watchers (line 455):

```swift
    /// The same file watcher again, pointed at `health.jsonl` — the server, a
    /// tool call and the monitor all append there, and this is what turns any
    /// of them into a status dot.
    @State private var healthWatcher = AuditWatcher(url: try? HealthLog.defaultURL())
```

Below the `@StateObject` declarations, add the monitor. It reads the model on each tick, so it is created in `init` and given closures rather than values:

```swift
    @State private var healthMonitor = AccountHealthMonitor()
```

In the `.task { … }` block, after `connectionWatcher.start { … }`:

```swift
                    // Whatever the log already says wins over the state file:
                    // the dots must be current before the first check lands.
                    model.applyHealth()
                    healthWatcher.start {
                        Task { @MainActor in model.applyHealth() }
                    }
                    healthMonitor.update(
                        accountIDs: model.accounts.map(\.id),
                        executableName: model.generalSettings.mcpExecutable
                    )
                    healthMonitor.start()
```

The monitor's first tick fires immediately, and `update` hops through its queue before the timer handler reads the snapshot, so the ordering holds. `runOnce` bails on an empty executable name, so even if a tick did land first it would skip rather than check nothing.

Keep the snapshot current by adding to the existing `.onChange(of: model.accounts, initial: true)` handler:

```swift
                    healthMonitor.update(
                        accountIDs: accounts.map(\.id),
                        executableName: model.generalSettings.mcpExecutable
                    )
```

- [ ] **Step 3: Build and check the file actually grows**

```bash
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Then build and launch the dev bundle, wait a few seconds, and look:

```bash
./scripts/make-app-bundle.sh && open apps/TorroMailApp/.build/TorroMail.app
```

```bash
tail -3 ~/Library/Application\ Support/TorroMail/health.jsonl
```

Expected: one line per configured account, `"source":"periodic"`.

- [ ] **Step 4: Commit**

```bash
git add apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift
git commit -m "feat(app): derive account status from the health log"
```

---

### Task 9: Notify on the crossing, not on the state

**Files:**
- Create: `apps/TorroMailApp/Sources/TorroMailApp/HealthNotifier.swift`
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`

- [ ] **Step 1: Write the notifier**

Create `apps/TorroMailApp/Sources/TorroMailApp/HealthNotifier.swift`:

```swift
import Foundation
import UserNotifications

import TorroMailKit

/// Delivers what `HealthLog.transitions` decided. Deciding *whether* an
/// account crossed lives in TorroMailKit, where it is a pure function with
/// tests; this type only knows how to put one on screen and how to ask for
/// permission once.
///
/// A broken account is announced once, when it breaks. It stays red in the app
/// for as long as it is broken, which is where a standing problem belongs; a
/// notification every fifteen minutes would only teach the user to dismiss
/// them without reading.
@MainActor
final class HealthNotifier {
    private var known: [String: Bool] = [:]
    private var authorized = false
    private var askedForAuthorization = false

    /// Seeds from the state the accounts were restored with, so an account
    /// that was already broken when the app quit does not announce itself
    /// again at launch — only a genuine crossing does.
    func seed(accounts: [MailAccount]) {
        known = HealthLog.brokenness(accounts: accounts)
    }

    func reconcile(accounts: [MailAccount]) {
        let names = Dictionary(
            accounts.map { ($0.id, $0.name) },
            uniquingKeysWith: { first, _ in first }
        )
        for transition in HealthLog.transitions(previous: known, accounts: accounts) {
            switch transition {
            case let .broke(accountID, reason):
                notify(
                    title: names[accountID] ?? accountID,
                    body: reason.isEmpty
                        ? L("TorroMail can no longer reach this account.")
                        : reason,
                    id: "health-broken-\(accountID)"
                )
            case let .recovered(accountID):
                notify(
                    title: names[accountID] ?? accountID,
                    body: L("Reachable again."),
                    id: "health-ok-\(accountID)"
                )
            }
        }
        // Also drops accounts the user removed, which should not keep a slot.
        known = HealthLog.brokenness(accounts: accounts)
    }

    private func notify(title: String, body: String, id: String) {
        requestAuthorizationIfNeeded()
        guard authorized else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        let request = UNNotificationRequest(identifier: id, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }

    /// Asked once, on the first crossing there is something to say about —
    /// not at launch, where a permission sheet would greet a user who has not
    /// yet done anything. A refusal is final: the app falls back to its own
    /// status display and does not ask again.
    private func requestAuthorizationIfNeeded() {
        guard !askedForAuthorization else { return }
        askedForAuthorization = true
        UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound]) { granted, _ in
                Task { @MainActor in self.authorized = granted }
            }
    }
}
```

`requestAuthorization` answers asynchronously, so the very first crossing may be missed while the user is still looking at the permission sheet. That is deliberate: the account stays red in the app, and the next crossing notifies. Do not add a queue for it.

- [ ] **Step 2: Wire it**

In `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`, add beside the other `@State` properties:

```swift
    @State private var healthNotifier = HealthNotifier()
```

In the `.task { … }` block, before `model.applyHealth()`:

```swift
                    // Seed before the first derivation, so the state the app
                    // was restored with is the baseline a crossing is measured
                    // against.
                    healthNotifier.seed(accounts: model.accounts)
```

And extend the existing `.onChange(of: model.accounts, initial: true)` handler by adding, as its last statement:

```swift
                    healthNotifier.reconcile(accounts: accounts)
```

- [ ] **Step 3: Build**

```bash
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Expected: builds clean. `UNUserNotificationCenter.current()` traps in a process with no bundle identifier — it works from `TorroMail.app` (built by `scripts/make-app-bundle.sh`, which copies `Info.plist`) and would crash under a bare `swift run`. The app target is only ever run from the bundle, so this is fine; do not add notification calls to `TorroMailKit`, which the contract test runs unbundled.

- [ ] **Step 4: Commit**

```bash
git add apps/TorroMailApp/Sources/TorroMailApp/HealthNotifier.swift apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift
git commit -m "feat(app): notify when an account breaks, once"
```

---

### Task 10: Show it

**Files:**
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift:701-719` (dashboard), `:1371-1383` (account detail)
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/Resources/de.lproj/Localizable.strings`

- [ ] **Step 1: Add the dashboard section**

In `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`, add a new private view near `AccountsOverviewCard`:

```swift
/// Accounts that are actually broken, and why. Exists only while there is
/// something wrong: healthy state stays quiet, and a card that is always there
/// saying "all good" trains people to stop reading it.
private struct BrokenAccountsCard: View {
    @EnvironmentObject private var model: TorroMailModel

    private var broken: [MailAccount] {
        model.accounts.filter { $0.connectionState.isBroken }
    }

    var body: some View {
        DashboardCard(title: L("Needs your attention")) {
            VStack(spacing: 0) {
                ForEach(Array(broken.enumerated()), id: \.element.id) { index, account in
                    if index > 0 { Divider().padding(.leading, 38) }
                    Button {
                        model.openAccount(id: account.id)
                    } label: {
                        HStack(spacing: 10) {
                            Image(systemName: "exclamationmark.triangle.fill")
                                .foregroundStyle(.red)
                                .frame(width: 26)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(account.name).font(.body)
                                Text(reason(for: account))
                                    .font(.subheadline)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            Spacer(minLength: 8)
                            Image(systemName: "chevron.right")
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(.tertiary)
                        }
                        .contentShape(.rect)
                        .padding(.vertical, 8)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    private func reason(for account: MailAccount) -> String {
        if case let .failed(message) = account.connectionState, !message.isEmpty {
            return message
        }
        return L("TorroMail can no longer reach this account.")
    }
}
```

And render it in `DashboardView`, directly after `ServiceStatusCard()`:

```swift
                    ServiceStatusCard()
                    if model.accounts.contains(where: { $0.connectionState.isBroken }) {
                        BrokenAccountsCard()
                    }
```

- [ ] **Step 2: Add "last checked" to the account detail**

In the same file, in the `HStack` at line 1371 that holds `ConnectionStatusBadge`, insert after the badge:

```swift
                ConnectionStatusBadge(state: account.connectionState)
                if let checked = model.lastHealthCheck[account.id] {
                    Text(lastCheckedText(checked))
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
                Spacer()
```

and, in the same view's private helpers:

```swift
    /// Relative, because the exact second is never the question — "is this
    /// current?" is.
    private func lastCheckedText(_ date: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return String(
            format: L("checked %@"),
            formatter.localizedString(for: date, relativeTo: Date())
        )
    }
```

- [ ] **Step 3: Add the German strings**

Append to `apps/TorroMailApp/Sources/TorroMailApp/Resources/de.lproj/Localizable.strings`:

```
/* Account health */
"Needs your attention" = "Braucht deine Aufmerksamkeit";
"TorroMail can no longer reach this account." = "TorroMail erreicht dieses Konto nicht mehr.";
"Reachable again." = "Wieder erreichbar.";
"checked %@" = "geprüft %@";
"credentials rejected" = "Zugangsdaten abgelehnt";
"server not reachable" = "Server nicht erreichbar";
"the connection check timed out" = "Die Verbindungsprüfung hat zu lange gedauert.";
```

The messages that come from the Rust side (`credentials rejected: …`, `connecting … failed: …`) arrive as English prose from the mail server and are shown as-is — they are the server's words, not ours, and translating a server's answer would misrepresent it.

- [ ] **Step 4: Run every check**

```bash
cargo test && cargo build -p torromail-mcp && swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract && swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Expected: all four succeed.

- [ ] **Step 5: Prove it end to end by hand**

```bash
./scripts/make-app-bundle.sh && open apps/TorroMailApp/.build/TorroMail.app
```

1. Open an account, type a wrong password, press Test Connection. The badge goes red with the server's reason; `health.jsonl` gains a `"source":"manual","outcome":"rejected"` line.
2. Quit and relaunch. The dot is still red — this is the case that was broken before.
3. Watch for a notification: with the account already broken at launch there is none (correct — no crossing). Fix the password and press Test Connection; an all-clear notification arrives and the dashboard card disappears.
4. Break it again and wait one interval without touching the button. The periodic check writes a `"source":"periodic","outcome":"rejected"` record and the failure notification arrives.

```bash
tail -5 ~/Library/Application\ Support/TorroMail/health.jsonl
```

- [ ] **Step 6: Update the agent guide**

`AGENTS.md` documents the file layout the server and app share. Add `health.jsonl` beside the existing mentions of the policy document and audit log, in two or three sentences: what it holds, who writes it, and that the status dot is derived from it rather than stored.

- [ ] **Step 7: Commit**

```bash
git add apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift apps/TorroMailApp/Sources/TorroMailApp/Resources/de.lproj/Localizable.strings AGENTS.md
git commit -m "feat(app): surface broken accounts on the dashboard"
```

---

## Notes for the implementer

**The one thing that must not regress.** `ToolFailure::is_connection` (`crates/torromail-mcp/src/lib.rs:1460`) must keep matching only `ProviderFailure`. It drives "drop the pooled session and retry once". Adding `CredentialRejected` to it would retry a login the server just refused — pointless at best, and a step toward a provider lockout at worst.

**Why the grace period is asymmetric.** Refused credentials go red on the first record; an unreachable server needs three. That is not timidity about network errors, it is that the two failures have different futures: a wrong password stays wrong, a dropped Wi-Fi does not. A red dot the user learns to ignore is worse than no red dot.

**What "green" means after this.** Checked within the last fifteen minutes — or proven by a real tool call more recently than that. The dashboard does not say so, deliberately; the account detail's "checked …" line is where the question gets an answer.
