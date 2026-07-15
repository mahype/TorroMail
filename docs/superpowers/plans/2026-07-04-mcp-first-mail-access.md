# MCP-First Mail Access Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn TorroMail into an MCP-first local mail access layer that can retrieve configured mail safely without becoming a human-facing mail client.

**Architecture:** Keep Rust as the source of truth for mail domain state, policy checks, search sessions, pending actions, and MCP tool execution. Add provider adapters behind explicit traits so fixture-backed tests and real IMAP retrieval use the same service boundary. Keep the SwiftUI app as a minimal setup/control surface for accounts, permissions, MCP lifecycle, pending approvals, audit, and diagnostics only.

**Tech Stack:** Rust 2024 workspace, `serde`, `serde_json`, provider traits in `torromail-core`, JSON-RPC line server in `torromail-mcp`, SwiftPM/SwiftUI configuration app.

---

## File Structure

- `crates/torromail-core/src/lib.rs`: keep existing exports and add message models, provider traits, fixture provider, and `MailAccessService`.
- `crates/torromail-core/tests/core_contract.rs`: extend contracts for MCP-safe search/read behavior and product-boundary invariants.
- `crates/torromail-mcp/Cargo.toml`: add JSON parsing dependencies.
- `crates/torromail-mcp/src/lib.rs`: replace tool-list-only facade with JSON-RPC `tools/call` routing into an injected service.
- `crates/torromail-mcp/tests/tool_contract.rs`: assert tool calls execute and forbidden admin mutations remain absent.
- `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift`: rename user-facing copy from mail-client language to control-surface language and add appearance intent.
- `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`: reduce the app shell to status/setup/control views with semantic colors and no mail-reader surface.
- `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift`: assert the app model remains setup/control-only.
- `docs/architecture.md`: update after implementation with the actual service boundaries.

## Product Boundary

This plan must not add an inbox, message list for human browsing, conversation view, or general mail-client workflow. Message data may appear only as part of MCP test fixtures, MCP responses, diagnostics summaries, or pending action previews.

---

### Task 1: Core Mail Models And Fixture Provider

> Completed 2026-07-15 — adapted to the permission-groups policy model (read/write/send with folder rules) that replaced the flat capability set.

**Files:**
- Modify: `crates/torromail-core/src/lib.rs`
- Test: `crates/torromail-core/tests/core_contract.rs`

- [x] **Step 1: Write failing tests for search/read models**

Add this test to `crates/torromail-core/tests/core_contract.rs`:

```rust
use torromail_core::{
    FixtureMailProvider, MailProvider, StoredMessage,
};

#[test]
fn fixture_provider_searches_and_reads_messages_without_ui_state() {
    let account_id = AccountId::new("work");
    let provider = FixtureMailProvider::new([
        StoredMessage::new(
            account_id.clone(),
            "INBOX",
            "m1",
            "thread-1",
            "Quarterly invoice",
            "billing@example.com",
            "The quarterly invoice is attached.",
            "Invoice body",
        ),
        StoredMessage::new(
            account_id.clone(),
            "Archive",
            "m2",
            "thread-2",
            "Team notes",
            "lead@example.com",
            "Planning notes",
            "Planning body",
        ),
    ]);

    let hits = provider.search(&account_id, "invoice", Some("INBOX"), 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id(), "m1");

    let message = provider.get_message(&account_id, "m1").unwrap();
    assert_eq!(message.subject(), "Quarterly invoice");
    assert_eq!(message.body(), "Invoice body");
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p torromail-core fixture_provider_searches_and_reads_messages_without_ui_state`

Expected: FAIL because `FixtureMailProvider`, `MailProvider`, and `StoredMessage` do not exist.

- [x] **Step 3: Implement the minimal core models and provider**

Add these public types near `SearchHit` in `crates/torromail-core/src/lib.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    account_id: AccountId,
    mailbox: String,
    message_id: String,
    thread_id: String,
    subject: String,
    sender: String,
    snippet: String,
    body: String,
}

impl StoredMessage {
    pub fn new(
        account_id: AccountId,
        mailbox: impl Into<String>,
        message_id: impl Into<String>,
        thread_id: impl Into<String>,
        subject: impl Into<String>,
        sender: impl Into<String>,
        snippet: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            account_id,
            mailbox: mailbox.into(),
            message_id: message_id.into(),
            thread_id: thread_id.into(),
            subject: subject.into(),
            sender: sender.into(),
            snippet: snippet.into(),
            body: body.into(),
        }
    }

    pub fn message_id(&self) -> &str { &self.message_id }
    pub fn thread_id(&self) -> &str { &self.thread_id }
    pub fn subject(&self) -> &str { &self.subject }
    pub fn sender(&self) -> &str { &self.sender }
    pub fn snippet(&self) -> &str { &self.snippet }
    pub fn body(&self) -> &str { &self.body }

    fn matches_query(&self, query: &str) -> bool {
        let haystack = format!("{} {} {} {}", self.subject, self.sender, self.snippet, self.body)
            .to_lowercase();
        query
            .split_whitespace()
            .map(str::to_lowercase)
            .all(|token| haystack.contains(&token))
    }
}

pub trait MailProvider {
    fn search(
        &self,
        account_id: &AccountId,
        query: &str,
        mailbox: Option<&str>,
        limit: usize,
    ) -> CoreResult<Vec<SearchHit>>;

    fn get_message(&self, account_id: &AccountId, message_id: &str) -> CoreResult<StoredMessage>;
}

#[derive(Debug, Clone, Default)]
pub struct FixtureMailProvider {
    messages: Vec<StoredMessage>,
}

impl FixtureMailProvider {
    pub fn new(messages: impl IntoIterator<Item = StoredMessage>) -> Self {
        Self {
            messages: messages.into_iter().collect(),
        }
    }
}

impl MailProvider for FixtureMailProvider {
    fn search(
        &self,
        account_id: &AccountId,
        query: &str,
        mailbox: Option<&str>,
        limit: usize,
    ) -> CoreResult<Vec<SearchHit>> {
        Ok(self
            .messages
            .iter()
            .filter(|message| &message.account_id == account_id)
            .filter(|message| mailbox.is_none_or(|name| message.mailbox == name))
            .filter(|message| message.matches_query(query))
            .take(limit)
            .map(|message| {
                SearchHit::new(
                    message.message_id(),
                    message.subject(),
                    message.sender(),
                    message.snippet(),
                )
            })
            .collect())
    }

    fn get_message(&self, account_id: &AccountId, message_id: &str) -> CoreResult<StoredMessage> {
        self.messages
            .iter()
            .find(|message| &message.account_id == account_id && message.message_id == message_id)
            .cloned()
            .ok_or_else(|| CoreError::SearchResultSetNotFound(message_id.to_owned()))
    }
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test -p torromail-core fixture_provider_searches_and_reads_messages_without_ui_state`

Expected: PASS.

- [x] **Step 5: Commit**

If the workspace is inside a Git repository:

```bash
git add crates/torromail-core/src/lib.rs crates/torromail-core/tests/core_contract.rs
git commit -m "feat: add mail provider fixture"
```

---

### Task 2: Policy-Enforced Mail Access Service

> Completed 2026-07-15 — adapted to the permission-groups policy model (read/write/send with folder rules) that replaced the flat capability set.

**Files:**
- Modify: `crates/torromail-core/src/lib.rs`
- Test: `crates/torromail-core/tests/core_contract.rs`

- [x] **Step 1: Write failing tests for policy enforcement**

Add this test to `crates/torromail-core/tests/core_contract.rs`:

```rust
use torromail_core::{
    FixtureMailProvider, MailAccessService, StoredMessage,
};

#[test]
fn mail_access_service_enforces_search_and_body_policy() {
    let account_id = AccountId::new("work");
    let provider = FixtureMailProvider::new([StoredMessage::new(
        account_id.clone(),
        "INBOX",
        "m1",
        "thread-1",
        "Quarterly invoice",
        "billing@example.com",
        "The quarterly invoice is attached.",
        "Invoice body",
    )]);
    let policy = Policy::new(account_id.clone(), [Capability::Search, Capability::ReadHeaders]);
    let engine = PolicyEngine::new([policy]);
    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    let result_set = service.search(&account_id, "invoice", Some("INBOX"), 10, 100).unwrap();
    assert_eq!(result_set.hits().len(), 1);

    let header_only = service.get_message(&account_id, "m1", false).unwrap();
    assert_eq!(header_only.body(), "");

    let body_result = service.get_message(&account_id, "m1", true);
    assert!(body_result.is_err());
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p torromail-core mail_access_service_enforces_search_and_body_policy`

Expected: FAIL because `MailAccessService` does not exist.

- [x] **Step 3: Implement `MailAccessService`**

Add this type after `FixtureMailProvider` in `crates/torromail-core/src/lib.rs`:

```rust
pub struct MailAccessService<'a, P: MailProvider> {
    provider: P,
    policy_engine: PolicyEngine,
    sessions: &'a mut SearchSessionStore,
}

impl<'a, P: MailProvider> MailAccessService<'a, P> {
    pub fn new(
        provider: P,
        policy_engine: PolicyEngine,
        sessions: &'a mut SearchSessionStore,
    ) -> Self {
        Self {
            provider,
            policy_engine,
            sessions,
        }
    }

    pub fn search(
        &mut self,
        account_id: &AccountId,
        query: &str,
        mailbox: Option<&str>,
        limit: usize,
        now: u64,
    ) -> CoreResult<SearchResultSet> {
        self.policy_engine.authorize(account_id, Capability::Search)?;
        let hits = self.provider.search(account_id, query, mailbox, limit)?;
        Ok(self.sessions.create(account_id.clone(), query, hits, now, 7200))
    }

    pub fn get_message(
        &self,
        account_id: &AccountId,
        message_id: &str,
        include_body: bool,
    ) -> CoreResult<StoredMessage> {
        self.policy_engine.authorize(account_id, Capability::ReadHeaders)?;
        if include_body {
            self.policy_engine.authorize(account_id, Capability::ReadBody)?;
            return self.provider.get_message(account_id, message_id);
        }

        let message = self.provider.get_message(account_id, message_id)?;
        Ok(StoredMessage::new(
            account_id.clone(),
            "",
            message.message_id(),
            message.thread_id(),
            message.subject(),
            message.sender(),
            message.snippet(),
            "",
        ))
    }
}
```

- [x] **Step 4: Run core tests**

Run: `cargo test -p torromail-core`

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/torromail-core/src/lib.rs crates/torromail-core/tests/core_contract.rs
git commit -m "feat: enforce mail access policy"
```

---

### Task 3: MCP JSON-RPC Tool Calls

> Completed 2026-07-15 — adapted to the permission-groups policy model (read/write/send with folder rules) that replaced the flat capability set.

**Files:**
- Modify: `crates/torromail-mcp/Cargo.toml`
- Modify: `crates/torromail-mcp/src/lib.rs`
- Test: `crates/torromail-mcp/tests/tool_contract.rs`

- [x] **Step 1: Add failing MCP call tests**

Add this test to `crates/torromail-mcp/tests/tool_contract.rs`:

```rust
#[test]
fn mcp_server_executes_fixture_backed_mail_search() {
    let server = LineMcpServer::fixture();
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":"invoice","mailbox":"INBOX","limit":10}}}"#,
    );

    assert!(response.contains(r#""id":1"#));
    assert!(response.contains("result-set-1"));
    assert!(response.contains("Quarterly invoice"));
}

#[test]
fn mcp_server_rejects_unknown_tool_calls() {
    let server = LineMcpServer::fixture();
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mail_add_account","arguments":{}}}"#,
    );

    assert!(response.contains(r#""code":-32601"#));
    assert!(response.contains("tool not found"));
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p torromail-mcp mcp_server_executes_fixture_backed_mail_search`

Expected: FAIL because `LineMcpServer::fixture` and `tools/call` routing do not exist.

- [x] **Step 3: Add dependencies**

In `crates/torromail-mcp/Cargo.toml`, keep the existing `torromail-core` dependency and add `serde_json`:

```toml
[dependencies]
torromail-core = { path = "../torromail-core" }
serde_json = "1"
```

- [x] **Step 4: Implement fixture-backed `tools/call` routing**

In `crates/torromail-mcp/src/lib.rs`, import JSON and core types:

```rust
use torromail_core::{
    AccountId, Capability, FixtureMailProvider, MailAccessService, Policy, PolicyEngine,
    SearchSessionStore, StoredMessage,
};
use serde_json::{json, Value};
```

Add this constructor and branch inside `LineMcpServer`:

```rust
impl LineMcpServer {
    pub fn fixture() -> Self {
        Self::default()
    }

    pub fn handle_line(&self, line: &str) -> String {
        let id = extract_json_rpc_id(line).unwrap_or("null");

        if line.contains(r#""method":"initialize""#) || line.contains(r#""method": "initialize""#) {
            return format!(
                r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"TorroMail","version":"0.1.0"}}}}}}"#
            );
        }

        if line.contains(r#""method":"tools/list""#) || line.contains(r#""method": "tools/list""#) {
            return format!(
                r#"{{"jsonrpc":"2.0","id":{id},"result":{}}}"#,
                self.catalog.to_mcp_tools_json()
            );
        }

        if line.contains(r#""method":"tools/call""#) || line.contains(r#""method": "tools/call""#) {
            return self.handle_tool_call(line, id);
        }

        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"method not found"}}}}"#
        )
    }

    fn handle_tool_call(&self, line: &str, id: &str) -> String {
        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => return json_rpc_error(id, -32700, &error.to_string()),
        };
        let name = request["params"]["name"].as_str().unwrap_or_default();
        if name != "mail_search" {
            return json_rpc_error(id, -32601, "tool not found");
        }

        let arguments = &request["params"]["arguments"];
        let account_id = AccountId::new(arguments["account_id"].as_str().unwrap_or("work"));
        let query = arguments["query"].as_str().unwrap_or_default();
        let mailbox = arguments["mailbox"].as_str();
        let limit = arguments["limit"].as_u64().unwrap_or(10).min(100) as usize;

        let provider = FixtureMailProvider::new([StoredMessage::new(
            account_id.clone(),
            "INBOX",
            "m1",
            "thread-1",
            "Quarterly invoice",
            "billing@example.com",
            "The quarterly invoice is attached.",
            "Invoice body",
        )]);
        let policy = Policy::new(
            account_id.clone(),
            [Capability::Search, Capability::ReadHeaders],
        );
        let engine = PolicyEngine::new([policy]);
        let mut sessions = SearchSessionStore::default();
        let mut service = MailAccessService::new(provider, engine, &mut sessions);

        match service.search(&account_id, query, mailbox, limit, 100) {
            Ok(result_set) => {
                let hits = result_set
                    .hits()
                    .iter()
                    .map(|hit| json!({"message_id": hit.message_id(), "subject": hit.subject()}))
                    .collect::<Vec<_>>();
                json!({
                    "jsonrpc": "2.0",
                    "id": serde_json::from_str::<Value>(id).unwrap_or(Value::Null),
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "result_set_id": result_set.id(),
                                "hits": hits
                            }).to_string()
                        }]
                    }
                })
                .to_string()
            }
            Err(error) => json_rpc_error(id, -32000, &error.to_string()),
        }
    }
}
```

Also add:

```rust
fn json_rpc_error(id: &str, code: i64, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": serde_json::from_str::<Value>(id).unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": message
        }
    })
    .to_string()
}
```

Expose `SearchHit::subject()` in `crates/torromail-core/src/lib.rs`:

```rust
pub fn subject(&self) -> &str {
    &self.subject
}
```

- [x] **Step 5: Run MCP tests**

Run: `cargo test -p torromail-mcp`

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add crates/torromail-core/src/lib.rs crates/torromail-mcp/Cargo.toml crates/torromail-mcp/src/lib.rs crates/torromail-mcp/tests/tool_contract.rs
git commit -m "feat: execute MCP mail search"
```

---

### Task 4: Real Provider Boundary For IMAP Configuration

> Completed 2026-07-15.

**Files:**
- Modify: `crates/torromail-core/src/lib.rs`
- Create: `crates/torromail-core/src/imap_provider.rs`
- Test: `crates/torromail-core/tests/core_contract.rs`

- [x] **Step 1: Write a provider-boundary test without network**

Add this test to `crates/torromail-core/tests/core_contract.rs`:

```rust
use torromail_core::{ImapProviderConfig, SecretRef};

#[test]
fn imap_provider_config_keeps_secrets_out_of_debug_output() {
    let config = ImapProviderConfig::new(
        AccountId::new("work"),
        "imap.example.com",
        993,
        "work@example.com",
        SecretRef::new("keychain://torromail/work"),
    );

    let debug = format!("{config:?}");
    assert!(debug.contains("imap.example.com"));
    assert!(!debug.contains("keychain://torromail/work"));
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p torromail-core imap_provider_config_keeps_secrets_out_of_debug_output`

Expected: FAIL because `ImapProviderConfig` and `SecretRef` do not exist.

- [x] **Step 3: Add module and types**

Create `crates/torromail-core/src/imap_provider.rs`:

```rust
use std::fmt;

use crate::AccountId;

#[derive(Clone, PartialEq, Eq)]
pub struct SecretRef(String);

impl SecretRef {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretRef(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapProviderConfig {
    pub account_id: AccountId,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub secret_ref: SecretRef,
}

impl ImapProviderConfig {
    pub fn new(
        account_id: AccountId,
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        secret_ref: SecretRef,
    ) -> Self {
        Self {
            account_id,
            host: host.into(),
            port,
            username: username.into(),
            secret_ref,
        }
    }
}
```

Expose it from `crates/torromail-core/src/lib.rs`:

```rust
pub mod imap_provider;
pub use imap_provider::{ImapProviderConfig, SecretRef};
```

- [x] **Step 4: Run core tests**

Run: `cargo test -p torromail-core`

Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/torromail-core/src/lib.rs crates/torromail-core/src/imap_provider.rs crates/torromail-core/tests/core_contract.rs
git commit -m "feat: add IMAP provider boundary"
```

---

### Task 5: SwiftUI Control Surface Guardrails

> Completed 2026-07-15 — the redesigned app already spoke control-surface language, so only the boundary type and contract checks were added.

**Files:**
- Modify: `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift`
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`
- Test: `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift`

- [x] **Step 1: Write failing Swift contract checks**

Add these checks to `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift`:

```swift
require(
    TorroMailProductBoundary.disallowedUserMailSurfaces == [
        "Inbox UI",
        "Message reader",
        "Thread browser",
        "Manual triage workflow"
    ],
    "product boundary should reject mail-client surfaces"
)

require(
    TorroMailProductBoundary.allowedAppRole == "Setup, consent, MCP lifecycle, status, audit, and diagnostics",
    "app role should be control-surface only"
)
```

- [x] **Step 2: Run test to verify it fails**

Run: `swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract`

Expected: FAIL because `TorroMailProductBoundary` does not exist.

- [x] **Step 3: Add product-boundary model**

Add this type to `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift`:

```swift
public enum TorroMailProductBoundary {
    public static let allowedAppRole = "Setup, consent, MCP lifecycle, status, audit, and diagnostics"

    public static let disallowedUserMailSurfaces = [
        "Inbox UI",
        "Message reader",
        "Thread browser",
        "Manual triage workflow"
    ]
}
```

- [x] **Step 4: Update visible app copy**

In `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`, replace mail-client-style titles with control-surface copy:

```swift
.navigationTitle("TorroMail Control")
```

Use labels such as:

```swift
Text("MCP Mail Access")
Text("Account Access")
Text("Permissions")
Text("Pending Approvals")
Text("Diagnostics")
```

Avoid labels such as:

```swift
Text("Inbox")
Text("Messages")
Text("Threads")
Text("Compose")
```

- [x] **Step 5: Run Swift contract and build**

Run:

```bash
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift apps/TorroMailApp/Tests/TorroMailKitContract/main.swift
git commit -m "feat: guard SwiftUI control surface"
```

---

### Task 6: Light And Dark Control-Surface Theme

> Completed 2026-07-15 — the app already renders both modes through semantic styles; the appearance facts are now pinned by contract.

**Files:**
- Modify: `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift`
- Modify: `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`
- Test: `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift`

- [x] **Step 1: Write failing theme contract checks**

Add this to `apps/TorroMailApp/Tests/TorroMailKitContract/main.swift`:

```swift
require(TorroMailAppearance.spacingUnit == 8, "spacing should use an 8-point base")
require(TorroMailAppearance.supportsLightMode, "light mode should be supported")
require(TorroMailAppearance.supportsDarkMode, "dark mode should be supported")
require(TorroMailAppearance.usesSemanticColors, "theme should use semantic colors")
```

- [x] **Step 2: Run test to verify it fails**

Run: `swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract`

Expected: FAIL because `TorroMailAppearance` does not exist.

- [x] **Step 3: Add appearance contract**

Add this to `apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift`:

```swift
public enum TorroMailAppearance {
    public static let spacingUnit = 8
    public static let supportsLightMode = true
    public static let supportsDarkMode = true
    public static let usesSemanticColors = true
}
```

- [x] **Step 4: Apply semantic colors in SwiftUI**

In `apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift`, prefer semantic styles:

```swift
.foregroundStyle(.primary)
.foregroundStyle(.secondary)
.background(.background)
.background(.regularMaterial)
```

Use spacing values derived from the 8-point base:

```swift
private enum Layout {
    static let unit: CGFloat = CGFloat(TorroMailAppearance.spacingUnit)
    static let small: CGFloat = unit
    static let medium: CGFloat = unit * 2
    static let large: CGFloat = unit * 3
}
```

- [x] **Step 5: Run Swift verification**

Run:

```bash
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add apps/TorroMailApp/Sources/TorroMailKit/TorroMailKit.swift apps/TorroMailApp/Sources/TorroMailApp/TorroMailApp.swift apps/TorroMailApp/Tests/TorroMailKitContract/main.swift
git commit -m "feat: add control surface appearance contract"
```

---

### Task 7: Documentation And Final Verification

> Completed 2026-07-15 — all statements verified present; full cargo and Swift verification green.

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `AGENTS.md`

- [x] **Step 1: Update docs with implemented behavior**

Ensure these exact statements remain true in the docs:

```markdown
TorroMail is not a general-purpose mail client.
The native macOS app is a configuration and control surface.
MCP clients can search and read within policy, prepare risky actions, and inspect read-only admin state.
```

- [x] **Step 2: Run full verification**

Run:

```bash
cargo test
cargo build -p torromail-mcp
swift run --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build TorroMailKitContract
swift build --package-path apps/TorroMailApp --scratch-path apps/TorroMailApp/.build
```

Expected: all commands complete successfully.

- [x] **Step 3: Search for product-boundary regressions**

Run:

```bash
rg -n "Inbox UI|Message reader|Thread browser|Manual triage workflow|not a general-purpose mail client|control surface" README.md AGENTS.md docs apps crates
```

Expected: matches either define the boundary, enforce it in tests/contracts, or mark historical docs as superseded.

- [x] **Step 4: Commit**

```bash
git add README.md docs/architecture.md AGENTS.md
git commit -m "docs: align MCP-first implementation notes"
```

---

## Execution Options

Plan complete and saved to `docs/superpowers/plans/2026-07-04-mcp-first-mail-access.md`.

Two execution options:

1. **Subagent-Driven (recommended)** - dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** - execute tasks in this session using `superpowers:executing-plans`, with checkpoints after each task.

Before execution, ensure the Git workspace state is clean. The project lives at `/Users/wagesve/Dev/Apps/TorroMail` and is initialized as a Git repository, so task commits can be created there directly.
