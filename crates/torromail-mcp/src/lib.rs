//! TorroMail MCP facade.
//!
//! This crate keeps the public MCP surface explicit. Account setup, secret
//! changes, OAuth setup, and permission edits stay on the GUI/IPC side.

pub mod health;
mod keychain;
mod policy_document;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use torromail_core::smtp::SmtpAuth;
use torromail_core::{
    AccountId, Capability, CoreError, CoreResult, FixtureMailProvider, ImapAuth,
    ImapProviderConfig, MailAccessService, MailProvider, MarkChange, OutgoingAttachment,
    PermissionSet, Policy, PolicyEngine, ReadAccess, SearchSessionStore, SearchWindow,
    StoredMessage,
};
use torromail_oauth::TokenSet;

// `health` itself is this file's own module, already in scope.
use crate::health::{HealthOutcome, HealthThrottle};
use crate::policy_document::{
    DocumentAccount, DocumentClient, OAuthFacts, ParsedDocument, parse_policy_document,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    #[default]
    Stdio,
    LocalHttp,
    UnixSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolName {
    MailSearch,
    MailRefineSearch,
    MailGetMessage,
    MailGetThread,
    MailListMailboxes,
    MailMark,
    MailCreateDraft,
    MailPrepareSend,
    MailPrepareMove,
    MailPrepareDelete,
    MailConfirmAction,
    MailListAccounts,
    MailGetPolicy,
    MailGetCacheStatus,
}

impl ToolName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MailSearch => "mail_search",
            Self::MailRefineSearch => "mail_refine_search",
            Self::MailGetMessage => "mail_get_message",
            Self::MailGetThread => "mail_get_thread",
            Self::MailListMailboxes => "mail_list_mailboxes",
            Self::MailMark => "mail_mark",
            Self::MailCreateDraft => "mail_create_draft",
            Self::MailPrepareSend => "mail_prepare_send",
            Self::MailPrepareMove => "mail_prepare_move",
            Self::MailPrepareDelete => "mail_prepare_delete",
            Self::MailConfirmAction => "mail_confirm_action",
            Self::MailListAccounts => "mail_list_accounts",
            Self::MailGetPolicy => "mail_get_policy",
            Self::MailGetCacheStatus => "mail_get_cache_status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessLevel {
    Read,
    /// Writes without a GUI confirmation loop — low-risk mailbox mutations
    /// such as marking, still gated by the account's permissions.
    DirectWrite,
    PrepareAction,
    ConfirmPreparedAction,
    ReadOnlyAdmin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDescriptor {
    name: ToolName,
    description: &'static str,
    access_level: AccessLevel,
    requires_gui_confirmation: bool,
}

impl ToolDescriptor {
    pub fn name(&self) -> ToolName {
        self.name
    }

    pub fn name_str(&self) -> &'static str {
        self.name.as_str()
    }

    pub fn access_level(&self) -> AccessLevel {
        self.access_level
    }

    pub fn requires_gui_confirmation(&self) -> bool {
        self.requires_gui_confirmation
    }

    fn input_schema_json(&self) -> &'static str {
        match self.name {
            ToolName::MailSearch => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"query":{"type":"string","description":"Full-text query. Leave empty for the newest messages, no filter."},"mailbox":{"type":"string","description":"Defaults to every readable mailbox; an empty query with no date defaults to INBOX."},"since":{"type":"string","description":"Only messages on or after this ISO date (YYYY-MM-DD)."},"before":{"type":"string","description":"Only messages before this ISO date (YYYY-MM-DD)."},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["account_id"]}"#
            }
            ToolName::MailRefineSearch => {
                r#"{"type":"object","properties":{"result_set_id":{"type":"string"},"refinement":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["result_set_id","refinement"]}"#
            }
            ToolName::MailGetMessage => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"message_id":{"type":"string"},"include_body":{"type":"boolean"}},"required":["account_id","message_id"]}"#
            }
            ToolName::MailGetThread => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"thread_id":{"type":"string"},"include_bodies":{"type":"boolean"}},"required":["account_id","thread_id"]}"#
            }
            ToolName::MailListMailboxes => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"}},"required":["account_id"]}"#
            }
            ToolName::MailMark => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"message_ids":{"type":"array","items":{"type":"string"},"description":"IDs from mail_search; each already carries its mailbox."},"mark":{"type":"string","enum":["seen","unseen","flagged","unflagged"]}},"required":["account_id","message_ids","mark"]}"#
            }
            ToolName::MailCreateDraft => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"to":{"type":"array","items":{"type":"string"}},"cc":{"type":"array","items":{"type":"string"}},"bcc":{"type":"array","items":{"type":"string"}},"subject":{"type":"string"},"body":{"type":"string"},"attachments":{"type":"array","maxItems":20,"description":"Files to attach. Pass bytes as standard padded base64; TorroMail never reads local paths or URLs.","items":{"type":"object","additionalProperties":false,"properties":{"filename":{"type":"string","minLength":1,"maxLength":255},"media_type":{"type":"string","maxLength":127,"description":"IANA media type; defaults to application/octet-stream."},"content_base64":{"type":"string"}},"required":["filename","content_base64"]}}},"required":["account_id","to","subject","body"]}"#
            }
            ToolName::MailPrepareSend => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"draft_id":{"type":"string"}},"required":["account_id","draft_id"]}"#
            }
            ToolName::MailPrepareMove => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"message_ids":{"type":"array","items":{"type":"string"}},"target_mailbox":{"type":"string"}},"required":["account_id","message_ids","target_mailbox"]}"#
            }
            ToolName::MailPrepareDelete => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"message_ids":{"type":"array","items":{"type":"string"}},"permanent":{"type":"boolean","default":false}},"required":["account_id","message_ids"]}"#
            }
            ToolName::MailConfirmAction => {
                r#"{"type":"object","properties":{"pending_action_id":{"type":"string"},"confirmation_code":{"type":"string"}},"required":["pending_action_id","confirmation_code"]}"#
            }
            ToolName::MailListAccounts => r#"{"type":"object","properties":{}}"#,
            ToolName::MailGetPolicy => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"}},"required":["account_id"]}"#
            }
            ToolName::MailGetCacheStatus => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"}}}"#
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCatalog {
    tools: Vec<ToolDescriptor>,
}

impl Default for ToolCatalog {
    fn default() -> Self {
        Self {
            tools: canonical_tools(),
        }
    }
}

impl ToolCatalog {
    pub fn tools(&self) -> &[ToolDescriptor] {
        &self.tools
    }

    pub fn find(&self, name: ToolName) -> Option<&ToolDescriptor> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    pub fn tool_names(&self) -> Vec<ToolName> {
        self.tools.iter().map(ToolDescriptor::name).collect()
    }

    pub fn names_as_str(&self) -> Vec<&'static str> {
        self.tools.iter().map(ToolDescriptor::name_str).collect()
    }

    pub fn to_mcp_tools_json(&self) -> String {
        let tools = self
            .tools
            .iter()
            .map(tool_descriptor_json)
            .collect::<Vec<_>>()
            .join(",");

        format!(r#"{{"tools":[{tools}]}}"#)
    }
}

/// One account's live connection, kept between tool calls. The identity is
/// what it was built for; if the document later describes a different
/// configuration, the stored session no longer matches and is rebuilt.
struct PooledConnection {
    identity: ConnIdentity,
    provider: Box<dyn MailProvider>,
}

/// What a pooled connection was opened for. Reusing one across calls is only
/// safe while this still matches the document — a changed host, user or auth
/// means a different mailbox.
#[derive(PartialEq, Eq)]
enum ConnIdentity {
    Fixture,
    Configured(ImapProviderConfig),
}

/// How a connection is opened, so tests can supply a scripted mailbox in
/// place of a TLS session. Takes only the account id — the production path
/// resolves the rest from the reloaded document itself.
type ConnectOverride = Box<dyn Fn(&AccountId) -> CoreResult<Box<dyn MailProvider>>>;

const MAX_ATTACHMENTS: usize = 20;
const MAX_MESSAGE_BYTES: usize = 20 * 1024 * 1024;

/// Metadata safe to show in an approval or audit line. File bytes remain only
/// in the raw MIME message and never enter either surface.
#[derive(Clone)]
struct AttachmentSummary {
    filename: String,
    media_type: String,
    size_bytes: usize,
}

/// A composed draft the server remembers so a later `prepare_send` can find
/// it by id. The recipients are the full envelope — to, cc and bcc — while
/// the raw message carries only the visible headers.
#[derive(Clone)]
struct DraftRecord {
    account_id: AccountId,
    from: String,
    recipients: Vec<String>,
    subject: String,
    attachments: Vec<AttachmentSummary>,
    raw: String,
}

/// Drafts composed this session, by id.
#[derive(Default)]
struct DraftCache {
    next_id: u64,
    records: HashMap<String, DraftRecord>,
}

impl DraftCache {
    fn insert(&mut self, record: DraftRecord) -> String {
        self.next_id += 1;
        let id = format!("draft-{}", self.next_id);
        self.records.insert(id.clone(), record);
        id
    }

    fn get(&self, id: &str) -> Option<DraftRecord> {
        self.records.get(id).cloned()
    }
}

/// The mutation a prepared action will carry out once confirmed.
enum Operation {
    Move {
        message_ids: Vec<String>,
        target: String,
    },
    Trash {
        message_ids: Vec<String>,
    },
    Expunge {
        message_ids: Vec<String>,
    },
    Send {
        record: DraftRecord,
    },
}

/// A risky action held between `prepare` and `confirm`. The code is the
/// handshake that ties a confirmation back to exactly this preparation; the
/// GUI is meant to be where a human reads the preview and approves.
struct PreparedAction {
    account_id: AccountId,
    code: String,
    operation: Operation,
    expires_at: u64,
}

/// Prepared actions by id, with a single-use, time-bounded confirm. Kept in
/// the server so a `prepare` and its `confirm` can be different tool calls.
#[derive(Default)]
struct PendingActions {
    next_id: u64,
    actions: HashMap<String, PreparedAction>,
}

impl PendingActions {
    /// Store an action and return its id and confirmation code.
    fn prepare(&mut self, mut action: PreparedAction) -> (String, String) {
        self.next_id += 1;
        let id = format!("pending-{}", self.next_id);
        let code = confirmation_code(&id);
        action.code = code.clone();
        self.actions.insert(id.clone(), action);
        (id, code)
    }

    /// Validate a confirmation and take the action out — single use. A wrong
    /// code is the caller's mistake; a missing or expired action is not.
    fn confirm(&mut self, id: &str, code: &str, now: u64) -> Result<PreparedAction, ToolFailure> {
        let (expires_at, stored_code) = {
            let action = self.actions.get(id).ok_or_else(|| {
                ToolFailure::Core(CoreError::PendingActionNotFound(id.to_owned()))
            })?;
            (action.expires_at, action.code.clone())
        };

        if stored_code != code {
            return Err(ToolFailure::InvalidParams(
                "confirmation_code does not match the prepared action".to_owned(),
            ));
        }
        if now > expires_at {
            self.actions.remove(id);
            return Err(ToolFailure::Core(CoreError::PendingActionExpired(id.to_owned())));
        }
        self.actions
            .remove(id)
            .ok_or_else(|| ToolFailure::Core(CoreError::PendingActionNotFound(id.to_owned())))
    }
}

pub struct LineMcpServer {
    catalog: ToolCatalog,
    /// Where the app publishes the account permissions. `None` runs the
    /// product default (read + drafts) for any account — the fixture mode
    /// tests use.
    policy_path: Option<PathBuf>,
    /// Whether an account the document describes but never finished
    /// configuring may fall back to fixture mailboxes. Only ever true for
    /// tests: in the product that fallback would hand an assistant demo
    /// messages and call them the user's mail.
    fixtures_for_unconfigured_accounts: bool,
    /// Live connections kept between tool calls, one per account, so a burst
    /// of calls is one login rather than one login each. `RefCell` because
    /// the stdio server hands out `&self`; single-threaded, so no lock.
    connections: RefCell<HashMap<AccountId, PooledConnection>>,
    /// Search result sets, kept so `mail_refine_search` can narrow a prior
    /// search by its id without rescanning every mailbox.
    sessions: RefCell<SearchSessionStore>,
    /// Risky actions awaiting confirmation, between their prepare and confirm.
    pending: RefCell<PendingActions>,
    /// Drafts composed this session, so `prepare_send` can name one by id.
    drafts: RefCell<DraftCache>,
    /// The last health outcome recorded per account, so the throttle on the
    /// tool-call path costs a map lookup rather than a read of the whole log.
    /// `RefCell` for the same reason as `connections`.
    health_throttle: RefCell<HealthThrottle>,
    /// SHA-256 of the access key the spawning client presented via
    /// `TORROMAIL_TOKEN` — hashed once at startup, the plaintext is not kept.
    /// Checked per tool call against the document's `clients` allowlist, so
    /// a key revoked in the app locks its client out mid-session.
    presented_token_hash: Option<String>,
    /// Test seam: when set, this makes the mailbox instead of a TLS session.
    connect_override: Option<ConnectOverride>,
    /// Sibling of the policy file: every tool call appends one JSONL line here
    /// so the app can show what assistants have been doing. `None` in fixture
    /// mode — a test run without a policy path has no shared place to write,
    /// and no activity worth keeping.
    audit_path: Option<PathBuf>,
    /// Sibling of the policy file: every `initialize` handshake from a paired
    /// client appends one JSONL line here, so the app can prove a client
    /// actually connected — not merely that its config points here. `None` in
    /// fixture mode, for the same reason as `audit_path`.
    connections_path: Option<PathBuf>,
    /// Sibling of the policy file: every tool call that touches a mailbox
    /// appends one JSONL line here, so the app's status dot reflects what
    /// actually happens rather than what happened once at setup. `None` in
    /// fixture mode, for the same reason as `audit_path`.
    health_path: Option<PathBuf>,
}

impl LineMcpServer {
    /// A server wired to fixture data — the same policy-checked path real
    /// providers will use, minus the network.
    pub fn fixture() -> Self {
        Self {
            catalog: ToolCatalog::default(),
            policy_path: None,
            fixtures_for_unconfigured_accounts: false,
            connections: RefCell::default(),
            sessions: RefCell::default(),
            pending: RefCell::default(),
            drafts: RefCell::default(),
            health_throttle: RefCell::default(),
            presented_token_hash: None,
            connect_override: None,
            audit_path: None,
            connections_path: None,
            health_path: None,
        }
    }

    /// The access key the client that spawned this process presented, or
    /// `None` when it presented nothing. Enforcement is decided by the
    /// policy document, not here — see `client_gate`.
    #[must_use]
    pub fn with_presented_token(mut self, token: Option<&str>) -> Self {
        self.presented_token_hash = token.map(sha256_hex);
        self
    }

    /// A server that enforces the policy document at `path`, reloading it on
    /// every tool call so permission changes in the app apply immediately.
    pub fn with_policy_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        // The logs live beside the policy document, in the app's shared
        // support folder — the one place both the server and the app agree on.
        let audit_path = path.parent().map(|dir| dir.join("audit.jsonl"));
        let connections_path = path.parent().map(|dir| dir.join("connections.jsonl"));
        let health_path = path.parent().map(|dir| dir.join("health.jsonl"));
        Self {
            policy_path: Some(path),
            audit_path,
            connections_path,
            health_path,
            ..Self::fixture()
        }
    }

    /// Test-only: a policy document *and* fixture mailboxes, so the permission
    /// plumbing can be exercised without an IMAP server. The product must
    /// never take this door — an account with no connection facts has no mail,
    /// and the honest answer is an error.
    pub fn with_policy_path_and_fixtures(path: impl Into<PathBuf>) -> Self {
        Self {
            fixtures_for_unconfigured_accounts: true,
            ..Self::with_policy_path(path)
        }
    }

    /// Test-only: a server whose connections come from `connect` rather than a
    /// TLS handshake, so pooling and reconnection can be exercised without a
    /// network. `connect` is called once per real open — reuse from the pool
    /// never calls it.
    #[doc(hidden)]
    pub fn with_connect_override<F>(path: impl Into<PathBuf>, fixtures: bool, connect: F) -> Self
    where
        F: Fn(&AccountId) -> CoreResult<Box<dyn MailProvider>> + 'static,
    {
        Self {
            fixtures_for_unconfigured_accounts: fixtures,
            connect_override: Some(Box::new(connect)),
            ..Self::with_policy_path(path)
        }
    }

    /// One JSON-RPC line in, at most one line out. `None` means "say
    /// nothing": notifications carry no id and must never be answered, and a
    /// line we cannot parse has no id to answer to either.
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let request: Value = serde_json::from_str(line).ok()?;
        let id = match request.get("id") {
            Some(id) if !id.is_null() => id.clone(),
            _ => return None,
        };

        Some(match request["method"].as_str().unwrap_or_default() {
            "initialize" => {
                // The handshake is the first — and for a client that only
                // lists tools, the only — proof it actually reached the server.
                // Record it so the app can show a real connection, not just a
                // config file that points here.
                self.record_connection(&request);
                json_rpc_result(
                    &id,
                    &json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "TorroMail", "version": "0.1.0"}
                    }),
                )
            }
            "tools/list" => format!(
                r#"{{"jsonrpc":"2.0","id":{id},"result":{}}}"#,
                self.catalog.to_mcp_tools_json()
            ),
            "tools/call" => {
                let response = self.handle_tool_call(&request, &id);
                self.record_audit(&request, &response);
                response
            }
            _ => json_rpc_error(&id, -32601, "method not found"),
        })
    }

    fn handle_tool_call(&self, request: &Value, id: &Value) -> String {
        // The pairing check guards every tool, the read-only admin ones
        // included — they name accounts and their permissions, which is
        // exactly what an unpaired process has no business seeing.
        match self.client_gate() {
            Ok(ClientGate::Allowed) => {}
            Ok(ClientGate::Refused(message)) => return json_rpc_error(id, -32001, message),
            Err(message) => return json_rpc_error(id, -32000, &message),
        }

        let name = request["params"]["name"].as_str().unwrap_or_default();
        if !self.catalog.names_as_str().contains(&name) {
            return json_rpc_error(id, -32601, "tool not found");
        }

        let arguments = &request["params"]["arguments"];

        // The admin reads answer from the policy document alone. They run
        // before any account lookup on purpose: naming an account is what
        // `mail_list_accounts` is for, so requiring one here would leave a
        // fresh client with no way in.
        if name == ToolName::MailListAccounts.as_str() {
            return self.handle_mail_list_accounts(id);
        }
        if name == ToolName::MailGetCacheStatus.as_str() {
            return self.handle_mail_get_cache_status(arguments, id);
        }

        // Refining works on a stored result set, not a live mailbox: the id
        // is the key, and the set already passed the policy at search time.
        if name == ToolName::MailRefineSearch.as_str() {
            return self.handle_mail_refine_search(arguments, id);
        }

        // Confirming names a prepared action, not an account — the account
        // rides along with the action it was prepared for.
        if name == ToolName::MailConfirmAction.as_str() {
            return self.handle_mail_confirm_action(arguments, id);
        }

        let account_id = AccountId::new(arguments["account_id"].as_str().unwrap_or_default());
        if name == ToolName::MailGetPolicy.as_str() {
            return self.handle_mail_get_policy(&account_id, id);
        }
        if name == ToolName::MailPrepareMove.as_str() {
            return self.handle_mail_prepare_move(arguments, &account_id, id);
        }
        if name == ToolName::MailPrepareDelete.as_str() {
            return self.handle_mail_prepare_delete(arguments, &account_id, id);
        }

        if name == ToolName::MailSearch.as_str() {
            return self.run_with_connection(&account_id, id, &|provider, engine| {
                let mut sessions = self.sessions.borrow_mut();
                handle_mail_search(arguments, &account_id, provider, engine, &mut sessions)
            });
        }
        if name == ToolName::MailGetMessage.as_str() {
            return self.run_with_connection(&account_id, id, &|provider, engine| {
                handle_mail_get_message(arguments, &account_id, provider, engine)
            });
        }
        if name == ToolName::MailGetThread.as_str() {
            return self.run_with_connection(&account_id, id, &|provider, engine| {
                handle_mail_get_thread(arguments, &account_id, provider, engine)
            });
        }
        if name == ToolName::MailCreateDraft.as_str() {
            let from = self.account_email(&account_id).unwrap_or_default();
            return self.run_with_connection(&account_id, id, &|provider, engine| {
                let (mailbox, record) =
                    compose_and_append(arguments, &account_id, provider, engine, &from)?;
                let attachments = attachment_summaries_json(&record.attachments);
                let attachment_count = record.attachments.len();
                let total_attachment_bytes = total_attachment_bytes(&record.attachments);
                let message_bytes = record.raw.len();
                let draft_id = self.drafts.borrow_mut().insert(record);
                Ok(json!({
                    "status": "draft_created",
                    "draft_id": draft_id,
                    "mailbox": mailbox,
                    "attachments": attachments,
                    "attachment_count": attachment_count,
                    "total_attachment_bytes": total_attachment_bytes,
                    "message_bytes": message_bytes
                }))
            });
        }
        if name == ToolName::MailPrepareSend.as_str() {
            return self.handle_mail_prepare_send(arguments, &account_id, id);
        }
        if name == ToolName::MailMark.as_str() {
            return self.run_with_connection(&account_id, id, &|provider, engine| {
                handle_mail_mark(arguments, &account_id, provider, engine)
            });
        }
        if name == ToolName::MailListMailboxes.as_str() {
            return self.run_with_connection(&account_id, id, &|provider, engine| {
                handle_mail_list_mailboxes(&account_id, provider, engine)
            });
        }

        json_rpc_error(id, -32000, "tool not implemented yet")
    }

    /// Append one line describing a finished tool call, so the app's log and
    /// activity view have something real to show. Best-effort by design: a
    /// mail action must never fail because its audit line could not be written,
    /// so every error here is swallowed.
    fn record_audit(&self, request: &Value, response: &str) {
        let Some(path) = &self.audit_path else {
            return;
        };
        let name = request["params"]["name"].as_str().unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let arguments = &request["params"]["arguments"];
        let account = arguments["account_id"].as_str().unwrap_or_default();
        // A JSON-RPC error object is the only failure shape; anything else the
        // dispatch produced is a result the tool meant to return.
        let parsed = serde_json::from_str::<Value>(response).ok();
        let failed = parsed
            .as_ref()
            .is_some_and(|value| value.get("error").is_some());
        // The tool's own payload rides as a JSON string inside the MCP text
        // content — where a confirmed action's executed counts live.
        let payload = parsed.as_ref().and_then(audit_payload);
        let detail = audit_detail(name, arguments, payload.as_ref());
        let (client_id, client_name) = self.client_identity();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let line = json!({
            "ts": ts,
            "client_id": client_id,
            "client": client_name,
            "account": account,
            "tool": name,
            "detail": detail,
            "result": if failed { "error" } else { "ok" },
        })
        .to_string();

        // Append mode is atomic per write on the platforms we run, so parallel
        // clients writing their own processes' lines never interleave.
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes());
        }
    }

    /// Append one health record for `account_id`. Best-effort like the audit
    /// log — a mail action must never fail because its health line could not
    /// be written.
    ///
    /// Repeated successes are dropped: `audit.jsonl` already holds the
    /// transcript of every call, and a second "still fine" line thirty seconds
    /// after the first says nothing while crowding other accounts out of the
    /// log. Failures always land.
    fn record_health(&self, account_id: &AccountId, outcome: HealthOutcome, detail: &str) {
        let Some(path) = &self.health_path else {
            return;
        };
        let account = account_id.as_str();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        // Bound to a local so the borrow ends here rather than spanning the
        // append below.
        let admitted = self
            .health_throttle
            .borrow_mut()
            .admit(account, outcome, now);
        if !admitted {
            return;
        }
        health::append(path, account, outcome, "tool-call", detail);
    }

    /// Append one line noting that a paired client completed the `initialize`
    /// handshake — the "last connected" the app cannot learn any other way, and
    /// the half of "is it set up?" that only the client itself can prove.
    /// Best-effort like the audit log: a handshake must never fail because its
    /// record could not be written. Only paired clients are recorded; an
    /// unknown key resolves to no identity and leaves no trace, so the signal
    /// stays trustworthy — a line here means a client the app minted a key for
    /// really connected.
    fn record_connection(&self, request: &Value) {
        let Some(path) = &self.connections_path else {
            return;
        };
        let (client_id, client_name) = self.client_identity();
        // The neutral fallback from `client_identity` — an unpaired or
        // unrecognised key. Recording it would claim a connection we cannot
        // attribute, so we stay silent.
        if client_id == "unknown" {
            return;
        }
        let params = &request["params"];
        let protocol = params["protocolVersion"].as_str().unwrap_or_default();
        let reported_name = params["clientInfo"]["name"].as_str().unwrap_or_default();
        let reported_version = params["clientInfo"]["version"].as_str().unwrap_or_default();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let line = json!({
            "ts": ts,
            "client_id": client_id,
            "client": client_name,
            "protocol_version": protocol,
            "client_name": reported_name,
            "client_version": reported_version,
        })
        .to_string();

        // Append mode is atomic per write on the platforms we run, so parallel
        // clients handshaking never interleave their lines. The app derives
        // each client's latest connection by scanning for the newest line.
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes());
        }
    }

    /// Which paired client is calling, resolved from the token it presented
    /// against the document's allowlist — labels for attribution, not a right.
    /// Falls back to a neutral label when there is no allowlist (fixture data)
    /// or the key matches no entry.
    fn client_identity(&self) -> (String, String) {
        let unknown = || ("unknown".to_owned(), "Unknown".to_owned());
        let Some(hash) = &self.presented_token_hash else {
            return unknown();
        };
        let Ok(Some(document)) = self.document() else {
            return unknown();
        };
        let Some(clients) = document.clients else {
            return unknown();
        };
        clients
            .into_iter()
            .find(|client| &client.token_sha256 == hash)
            .map_or_else(unknown, |client| (client.id, client.name))
    }

    /// Run one mail operation against a pooled connection. The connection is
    /// reused across calls; if it went stale — the server dropped it while we
    /// idled — the first command fails, and this rebuilds it and tries once
    /// more. A logical refusal (policy, missing message) is not a connection
    /// fault and is returned as-is.
    fn run_with_connection(
        &self,
        account_id: &AccountId,
        id: &Value,
        run: &dyn Fn(&mut dyn MailProvider, PolicyEngine) -> ToolResult,
    ) -> String {
        let mut rebuilt = false;
        loop {
            let (engine, facts) = match self.runtime_for(account_id) {
                Ok(runtime) => runtime,
                Err(message) => return json_rpc_error(id, -32000, &message),
            };
            // Refused here, ahead of every recording site below, because an
            // account with no connection has nothing to say about its own
            // health — see `unconfigured`.
            let Some(identity) = ConnIdentity::from_facts(&facts) else {
                return json_rpc_error(id, -32000, &unconfigured(account_id).to_string());
            };

            let mut pool = self.connections.borrow_mut();
            let stale = pool
                .get(account_id)
                .is_none_or(|connection| connection.identity != identity);
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
            let connection = pool
                .get_mut(account_id)
                .expect("a connection was just ensured");

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
        }
    }

    /// Open a real connection, or defer to the test override when one is set.
    fn open_connection(
        &self,
        account_id: &AccountId,
        facts: ConnectionFacts,
    ) -> CoreResult<Box<dyn MailProvider>> {
        match &self.connect_override {
            Some(connect) => connect(account_id),
            None => open_provider(account_id, facts),
        }
    }

    /// Prepare a move for confirmation. The account-wide right is checked now,
    /// so a forbidden move is refused before anyone is asked to approve it; the
    /// folder-scoped check runs again at execution.
    fn handle_mail_prepare_move(&self, arguments: &Value, account_id: &AccountId, id: &Value) -> String {
        let message_ids = string_array(&arguments["message_ids"]);
        let target = arguments["target_mailbox"].as_str().unwrap_or_default();
        if message_ids.is_empty() {
            return json_rpc_error(id, -32602, "message_ids must not be empty");
        }
        if target.is_empty() {
            return json_rpc_error(id, -32602, "target_mailbox is required");
        }

        let engine = match self.runtime_for(account_id) {
            Ok((engine, _facts)) => engine,
            Err(message) => return json_rpc_error(id, -32000, &message),
        };
        if let Err(error) = engine.authorize(account_id, Capability::Move) {
            return json_rpc_error(id, -32000, &error.to_string());
        }

        let preview = format!("Move {} message(s) to {target}", message_ids.len());
        self.store_prepared(
            account_id,
            preview,
            Operation::Move {
                message_ids,
                target: target.to_owned(),
            },
            id,
        )
    }

    /// Prepare a delete for confirmation — a soft delete (a move to Trash) or,
    /// with `permanent`, an outright expunge. Each is gated by its own right.
    fn handle_mail_prepare_delete(
        &self,
        arguments: &Value,
        account_id: &AccountId,
        id: &Value,
    ) -> String {
        let message_ids = string_array(&arguments["message_ids"]);
        let permanent = arguments["permanent"].as_bool().unwrap_or(false);
        if message_ids.is_empty() {
            return json_rpc_error(id, -32602, "message_ids must not be empty");
        }

        let engine = match self.runtime_for(account_id) {
            Ok((engine, _facts)) => engine,
            Err(message) => return json_rpc_error(id, -32000, &message),
        };
        let capability = if permanent {
            Capability::DeletePermanent
        } else {
            Capability::DeleteSoft
        };
        if let Err(error) = engine.authorize(account_id, capability) {
            return json_rpc_error(id, -32000, &error.to_string());
        }

        let (preview, operation) = if permanent {
            (
                format!("Permanently delete {} message(s)", message_ids.len()),
                Operation::Expunge { message_ids },
            )
        } else {
            (
                format!("Move {} message(s) to Trash", message_ids.len()),
                Operation::Trash { message_ids },
            )
        };
        self.store_prepared(account_id, preview, operation, id)
    }

    /// Prepare a send for confirmation. The draft must have been composed this
    /// session, and the account must hold the send right — the one capability
    /// that acts on the outside world.
    fn handle_mail_prepare_send(
        &self,
        arguments: &Value,
        account_id: &AccountId,
        id: &Value,
    ) -> String {
        let draft_id = arguments["draft_id"].as_str().unwrap_or_default();
        let Some(record) = self.drafts.borrow().get(draft_id) else {
            return json_rpc_error(id, -32000, &format!("draft not found: {draft_id}"));
        };
        if &record.account_id != account_id {
            return json_rpc_error(id, -32000, "draft belongs to a different account");
        }

        let engine = match self.runtime_for(account_id) {
            Ok((engine, _facts)) => engine,
            Err(message) => return json_rpc_error(id, -32000, &message),
        };
        if let Err(error) = engine.authorize(account_id, Capability::Send) {
            return json_rpc_error(id, -32000, &error.to_string());
        }

        let preview = if record.attachments.is_empty() {
            format!("Send to {}", record.recipients.join(", "))
        } else {
            format!(
                "Send to {} with {} attachment(s)",
                record.recipients.join(", "),
                record.attachments.len()
            )
        };
        self.store_prepared(account_id, preview, Operation::Send { record }, id)
    }

    /// Store a prepared action and answer with its id, code and preview.
    fn store_prepared(
        &self,
        account_id: &AccountId,
        preview: String,
        operation: Operation,
        id: &Value,
    ) -> String {
        let send_details = match &operation {
            Operation::Send { record } => Some(json!({
                "subject": record.subject,
                "attachments": attachment_summaries_json(&record.attachments),
                "attachment_count": record.attachments.len(),
                "total_attachment_bytes": total_attachment_bytes(&record.attachments)
            })),
            _ => None,
        };
        let action = PreparedAction {
            account_id: account_id.clone(),
            code: String::new(),
            operation,
            expires_at: now_secs() + PENDING_TTL_SECONDS,
        };
        let (pending_id, code) = self.pending.borrow_mut().prepare(action);
        let mut payload = json!({
            "pending_action_id": pending_id,
            "confirmation_code": code,
            "preview": preview,
            "expires_in_seconds": PENDING_TTL_SECONDS
        });
        if let Some(details) = send_details {
            payload["subject"] = details["subject"].clone();
            payload["attachments"] = details["attachments"].clone();
            payload["attachment_count"] = details["attachment_count"].clone();
            payload["total_attachment_bytes"] = details["total_attachment_bytes"].clone();
        }
        json_rpc_text_result(id, &payload)
    }

    /// Confirm and carry out a prepared action. The action names its own
    /// account, so this opens that account's connection and runs the mutation
    /// through the policy one more time.
    fn handle_mail_confirm_action(&self, arguments: &Value, id: &Value) -> String {
        let pending_id = arguments["pending_action_id"].as_str().unwrap_or_default();
        let code = arguments["confirmation_code"].as_str().unwrap_or_default();

        let action = match self.pending.borrow_mut().confirm(pending_id, code, now_secs()) {
            Ok(action) => action,
            Err(failure) => return failure.into_response(id),
        };
        let account_id = action.account_id.clone();

        // Sending leaves the house over SMTP, not the pooled IMAP connection;
        // every other action mutates the mailbox and runs on that connection.
        match &action.operation {
            Operation::Send { record } => self.execute_send(&account_id, record, id),
            operation => self.run_with_connection(&account_id, id, &|provider, engine| {
                execute_operation(operation, &account_id, provider, engine)
            }),
        }
    }

    /// Submit a confirmed draft over SMTP. A fresh session each time — sends
    /// are rare enough that pooling a second connection is not worth it.
    fn execute_send(
        &self,
        account_id: &AccountId,
        record: &DraftRecord,
        id: &Value,
    ) -> String {
        if &record.account_id != account_id {
            return json_rpc_error(id, -32000, "draft belongs to a different account");
        }
        let engine = match self.runtime_for(account_id) {
            Ok((engine, _facts)) => engine,
            Err(message) => return json_rpc_error(id, -32000, &message),
        };
        if let Err(error) = engine.authorize(account_id, Capability::Send) {
            return json_rpc_error(id, -32000, &error.to_string());
        }
        let Some((config, oauth)) = self.smtp_facts(account_id) else {
            return json_rpc_error(
                id,
                -32000,
                "no SMTP is configured for this account — finish setting it up in TorroMail",
            );
        };

        match send_over_smtp(&config, oauth.as_ref(), record) {
            Ok(()) => json_rpc_text_result(
                id,
                &json!({
                    "status": "sent",
                    "recipients": record.recipients.len(),
                    "attachment_count": record.attachments.len(),
                    "total_attachment_bytes": total_attachment_bytes(&record.attachments)
                }),
            ),
            Err(error) => json_rpc_error(id, -32000, &error.to_string()),
        }
    }

    /// The SMTP connection facts for an account, and the OAuth facts that go
    /// with them — read from the document, reloaded per call.
    fn smtp_facts(&self, account_id: &AccountId) -> Option<(ImapProviderConfig, Option<OAuthFacts>)> {
        let accounts = self.document_accounts().ok().flatten()?;
        let account = accounts
            .into_iter()
            .find(|account| account.policy.account_id() == account_id)?;
        account.smtp.map(|config| (config, account.oauth))
    }

    /// The account's own address, for the `From` of a draft. Read from the
    /// document, which is the only place it lives.
    fn account_email(&self, account_id: &AccountId) -> Option<String> {
        let accounts = self.document_accounts().ok().flatten()?;
        accounts
            .iter()
            .find(|account| account.policy.account_id() == account_id)
            .map(|account| account.email.clone())
    }

    /// Narrow a prior search by its id. The stored hits already passed the
    /// policy when the search ran, and refining only removes hits, so this
    /// needs neither a connection nor a fresh authorization.
    fn handle_mail_refine_search(&self, arguments: &Value, id: &Value) -> String {
        let result_set_id = arguments["result_set_id"].as_str().unwrap_or_default();
        let refinement = arguments["refinement"].as_str().unwrap_or_default();

        let mut sessions = self.sessions.borrow_mut();
        match sessions.refine(result_set_id, refinement, 100) {
            Ok(result_set) => json_rpc_text_result(id, &result_set_payload(&result_set)),
            Err(error) => json_rpc_error(id, -32000, &error.to_string()),
        }
    }

    /// The policy document, reloaded per call. `None` means there is no
    /// document at all — fixture mode, or an app that has not published yet.
    fn document(&self) -> Result<Option<ParsedDocument>, String> {
        let Some(path) = &self.policy_path else {
            return Ok(None);
        };

        match std::fs::read_to_string(path) {
            Ok(text) => parse_policy_document(&text).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("policy document unreadable: {error}")),
        }
    }

    /// The accounts the document describes. An empty list is a different
    /// answer than `None`: a document that names no accounts grants nothing.
    fn document_accounts(&self) -> Result<Option<Vec<DocumentAccount>>, String> {
        Ok(self.document()?.map(|document| document.accounts))
    }

    /// Whether the client that spawned this process may call tools. Three
    /// verdicts: allowed, refused with a message the assistant can relay to
    /// its human, or `Err` when the document itself is unreadable.
    ///
    /// No document, or a document without a `clients` key, enforces nothing —
    /// the first serves fixture data only, the second predates pairing and
    /// heals the moment the app republishes. A published allowlist admits
    /// exactly the keys it names.
    fn client_gate(&self) -> Result<ClientGate, String> {
        let Some(document) = self.document()? else {
            return Ok(ClientGate::Allowed);
        };
        let Some(clients) = document.clients else {
            return Ok(ClientGate::Allowed);
        };

        Ok(match &self.presented_token_hash {
            None => ClientGate::Refused(NOT_PAIRED),
            Some(presented) => {
                if is_paired(&clients, presented) {
                    ClientGate::Allowed
                } else {
                    ClientGate::Refused(KEY_REJECTED)
                }
            }
        })
    }

    /// The policy engine and connection facts for this call. With a policy
    /// path the document decides — reloaded every time, failing closed when
    /// unreadable, refusing accounts it does not contain. Without one, the
    /// product default over fixture data.
    fn runtime_for(
        &self,
        account_id: &AccountId,
    ) -> Result<(PolicyEngine, ConnectionFacts), String> {
        let Some(accounts) = self.document_accounts()? else {
            return Ok((default_engine(account_id), ConnectionFacts::FixtureMode));
        };

        // An account the document does not name is refused here, before any
        // connection is opened. The engine would refuse it too, but only after
        // `connect_provider` had already run a TLS handshake and a LOGIN for a
        // mailbox this call was never allowed to touch.
        let Some(account) = accounts
            .iter()
            .find(|account| account.policy.account_id() == account_id)
        else {
            return Err(CoreError::AccountNotFound(account_id.clone()).to_string());
        };

        // A document exists, so an account without connection facts is an
        // unfinished account — never a reason to reach for the fixtures.
        let facts = match &account.imap {
            Some(config) => {
                ConnectionFacts::Configured(Box::new(config.clone()), account.oauth.clone())
            }
            None if self.fixtures_for_unconfigured_accounts => ConnectionFacts::FixtureMode,
            None => ConnectionFacts::Unconfigured,
        };
        let engine = PolicyEngine::new(accounts.into_iter().map(|account| account.policy));
        Ok((engine, facts))
    }

    /// The way in: every other tool needs an `account_id`, and this is the
    /// only place one can be learned.
    fn handle_mail_list_accounts(&self, id: &Value) -> String {
        let accounts = match self.document_accounts() {
            Ok(accounts) => accounts.unwrap_or_default(),
            Err(message) => return json_rpc_error(id, -32000, &message),
        };

        let accounts = accounts
            .iter()
            .map(|account| {
                json!({
                    "account_id": account.policy.account_id().as_str(),
                    "name": account.name,
                    "email": account.email,
                    "connected": account.imap.is_some(),
                    "permissions": permissions_json(account.policy.permissions())
                })
            })
            .collect::<Vec<_>>();

        json_rpc_text_result(id, &json!({ "accounts": accounts }))
    }

    fn handle_mail_get_policy(&self, account_id: &AccountId, id: &Value) -> String {
        let accounts = match self.document_accounts() {
            Ok(accounts) => accounts.unwrap_or_default(),
            Err(message) => return json_rpc_error(id, -32000, &message),
        };
        let Some(account) = accounts
            .iter()
            .find(|account| account.policy.account_id() == account_id)
        else {
            return json_rpc_error(
                id,
                -32000,
                &CoreError::AccountNotFound(account_id.clone()).to_string(),
            );
        };

        // Both halves, because they answer different questions: the switches
        // as the user set them, and what those switches actually permit.
        json_rpc_text_result(
            id,
            &json!({
                "account_id": account_id.as_str(),
                "permissions": permissions_json(account.policy.permissions()),
                "capabilities": capabilities_json(&account.policy)
            }),
        )
    }

    /// Cache facts for one account, or all of them when none is named —
    /// "what is TorroMail keeping on disk?" is a fair question to ask whole.
    fn handle_mail_get_cache_status(&self, arguments: &Value, id: &Value) -> String {
        let accounts = match self.document_accounts() {
            Ok(accounts) => accounts.unwrap_or_default(),
            Err(message) => return json_rpc_error(id, -32000, &message),
        };
        let wanted = arguments["account_id"].as_str();

        let reports = accounts
            .iter()
            .filter(|account| wanted.is_none_or(|id| account.policy.account_id().as_str() == id))
            .map(|account| {
                json!({
                    "account_id": account.policy.account_id().as_str(),
                    "local_cache_enabled": account.cache.local_cache_enabled,
                    "mode": account.cache.mode,
                    "index_bodies": account.cache.index_bodies,
                    "index_attachments": account.cache.index_attachments,
                    "storage": account.cache.storage
                })
            })
            .collect::<Vec<_>>();

        if let Some(wanted) = wanted
            && reports.is_empty()
        {
            return json_rpc_error(
                id,
                -32000,
                &CoreError::AccountNotFound(AccountId::new(wanted)).to_string(),
            );
        }

        json_rpc_text_result(id, &json!({ "accounts": reports }))
    }
}

fn permissions_json(permissions: &PermissionSet) -> Value {
    let write = permissions.write.sanitized();
    json!({
        "read": read_access_name(permissions.read),
        "write": {
            "drafts": write.drafts,
            "mark": write.mark,
            "move": write.move_messages,
            "trash": write.trash,
            "permanent_delete": write.permanent_delete
        },
        "send": permissions.send,
        "per_folder": permissions.per_folder,
        "folder_rules": permissions
            .folder_rules
            .iter()
            .map(|(mailbox, rule)| {
                (mailbox.clone(), json!({"read": rule.read, "write": rule.write}))
            })
            .collect::<serde_json::Map<_, _>>()
    })
}

/// The account-wide verdicts, named the way tools ask for them. Folder rules
/// can still say no to any of these in a given mailbox.
fn capabilities_json(policy: &Policy) -> Value {
    let capabilities = [
        (Capability::Search, "search"),
        (Capability::ReadHeaders, "read_headers"),
        (Capability::ReadBody, "read_body"),
        (Capability::DownloadAttachments, "download_attachments"),
        (Capability::Draft, "draft"),
        (Capability::Send, "send"),
        (Capability::Mark, "mark"),
        (Capability::Move, "move"),
        (Capability::DeleteSoft, "delete_soft"),
        (Capability::DeletePermanent, "delete_permanent"),
    ];

    capabilities
        .into_iter()
        .filter(|(capability, _)| policy.allows(*capability))
        .map(|(_, name)| Value::from(name))
        .collect()
}

fn read_access_name(read: ReadAccess) -> &'static str {
    match read {
        ReadAccess::None => "none",
        ReadAccess::Headers => "headers",
        ReadAccess::FullMessage => "full_message",
        ReadAccess::WithAttachments => "with_attachments",
    }
}

/// Where one account's mail comes from on this call.
///
/// The distinction between `FixtureMode` and `Unconfigured` is the whole point:
/// both used to arrive as a bare `None` and both got fixture data, which meant
/// an account whose setup was never finished served demo messages to the
/// assistant as if they were the user's mail.
enum ConnectionFacts {
    /// No policy document at all — nothing has been published, so the fixture
    /// mailbox is the product default rather than a stand-in for real mail.
    FixtureMode,
    /// The document describes this account's connection. `oauth` rides along
    /// for `xoauth2` accounts — the token in the keychain may need renewing
    /// before it can be used.
    Configured(Box<ImapProviderConfig>, Option<OAuthFacts>),
    /// The document knows the account but carries no connection facts for it.
    /// Setup is unfinished; there is no mail to serve and saying so is the
    /// only honest answer.
    Unconfigured,
}

impl ConnIdentity {
    /// The identity a pool entry must still match to be reused, or `None` when
    /// there is no connection to key at all. The caller turns that into the
    /// refusal: what an unfinished account means is a judgement about the
    /// account, not about the pool.
    fn from_facts(facts: &ConnectionFacts) -> Option<Self> {
        match facts {
            ConnectionFacts::FixtureMode => Some(Self::Fixture),
            ConnectionFacts::Configured(config, _) => Some(Self::Configured((**config).clone())),
            ConnectionFacts::Unconfigured => None,
        }
    }
}

/// What an account the wizard never finished has instead of a connection: a
/// sentence the assistant can relay to the person who has to act on it.
///
/// Every caller must refuse *before* it would record health. This failure is a
/// setup gap, not a health signal — it happens on every call by construction,
/// so three of them in a row would turn the dot red and fire a desktop
/// notification at a user who has simply not finished the wizard.
fn unconfigured(account_id: &AccountId) -> CoreError {
    CoreError::ProviderFailure(format!(
        "account {account_id} has no connection configured — finish setting it up in TorroMail"
    ))
}

/// Opens a real mailbox when the document has connection facts for the
/// account. The caller keeps the result in the pool; this is the expensive
/// step — a TLS handshake and a LOGIN — that pooling exists to avoid
/// repeating.
fn open_provider(
    account_id: &AccountId,
    facts: ConnectionFacts,
) -> CoreResult<Box<dyn MailProvider>> {
    match facts {
        ConnectionFacts::Configured(config, oauth) => {
            let secret = resolve_credential(&config, oauth.as_ref())?;
            let provider = torromail_imap_tls::connect_account(&config, &secret)?;
            Ok(Box::new(provider))
        }
        ConnectionFacts::FixtureMode => Ok(Box::new(fixture_provider(account_id))),
        // Callers refuse this before they get here; kept honest rather than
        // assumed away, and it must stay unrecorded wherever it does surface.
        ConnectionFacts::Unconfigured => Err(unconfigured(account_id)),
    }
}

/// The secret to hand the IMAP session: a password as stored, or a bearer
/// token that is good for right now.
///
/// The renewal has to happen here rather than in the app. MCP clients spawn
/// this binary themselves, so it must cope with a token that went stale while
/// TorroMail was closed — an hour is the whole life of a Google access token.
fn resolve_credential(
    config: &ImapProviderConfig,
    oauth: Option<&OAuthFacts>,
) -> CoreResult<String> {
    let stored = keychain::resolve_secret(&config.secret_ref)?;

    match config.auth {
        ImapAuth::Password => Ok(stored),
        ImapAuth::XOAuth2 => {
            let facts = oauth.ok_or_else(|| {
                CoreError::ProviderFailure(
                    "an xoauth2 account needs a token endpoint and client id".to_owned(),
                )
            })?;
            let tokens = TokenSet::from_json(&stored)
                .map_err(|error| CoreError::ProviderFailure(error.to_string()))?;

            if tokens.is_usable_at(torromail_oauth::now_unix()) {
                return Ok(tokens.access_token);
            }

            let fresh = torromail_oauth::refresh(&facts.token_endpoint, &facts.client_id, &tokens)
                .map_err(|error| CoreError::ProviderFailure(error.to_string()))?;
            // Store before use: a token this process fetched but never wrote
            // back would be fetched again by the next one, and a rotated
            // refresh token would be lost outright.
            keychain::store_secret(&config.secret_ref, &fresh.to_json())?;
            Ok(fresh.access_token)
        }
    }
}

/// The connection check behind the app's "Test Connection" button and the
/// `--check-account` flag: resolve the secret, log in over TLS, count the
/// mailboxes. No mail content is touched.
///
/// Answers with the verdict as well as the words for it, because the two have
/// different readers: the message is for whoever asked, and the outcome is
/// what a status dot is made of — `Rejected` turns it red at once, while
/// `Unreachable` earns a grace period first.
///
/// Writes nothing. What a check means for the log is the caller's call: the
/// same check serves a user standing at a button and the sweep below, and only
/// one of them is entitled to speak for the account afterwards.
#[must_use]
pub fn check_account(
    account_id: &str,
    policy_path: Option<PathBuf>,
    presented_token: Option<&str>,
) -> (HealthOutcome, String) {
    match check_account_inner(account_id, policy_path, presented_token) {
        Ok(summary) => (HealthOutcome::Ok, summary),
        Err(failure) => failure,
    }
}

/// The check proper, split out so every early return names its own outcome and
/// [`check_account`] does nothing but unwrap it.
fn check_account_inner(
    account_id: &str,
    policy_path: Option<PathBuf>,
    presented_token: Option<&str>,
) -> Result<String, (HealthOutcome, String)> {
    let path = policy_path.ok_or_else(|| unreachable("no policy document path available"))?;
    let text = std::fs::read_to_string(&path)
        .map_err(|error| unreachable(format!("policy document unreadable: {error}")))?;
    let document = parse_policy_document(&text).map_err(unreachable)?;

    // The check logs into the real mailbox, so it sits behind the same
    // pairing gate as the tools. The app passes its own key; a foreign
    // process invoking the flag gets the same refusal a tool call would.
    pairing_gate(document.clients.as_deref(), presented_token).map_err(unreachable)?;

    // Found by the account's own id, not by whether it has a connection, so an
    // account whose setup is unfinished is told apart from one nobody has ever
    // heard of. The caller needs that difference: only one of the two is worth
    // asking about again.
    let account = document
        .accounts
        .iter()
        .find(|account| account.policy.account_id().as_str() == account_id)
        .ok_or_else(|| {
            unreachable(CoreError::AccountNotFound(AccountId::new(account_id)).to_string())
        })?;
    let config = account
        .imap
        .as_ref()
        .ok_or_else(|| unreachable(unconfigured(account.policy.account_id()).to_string()))?;

    // The same credential path the tools take, token renewal included — so a
    // green dot in the app means an assistant would get in too. From here on
    // the mailbox itself is answering, so its failures classify.
    let secret =
        resolve_credential(config, account.oauth.as_ref()).map_err(|error| classify(&error))?;
    let provider =
        torromail_imap_tls::connect_account(config, &secret).map_err(|error| classify(&error))?;
    let mailboxes = provider
        .list_mailboxes(&config.account_id)
        .map_err(|error| classify(&error))?;

    Ok(format!("login ok, {} mailboxes visible", mailboxes.len()))
}

/// A failure that happened before the mailbox got a word in: a missing
/// document, a refused pairing, an account nobody configured. Never
/// `Rejected` — none of these is the credentials, and the app must not accuse
/// the password for something the password would not fix.
fn unreachable(message: impl Into<String>) -> (HealthOutcome, String) {
    (HealthOutcome::Unreachable, message.into())
}

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

/// Whether the caller may make this server log into a real mailbox. The
/// document's allowlist is the whole rule, and it is enforced identically for
/// a tool call, a check and a sweep — see `client_gate` for the same decision
/// on the stdio path.
fn pairing_gate(
    clients: Option<&[DocumentClient]>,
    presented_token: Option<&str>,
) -> Result<(), &'static str> {
    let Some(clients) = clients else {
        return Ok(());
    };
    match presented_token.map(sha256_hex) {
        None => Err(NOT_PAIRED),
        Some(presented) if is_paired(clients, &presented) => Ok(()),
        Some(_) => Err(KEY_REJECTED),
    }
}

/// Check every configured account once as the server comes up, skipping any
/// checked in the last [`health::SERVER_START_WINDOW`] seconds. Without the
/// debounce, five paired clients each spawning their own server process would
/// mean five logins per account on every launch.
///
/// This is the only thing that proves an account when the app is closed: an
/// MCP client can start this server on its own, and until the first tool call
/// touches a mailbox nothing else would ever say whether the login still
/// works.
///
/// Best-effort and silent throughout: a server must start and serve tools
/// whatever the mailboxes are doing, so nothing here is reported and nothing
/// here can fail a request.
///
/// Slow by nature — a TLS handshake and a login per account, with no timeout
/// underneath either. The server runs it on a thread of its own for exactly
/// that reason; see `main.rs`.
pub fn sweep_account_health(policy_path: Option<PathBuf>, presented_token: Option<&str>) {
    // No document is fixture mode: no accounts, nothing to prove, and no
    // folder to write a log into.
    let Some(path) = policy_path else {
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
    // Judged once here rather than per account. An unpaired client's server
    // would otherwise fail every check on the gate — a refusal that says
    // nothing about any mailbox — and write a log full of `unreachable` that
    // takes every dot down with it.
    if pairing_gate(document.clients.as_deref(), presented_token).is_err() {
        return;
    }

    let records = health::load(&health_path);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();

    for account in &document.accounts {
        // An account the wizard never finished has nothing to check, and
        // saying "unreachable" about it would be an alarm about unfinished
        // setup — see `unconfigured`.
        if account.imap.is_none() {
            continue;
        }
        let id = account.policy.account_id().as_str();
        if !health::needs_check(&records, id, now, health::SERVER_START_WINDOW) {
            continue;
        }
        // The document is re-read per account rather than threaded through, so
        // a check is one thing with one definition. It is a small local file
        // and this runs once per server, off the path of every call.
        let (outcome, detail) = check_account(id, Some(path.clone()), presented_token);
        health::append(&health_path, id, outcome, "server-start", &detail);
    }
}

/// The product default until the app has published permissions: read and
/// drafts, nothing else.
fn default_engine(account_id: &AccountId) -> PolicyEngine {
    PolicyEngine::new([Policy::new(account_id.clone(), PermissionSet::default())])
}

/// What the pairing check decided about the client on the other end of stdio.
enum ClientGate {
    Allowed,
    /// The message an assistant relays to its human, so refusal comes with
    /// the way to fix it. `initialize` and `tools/list` stay open on purpose:
    /// the client connects, lists tools, and the first call explains itself —
    /// a connection that fails outright would bury this text in a log.
    Refused(&'static str),
}

const NOT_PAIRED: &str = "This client is not paired with TorroMail. Open TorroMail → MCP Clients, \
     connect this client, then restart it.";
const KEY_REJECTED: &str = "This client's TorroMail access key was revoked or is not valid. Open \
     TorroMail → MCP Clients and reconnect this client, then restart it.";

/// Whether a presented key hash is on the allowlist. Hashes are compared,
/// never tokens: equality on digests of a high-entropy secret leaks nothing
/// a timing probe could grow into a key, so a plain `==` is sound here.
fn is_paired(clients: &[DocumentClient], presented_hash: &str) -> bool {
    clients
        .iter()
        .any(|client| client.token_sha256 == presented_hash)
}

/// Lowercase hex SHA-256 — the shape `token_sha256` carries in the document.
fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"));
    }
    hex
}

/// A tool's result payload, or why it could not be produced. Kept apart from
/// JSON-RPC formatting so the connection pool can tell a stale-session failure
/// (rebuild and retry) from a logical one (report as-is).
type ToolResult = Result<Value, ToolFailure>;

enum ToolFailure {
    /// The client asked for something malformed — a bad flag, a bad date.
    InvalidParams(String),
    /// Everything the domain can refuse or fail at: policy denials, missing
    /// messages, and provider (connection) failures.
    Core(CoreError),
}

impl ToolFailure {
    /// Whether the connection itself is suspect, as opposed to a refusal the
    /// connection reported faithfully.
    fn is_connection(&self) -> bool {
        matches!(self, Self::Core(CoreError::ProviderFailure(_)))
    }

    /// What this failure says about the account's health, if anything. A
    /// malformed request or a policy denial says nothing — the connection was
    /// fine, the answer was simply no.
    fn health_outcome(&self) -> Option<HealthOutcome> {
        match self {
            Self::Core(error) => HealthOutcome::from_error(error),
            Self::InvalidParams(_) => None,
        }
    }

    /// The human-readable reason, readable without consuming the failure.
    fn message(&self) -> String {
        match self {
            Self::InvalidParams(message) => message.clone(),
            Self::Core(error) => error.to_string(),
        }
    }

    fn into_response(self, id: &Value) -> String {
        match self {
            Self::InvalidParams(message) => json_rpc_error(id, -32602, &message),
            Self::Core(error) => json_rpc_error(id, -32000, &error.to_string()),
        }
    }
}

fn handle_mail_search(
    arguments: &Value,
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
    sessions: &mut SearchSessionStore,
) -> ToolResult {
    let query = arguments["query"].as_str().unwrap_or_default();
    let mailbox = arguments["mailbox"].as_str();
    let limit = arguments["limit"].as_u64().unwrap_or(10).min(100) as usize;

    let window = search_window(arguments).map_err(ToolFailure::InvalidParams)?;

    let mut service = MailAccessService::new(provider, engine, sessions);
    let result_set = service
        .search(account_id, query, mailbox, limit, &window, 100)
        .map_err(ToolFailure::Core)?;

    Ok(result_set_payload(&result_set))
}

/// The wire shape of a result set — the same for a fresh search and a
/// refinement, so a client sees one kind of answer.
fn result_set_payload(result_set: &torromail_core::SearchResultSet) -> Value {
    let hits = result_set
        .hits()
        .iter()
        .map(|hit| {
            json!({
                "message_id": hit.message_id(),
                "mailbox": hit.mailbox(),
                "subject": hit.subject(),
                "date": hit.date()
            })
        })
        .collect::<Vec<_>>();
    json!({
        "result_set_id": result_set.id(),
        "hits": hits
    })
}

/// The `since` / `before` arguments as an IMAP-ready date window. Clients
/// speak ISO `YYYY-MM-DD`; IMAP wants `DD-Mon-YYYY`, so the conversion — and
/// the validation that rejects a nonsense date up front — lives here.
fn search_window(arguments: &Value) -> Result<SearchWindow, String> {
    Ok(SearchWindow {
        since: imap_date(arguments, "since")?,
        before: imap_date(arguments, "before")?,
    })
}

fn imap_date(arguments: &Value, field: &str) -> Result<Option<String>, String> {
    let Some(value) = arguments[field].as_str() else {
        return Ok(None);
    };
    to_imap_date(value)
        .map(Some)
        .ok_or_else(|| format!("{field} must be an ISO date like 2026-07-08"))
}

/// `2026-07-08` → `08-Jul-2026`, the only date form IMAP SEARCH accepts.
/// `None` for anything that is not a real calendar date.
fn to_imap_date(iso: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let mut parts = iso.split('-');
    let year: u32 = parts.next()?.parse().ok()?;
    let month: usize = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let name = MONTHS.get(month.checked_sub(1)?)?;
    if !(1..=31).contains(&day) || !(1000..=9999).contains(&year) {
        return None;
    }
    Some(format!("{day:02}-{name}-{year}"))
}

fn handle_mail_get_message(
    arguments: &Value,
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
) -> ToolResult {
    let message_id = arguments["message_id"].as_str().unwrap_or_default();
    let include_body = arguments["include_body"].as_bool().unwrap_or(false);

    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(provider, engine, &mut sessions);

    let message = service
        .get_message(account_id, message_id, include_body)
        .map_err(ToolFailure::Core)?;
    Ok(message_json(&message))
}

/// One message on the wire — the shape `mail_get_message` returns and
/// `mail_get_thread` repeats for each message in the conversation.
fn message_json(message: &StoredMessage) -> Value {
    json!({
        "message_id": message.message_id(),
        "mailbox": message.mailbox(),
        "thread_id": message.thread_id(),
        "subject": message.subject(),
        "sender": message.sender(),
        "date": message.date(),
        "snippet": message.snippet(),
        "body": message.body(),
        "seen": message.seen(),
        "flagged": message.flagged()
    })
}

fn handle_mail_get_thread(
    arguments: &Value,
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
) -> ToolResult {
    let thread_id = arguments["thread_id"].as_str().unwrap_or_default();
    let include_bodies = arguments["include_bodies"].as_bool().unwrap_or(false);

    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(provider, engine, &mut sessions);

    let messages = service
        .get_thread(account_id, thread_id, include_bodies)
        .map_err(ToolFailure::Core)?;
    let messages = messages.iter().map(message_json).collect::<Vec<_>>();
    Ok(json!({ "thread_id": thread_id, "messages": messages }))
}

/// Compose a draft, append it to the drafts folder, and hand back the folder
/// it landed in and a record of it for a later send.
fn compose_and_append(
    arguments: &Value,
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
    from: &str,
) -> Result<(String, DraftRecord), ToolFailure> {
    let to = string_array(&arguments["to"]);
    let cc = string_array(&arguments["cc"]);
    let bcc = string_array(&arguments["bcc"]);
    let subject = arguments["subject"].as_str().unwrap_or_default();
    let body = arguments["body"].as_str().unwrap_or_default();
    let attachments = parse_attachments(&arguments["attachments"])?;

    if to.is_empty() {
        return Err(ToolFailure::InvalidParams(
            "to needs at least one recipient".to_owned(),
        ));
    }

    let attachment_summaries = attachments
        .iter()
        .map(|attachment| AttachmentSummary {
            filename: attachment.filename().to_owned(),
            media_type: attachment.media_type().to_owned(),
            size_bytes: attachment.content().len(),
        })
        .collect::<Vec<_>>();
    let message = torromail_core::compose_message_with_attachments(
        from,
        &to,
        &cc,
        subject,
        body,
        &attachments,
    );
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(ToolFailure::InvalidParams(format!(
            "the finished message is {} bytes; the limit is {MAX_MESSAGE_BYTES} bytes",
            message.len()
        )));
    }

    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    // The drafts folder is wherever the account keeps it — its name is the
    // last path segment, so `INBOX.Drafts` and a plain `Drafts` both match.
    let mailboxes = service.list_mailboxes(account_id).map_err(ToolFailure::Core)?;
    let mailbox = drafts_mailbox(&mailboxes);
    service
        .create_draft(account_id, &mailbox, &message)
        .map_err(ToolFailure::Core)?;

    // The envelope is every recipient; the raw message shows only to and cc.
    let mut recipients = to;
    recipients.extend(cc);
    recipients.extend(bcc);
    let record = DraftRecord {
        account_id: account_id.clone(),
        from: from.to_owned(),
        recipients,
        subject: subject.to_owned(),
        attachments: attachment_summaries,
        raw: message,
    };
    Ok((mailbox, record))
}

/// Parse the only attachment source TorroMail accepts: bytes carried by the
/// paired MCP client. Local paths and URLs deliberately have no schema shape,
/// so the server never becomes a filesystem or network deputy.
fn parse_attachments(value: &Value) -> Result<Vec<OutgoingAttachment>, ToolFailure> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = value
        .as_array()
        .ok_or_else(|| ToolFailure::InvalidParams("attachments must be an array".to_owned()))?;
    if values.len() > MAX_ATTACHMENTS {
        return Err(ToolFailure::InvalidParams(format!(
            "attachments may contain at most {MAX_ATTACHMENTS} files"
        )));
    }

    let mut attachments = Vec::with_capacity(values.len());
    let mut total_bytes = 0_usize;
    for (index, value) in values.iter().enumerate() {
        let number = index + 1;
        let object = value.as_object().ok_or_else(|| {
            ToolFailure::InvalidParams(format!("attachment {number} must be an object"))
        })?;
        if let Some(field) = object
            .keys()
            .find(|field| !matches!(field.as_str(), "filename" | "media_type" | "content_base64"))
        {
            return Err(ToolFailure::InvalidParams(format!(
                "attachment {number} contains unsupported field {field}"
            )));
        }
        let filename = value["filename"].as_str().ok_or_else(|| {
            ToolFailure::InvalidParams(format!("attachment {number} needs a filename"))
        })?;
        validate_attachment_filename(filename).map_err(|message| {
            ToolFailure::InvalidParams(format!("attachment {number}: {message}"))
        })?;

        let media_type = match object.get("media_type") {
            None => "application/octet-stream",
            Some(value) => value.as_str().ok_or_else(|| {
                ToolFailure::InvalidParams(format!(
                    "attachment {number}: media_type must be a string"
                ))
            })?,
        };
        validate_media_type(media_type).map_err(|message| {
            ToolFailure::InvalidParams(format!("attachment {number}: {message}"))
        })?;

        let encoded = value["content_base64"].as_str().ok_or_else(|| {
            ToolFailure::InvalidParams(format!("attachment {number} needs content_base64"))
        })?;
        let content = decode_base64_strict(encoded).map_err(|message| {
            ToolFailure::InvalidParams(format!("attachment {number}: {message}"))
        })?;
        total_bytes = total_bytes.checked_add(content.len()).ok_or_else(|| {
            ToolFailure::InvalidParams("attachment size overflow".to_owned())
        })?;
        if total_bytes > MAX_MESSAGE_BYTES {
            return Err(ToolFailure::InvalidParams(format!(
                "attachment bytes exceed the {MAX_MESSAGE_BYTES}-byte message limit"
            )));
        }

        attachments.push(OutgoingAttachment::new(filename, media_type, content));
    }
    Ok(attachments)
}

fn validate_attachment_filename(filename: &str) -> Result<(), &'static str> {
    if filename.is_empty() || filename.trim().is_empty() {
        return Err("filename must not be empty");
    }
    if filename.len() > 255 {
        return Err("filename must not exceed 255 UTF-8 bytes");
    }
    if filename == "." || filename == ".." || filename.contains(['/', '\\']) {
        return Err("filename must not contain path components");
    }
    if filename.chars().any(char::is_control) {
        return Err("filename must not contain control characters");
    }
    Ok(())
}

fn validate_media_type(media_type: &str) -> Result<(), &'static str> {
    if media_type.len() > 127 || !media_type.is_ascii() {
        return Err("media_type must be at most 127 ASCII characters");
    }
    let Some((kind, subtype)) = media_type.split_once('/') else {
        return Err("media_type must have the form type/subtype");
    };
    if kind.is_empty()
        || subtype.is_empty()
        || subtype.contains('/')
        || !kind.bytes().all(is_mime_token_byte)
        || !subtype.bytes().all(is_mime_token_byte)
    {
        return Err("media_type must be a valid type/subtype without parameters");
    }
    Ok(())
}

fn is_mime_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// Standard padded base64, with ASCII whitespace tolerated for clients that
/// wrap long values. Padding and unused bits are checked so malformed payloads
/// never silently turn into different bytes.
fn decode_base64_strict(input: &str) -> Result<Vec<u8>, &'static str> {
    let compact_len = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .count();
    let maximum_encoded = MAX_MESSAGE_BYTES.saturating_mul(4).div_ceil(3) + 4;
    if compact_len > maximum_encoded {
        return Err("content_base64 is too large");
    }
    if compact_len % 4 != 0 {
        return Err("content_base64 must use standard padded base64");
    }

    let compact = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    let mut decoded = Vec::with_capacity(compact.len() / 4 * 3);
    let chunk_count = compact.len() / 4;
    for (index, chunk) in compact.chunks_exact(4).enumerate() {
        let last = index + 1 == chunk_count;
        let a = base64_sextet(chunk[0]).ok_or("content_base64 contains invalid characters")?;
        let b = base64_sextet(chunk[1]).ok_or("content_base64 contains invalid characters")?;
        decoded.push((a << 2) | (b >> 4));

        if chunk[2] == b'=' {
            if !last || chunk[3] != b'=' || b & 0x0f != 0 {
                return Err("content_base64 has invalid padding");
            }
            continue;
        }
        let c = base64_sextet(chunk[2]).ok_or("content_base64 contains invalid characters")?;
        decoded.push((b << 4) | (c >> 2));

        if chunk[3] == b'=' {
            if !last || c & 0x03 != 0 {
                return Err("content_base64 has invalid padding");
            }
            continue;
        }
        let d = base64_sextet(chunk[3]).ok_or("content_base64 contains invalid characters")?;
        decoded.push((c << 6) | d);
    }
    Ok(decoded)
}

fn base64_sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn attachment_summaries_json(attachments: &[AttachmentSummary]) -> Value {
    Value::Array(
        attachments
            .iter()
            .map(|attachment| {
                json!({
                    "filename": attachment.filename,
                    "media_type": attachment.media_type,
                    "size_bytes": attachment.size_bytes
                })
            })
            .collect(),
    )
}

fn total_attachment_bytes(attachments: &[AttachmentSummary]) -> usize {
    attachments
        .iter()
        .map(|attachment| attachment.size_bytes)
        .sum()
}

/// A JSON array of strings, or an empty list — used for recipient fields.
fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The mailbox a draft belongs in: one whose final path segment is "Drafts",
/// else the bare name for a server that will create it.
fn drafts_mailbox(mailboxes: &[String]) -> String {
    named_mailbox(mailboxes, "Drafts")
}

/// The Trash folder a soft delete moves into, by the same rule.
fn trash_mailbox(mailboxes: &[String]) -> String {
    named_mailbox(mailboxes, "Trash")
}

/// A mailbox whose final path segment matches `name` (so `INBOX.Trash` and a
/// plain `Trash` both count), or the bare name as a fallback.
fn named_mailbox(mailboxes: &[String], name: &str) -> String {
    mailboxes
        .iter()
        .find(|mailbox| {
            mailbox
                .rsplit(['.', '/'])
                .next()
                .unwrap_or(mailbox)
                .eq_ignore_ascii_case(name)
        })
        .cloned()
        .unwrap_or_else(|| name.to_owned())
}

/// How long a prepared action waits for its confirmation.
const PENDING_TTL_SECONDS: u64 = 300;

/// Carry out a confirmed action against a live connection.
fn execute_operation(
    operation: &Operation,
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
) -> ToolResult {
    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    match operation {
        Operation::Move {
            message_ids,
            target,
        } => {
            service
                .move_messages(account_id, message_ids, target)
                .map_err(ToolFailure::Core)?;
            Ok(json!({ "status": "moved", "moved": message_ids.len(), "target": target }))
        }
        Operation::Trash { message_ids } => {
            let mailboxes = service.list_mailboxes(account_id).map_err(ToolFailure::Core)?;
            let trash = trash_mailbox(&mailboxes);
            service
                .trash_messages(account_id, message_ids, &trash)
                .map_err(ToolFailure::Core)?;
            Ok(json!({ "status": "trashed", "trashed": message_ids.len(), "mailbox": trash }))
        }
        Operation::Expunge { message_ids } => {
            service
                .expunge_messages(account_id, message_ids)
                .map_err(ToolFailure::Core)?;
            Ok(json!({ "status": "deleted", "deleted": message_ids.len() }))
        }
        // Sending is handled off the IMAP connection; confirm never routes it
        // here.
        Operation::Send { .. } => Err(ToolFailure::Core(CoreError::ProviderFailure(
            "send is not a mailbox operation".to_owned(),
        ))),
    }
}

/// Submit a draft over an SMTP TLS session. The secret is resolved the same
/// way as for IMAP — a password, or a bearer token renewed if it went stale.
fn send_over_smtp(
    config: &ImapProviderConfig,
    oauth: Option<&OAuthFacts>,
    record: &DraftRecord,
) -> CoreResult<()> {
    let secret = resolve_credential(config, oauth)?;
    let auth = match config.auth {
        ImapAuth::Password => SmtpAuth::Login {
            username: config.username.clone(),
            secret,
        },
        ImapAuth::XOAuth2 => SmtpAuth::XOAuth2 {
            username: config.username.clone(),
            access_token: secret,
        },
    };

    let mut client = torromail_imap_tls::connect_smtp(
        &config.host,
        config.port,
        config.security,
        &ehlo_domain(&record.from),
        auth,
    )?;
    let outcome = client.send_message(&record.from, &record.recipients, &record.raw);
    client.quit();
    outcome
}

/// What the client announces itself as: the sender's domain, or `localhost`
/// when the address has none.
fn ehlo_domain(from: &str) -> String {
    from.split_once('@')
        .map(|(_local, domain)| domain)
        .filter(|domain| !domain.is_empty())
        .unwrap_or("localhost")
        .to_owned()
}

/// Seconds since the Unix epoch. A clock is fine here — this is the MCP
/// facade, not the portable core — and a prepared action needs a real
/// expiry, not the fixed clock the search sessions run on.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// A short, stable confirmation code derived from the action id (FNV-1a). Not
/// a secret — the caller is handed it by `prepare` — but it ties a confirm to
/// one specific preparation and guards against confusing two pending actions.
fn confirmation_code(seed: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in seed.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:06x}", hash % 0x0100_0000)
}

fn handle_mail_mark(
    arguments: &Value,
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
) -> ToolResult {
    let Some(change) = arguments["mark"].as_str().and_then(MarkChange::parse) else {
        return Err(ToolFailure::InvalidParams(
            "mark must be one of seen, unseen, flagged, unflagged".to_owned(),
        ));
    };
    let message_ids: Vec<&str> = arguments["message_ids"]
        .as_array()
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    let mut marked = Vec::new();
    for message_id in message_ids {
        let message = service
            .mark(account_id, message_id, change)
            .map_err(ToolFailure::Core)?;
        marked.push(json!({
            "message_id": message.message_id(),
            "seen": message.seen(),
            "flagged": message.flagged()
        }));
    }

    Ok(json!({ "marked": marked }))
}

fn handle_mail_list_mailboxes(
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
) -> ToolResult {
    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(provider, engine, &mut sessions);

    let mailboxes = service
        .list_mailboxes(account_id)
        .map_err(ToolFailure::Core)?;
    Ok(json!({ "mailboxes": mailboxes }))
}

/// Unwrap a tool's own payload from a response envelope: it rides as a JSON
/// string inside the first text-content block. `None` for an error response
/// (no content) or anything that does not parse.
fn audit_payload(response: &Value) -> Option<Value> {
    let text = response["result"]["content"][0]["text"].as_str()?;
    serde_json::from_str(text).ok()
}

/// A short, human-meaningful description of what a call acted on — the query
/// searched, the message read, the recipients a draft names, the count and
/// destination of a move or delete. Empty for calls that name nothing (listing
/// accounts or mailboxes, reading the policy). For a confirmed action the
/// detail comes from the result, which is where the executed counts live.
fn audit_detail(name: &str, arguments: &Value, payload: Option<&Value>) -> String {
    let count = |value: &Value| string_array(value).len();
    match name {
        "mail_search" => {
            let query = arguments["query"].as_str().unwrap_or_default();
            match arguments["mailbox"].as_str() {
                Some(mailbox) if !mailbox.is_empty() && !query.is_empty() => {
                    format!("{query} · {mailbox}")
                }
                Some(mailbox) if !mailbox.is_empty() => mailbox.to_owned(),
                _ => query.to_owned(),
            }
        }
        "mail_refine_search" => arguments["refinement"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "mail_get_message" => arguments["message_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "mail_get_thread" => arguments["thread_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "mail_mark" => {
            let mark = arguments["mark"].as_str().unwrap_or_default();
            if mark.is_empty() {
                String::new()
            } else {
                format!("{mark} ×{}", count(&arguments["message_ids"]))
            }
        }
        "mail_create_draft" => {
            let to = string_array(&arguments["to"]).join(", ");
            let subject = arguments["subject"].as_str().unwrap_or_default();
            let base = match (to.is_empty(), subject.is_empty()) {
                (false, false) => format!("{to} — {subject}"),
                (false, true) => to,
                (true, false) => subject.to_owned(),
                (true, true) => String::new(),
            };
            let attachments = payload
                .and_then(|value| value["attachments"].as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value["filename"].as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if attachments.is_empty() {
                base
            } else {
                format!(
                    "{base} · {} attachment(s): {}",
                    attachments.len(),
                    attachments.join(", ")
                )
            }
        }
        "mail_prepare_send" => arguments["draft_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "mail_prepare_move" => format!(
            "{} → {}",
            count(&arguments["message_ids"]),
            arguments["target_mailbox"].as_str().unwrap_or_default()
        ),
        "mail_prepare_delete" => {
            let n = count(&arguments["message_ids"]);
            if arguments["permanent"].as_bool().unwrap_or(false) {
                format!("{n} · permanent")
            } else {
                format!("{n} → Trash")
            }
        }
        "mail_confirm_action" => confirm_detail(arguments, payload),
        _ => String::new(),
    }
}

/// What a confirmed action actually did, read from its result. Falls back to
/// the pending id when there is no result to describe — a refusal, an expired
/// or already-used confirmation.
fn confirm_detail(arguments: &Value, payload: Option<&Value>) -> String {
    let Some(payload) = payload else {
        return arguments["pending_action_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
    };
    let n = |key: &str| payload[key].as_u64().unwrap_or_default();
    let at = |key: &str| payload[key].as_str().unwrap_or_default().to_owned();
    match payload["status"].as_str().unwrap_or_default() {
        "moved" => format!("moved {} → {}", n("moved"), at("target")),
        "trashed" => format!("trashed {} → {}", n("trashed"), at("mailbox")),
        "deleted" => format!("deleted {}", n("deleted")),
        "sent" => {
            let attachments = n("attachment_count");
            if attachments == 0 {
                format!("sent → {} recipient(s)", n("recipients"))
            } else {
                format!(
                    "sent → {} recipient(s) · {attachments} attachment(s)",
                    n("recipients")
                )
            }
        }
        other => other.to_owned(),
    }
}

/// Tool payloads travel as MCP text content: JSON inside a text block.
fn json_rpc_text_result(id: &Value, payload: &Value) -> String {
    json_rpc_result(
        id,
        &json!({
            "content": [{
                "type": "text",
                "text": payload.to_string()
            }]
        }),
    )
}

fn json_rpc_result(id: &Value, result: &Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

/// The fixture mailbox behind `LineMcpServer::fixture`. Real accounts arrive
/// with the provider boundary for IMAP configuration.
fn fixture_provider(account_id: &AccountId) -> FixtureMailProvider {
    FixtureMailProvider::new([
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
    ])
}

fn json_rpc_error(id: &Value, code: i64, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
    .to_string()
}

fn canonical_tools() -> Vec<ToolDescriptor> {
    vec![
        read(
            ToolName::MailSearch,
            "Search one mail account and return a reusable result set, newest first. An empty query returns the latest messages instead of filtering; since/before narrow any search to a date range.",
        ),
        read(
            ToolName::MailRefineSearch,
            "Refine a prior search result set without repeating the account scan.",
        ),
        read(
            ToolName::MailGetMessage,
            "Fetch one message according to the account policy.",
        ),
        read(
            ToolName::MailGetThread,
            "Fetch a message thread according to the account policy.",
        ),
        read(
            ToolName::MailListMailboxes,
            "List mailboxes visible to the configured account.",
        ),
        direct_write(
            ToolName::MailMark,
            "Mark messages read or flagged where the account policy allows marking.",
        ),
        prepare(
            ToolName::MailCreateDraft,
            "Create a plain-text draft with optional base64 attachments when the account policy allows drafts.",
        ),
        prepare(
            ToolName::MailPrepareSend,
            "Prepare a send action for GUI confirmation.",
        ),
        prepare(
            ToolName::MailPrepareMove,
            "Prepare a move action for GUI confirmation.",
        ),
        prepare(
            ToolName::MailPrepareDelete,
            "Prepare a delete action for GUI confirmation.",
        ),
        ToolDescriptor {
            name: ToolName::MailConfirmAction,
            description: "Confirm a previously prepared action after GUI approval.",
            access_level: AccessLevel::ConfirmPreparedAction,
            requires_gui_confirmation: true,
        },
        admin_read(
            ToolName::MailListAccounts,
            "List accounts already configured in the GUI. Start here: every other tool needs an account_id from this list.",
        ),
        admin_read(
            ToolName::MailGetPolicy,
            "Read the effective policy for one account.",
        ),
        admin_read(
            ToolName::MailGetCacheStatus,
            "Read cache and index status for configured accounts.",
        ),
    ]
}

fn read(name: ToolName, description: &'static str) -> ToolDescriptor {
    ToolDescriptor {
        name,
        description,
        access_level: AccessLevel::Read,
        requires_gui_confirmation: false,
    }
}

fn direct_write(name: ToolName, description: &'static str) -> ToolDescriptor {
    ToolDescriptor {
        name,
        description,
        access_level: AccessLevel::DirectWrite,
        requires_gui_confirmation: false,
    }
}

fn prepare(name: ToolName, description: &'static str) -> ToolDescriptor {
    ToolDescriptor {
        name,
        description,
        access_level: AccessLevel::PrepareAction,
        requires_gui_confirmation: true,
    }
}

fn admin_read(name: ToolName, description: &'static str) -> ToolDescriptor {
    ToolDescriptor {
        name,
        description,
        access_level: AccessLevel::ReadOnlyAdmin,
        requires_gui_confirmation: false,
    }
}

fn tool_descriptor_json(tool: &ToolDescriptor) -> String {
    format!(
        r#"{{"name":"{}","description":"{}","inputSchema":{}}}"#,
        tool.name_str(),
        escape_json_string(tool.description),
        tool.input_schema_json()
    )
}

fn escape_json_string(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            character => vec![character],
        })
        .collect()
}
