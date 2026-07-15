//! TorroMail MCP facade.
//!
//! This crate keeps the public MCP surface explicit. Account setup, secret
//! changes, OAuth setup, and permission edits stay on the GUI/IPC side.

mod policy_document;

use std::path::PathBuf;

use serde_json::{Value, json};
use torromail_core::{
    AccountId, FixtureMailProvider, MailAccessService, MarkChange, PermissionSet, Policy,
    PolicyEngine, SearchSessionStore, StoredMessage,
};

use crate::policy_document::parse_policy_document;

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
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"query":{"type":"string"},"mailbox":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["query"]}"#
            }
            ToolName::MailRefineSearch => {
                r#"{"type":"object","properties":{"result_set_id":{"type":"string"},"refinement":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["result_set_id","refinement"]}"#
            }
            ToolName::MailGetMessage => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"message_id":{"type":"string"},"include_body":{"type":"boolean"},"include_attachments":{"type":"boolean"}},"required":["account_id","message_id"]}"#
            }
            ToolName::MailGetThread => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"thread_id":{"type":"string"},"include_bodies":{"type":"boolean"}},"required":["account_id","thread_id"]}"#
            }
            ToolName::MailListMailboxes => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"}},"required":["account_id"]}"#
            }
            ToolName::MailMark => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"mailbox":{"type":"string"},"message_ids":{"type":"array","items":{"type":"string"}},"mark":{"type":"string","enum":["seen","unseen","flagged","unflagged"]}},"required":["account_id","mailbox","message_ids","mark"]}"#
            }
            ToolName::MailCreateDraft => {
                r#"{"type":"object","properties":{"account_id":{"type":"string"},"to":{"type":"array","items":{"type":"string"}},"cc":{"type":"array","items":{"type":"string"}},"bcc":{"type":"array","items":{"type":"string"}},"subject":{"type":"string"},"body":{"type":"string"}},"required":["account_id","to","subject","body"]}"#
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LineMcpServer {
    catalog: ToolCatalog,
    /// Where the app publishes the account permissions. `None` runs the
    /// product default (read + drafts) for any account — the fixture mode
    /// tests use.
    policy_path: Option<PathBuf>,
}

impl LineMcpServer {
    /// A server wired to fixture data — the same policy-checked path real
    /// providers will use, minus the network.
    pub fn fixture() -> Self {
        Self::default()
    }

    /// A server that enforces the policy document at `path`, reloading it on
    /// every tool call so permission changes in the app apply immediately.
    pub fn with_policy_path(path: impl Into<PathBuf>) -> Self {
        Self {
            catalog: ToolCatalog::default(),
            policy_path: Some(path.into()),
        }
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
        if !self.catalog.names_as_str().contains(&name) {
            return json_rpc_error(id, -32601, "tool not found");
        }

        let arguments = &request["params"]["arguments"];
        let account_id = AccountId::new(arguments["account_id"].as_str().unwrap_or_default());
        let engine = match self.policy_engine_for(&account_id) {
            Ok(engine) => engine,
            Err(message) => return json_rpc_error(id, -32000, &message),
        };

        if name == ToolName::MailSearch.as_str() {
            return handle_mail_search(arguments, &account_id, engine, id);
        }
        if name == ToolName::MailGetMessage.as_str() {
            return handle_mail_get_message(arguments, &account_id, engine, id);
        }
        if name == ToolName::MailMark.as_str() {
            return handle_mail_mark(arguments, &account_id, engine, id);
        }
        if name == ToolName::MailListMailboxes.as_str() {
            return handle_mail_list_mailboxes(&account_id, engine, id);
        }

        json_rpc_error(id, -32000, "tool not implemented yet")
    }

    /// The engine for this call. With a policy path the document decides —
    /// reloaded every time, failing closed when unreadable, refusing
    /// accounts it does not contain. Without one, the product default.
    fn policy_engine_for(&self, account_id: &AccountId) -> Result<PolicyEngine, String> {
        let Some(path) = &self.policy_path else {
            return Ok(default_engine(account_id));
        };

        match std::fs::read_to_string(path) {
            Ok(text) => parse_policy_document(&text).map(PolicyEngine::new),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(default_engine(account_id))
            }
            Err(error) => Err(format!("policy document unreadable: {error}")),
        }
    }
}

/// The product default until the app has published permissions: read and
/// drafts, nothing else.
fn default_engine(account_id: &AccountId) -> PolicyEngine {
    PolicyEngine::new([Policy::new(account_id.clone(), PermissionSet::default())])
}

fn handle_mail_search(
    arguments: &Value,
    account_id: &AccountId,
    engine: PolicyEngine,
    id: &str,
) -> String {
    let query = arguments["query"].as_str().unwrap_or_default();
    let mailbox = arguments["mailbox"].as_str();
    let limit = arguments["limit"].as_u64().unwrap_or(10).min(100) as usize;

    let mut sessions = SearchSessionStore::default();
    let mut service = fixture_service(account_id, engine, &mut sessions);

    match service.search(account_id, query, mailbox, limit, 100) {
        Ok(result_set) => {
            let hits = result_set
                .hits()
                .iter()
                .map(|hit| {
                    json!({
                        "message_id": hit.message_id(),
                        "mailbox": hit.mailbox(),
                        "subject": hit.subject()
                    })
                })
                .collect::<Vec<_>>();
            json_rpc_text_result(
                id,
                &json!({
                    "result_set_id": result_set.id(),
                    "hits": hits
                }),
            )
        }
        Err(error) => json_rpc_error(id, -32000, &error.to_string()),
    }
}

fn handle_mail_get_message(
    arguments: &Value,
    account_id: &AccountId,
    engine: PolicyEngine,
    id: &str,
) -> String {
    let message_id = arguments["message_id"].as_str().unwrap_or_default();
    let include_body = arguments["include_body"].as_bool().unwrap_or(false);

    let mut sessions = SearchSessionStore::default();
    let service = fixture_service(account_id, engine, &mut sessions);

    match service.get_message(account_id, message_id, include_body) {
        Ok(message) => json_rpc_text_result(
            id,
            &json!({
                "message_id": message.message_id(),
                "mailbox": message.mailbox(),
                "thread_id": message.thread_id(),
                "subject": message.subject(),
                "sender": message.sender(),
                "snippet": message.snippet(),
                "body": message.body(),
                "seen": message.seen(),
                "flagged": message.flagged()
            }),
        ),
        Err(error) => json_rpc_error(id, -32000, &error.to_string()),
    }
}

fn handle_mail_mark(
    arguments: &Value,
    account_id: &AccountId,
    engine: PolicyEngine,
    id: &str,
) -> String {
    let Some(change) = arguments["mark"].as_str().and_then(MarkChange::parse) else {
        return json_rpc_error(
            id,
            -32602,
            "mark must be one of seen, unseen, flagged, unflagged",
        );
    };
    let message_ids: Vec<&str> = arguments["message_ids"]
        .as_array()
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut sessions = SearchSessionStore::default();
    let mut service = fixture_service(account_id, engine, &mut sessions);

    let mut marked = Vec::new();
    for message_id in message_ids {
        match service.mark(account_id, message_id, change) {
            Ok(message) => marked.push(json!({
                "message_id": message.message_id(),
                "seen": message.seen(),
                "flagged": message.flagged()
            })),
            Err(error) => return json_rpc_error(id, -32000, &error.to_string()),
        }
    }

    json_rpc_text_result(id, &json!({ "marked": marked }))
}

fn handle_mail_list_mailboxes(account_id: &AccountId, engine: PolicyEngine, id: &str) -> String {
    let mut sessions = SearchSessionStore::default();
    let service = fixture_service(account_id, engine, &mut sessions);

    match service.list_mailboxes(account_id) {
        Ok(mailboxes) => json_rpc_text_result(id, &json!({ "mailboxes": mailboxes })),
        Err(error) => json_rpc_error(id, -32000, &error.to_string()),
    }
}

/// One policy-checked service over the fixture mailbox. The engine carries
/// whatever the policy document granted; real mail data arrives with the
/// IMAP provider.
fn fixture_service<'a>(
    account_id: &AccountId,
    engine: PolicyEngine,
    sessions: &'a mut SearchSessionStore,
) -> MailAccessService<'a, FixtureMailProvider> {
    MailAccessService::new(fixture_provider(account_id), engine, sessions)
}

fn json_rpc_text_result(id: &str, payload: &Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": serde_json::from_str::<Value>(id).unwrap_or(Value::Null),
        "result": {
            "content": [{
                "type": "text",
                "text": payload.to_string()
            }]
        }
    })
    .to_string()
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

fn canonical_tools() -> Vec<ToolDescriptor> {
    vec![
        read(
            ToolName::MailSearch,
            "Search configured mail accounts and return a reusable result set.",
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
            "Create a draft when the account policy allows drafts.",
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
            "List accounts already configured in the GUI.",
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

fn extract_json_rpc_id(line: &str) -> Option<&str> {
    let id_key = line.find(r#""id""#)?;
    let after_key = &line[id_key + 4..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let end = after_colon.find([',', '}']).unwrap_or(after_colon.len());
    Some(after_colon[..end].trim())
}
