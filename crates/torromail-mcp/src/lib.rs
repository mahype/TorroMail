//! TorroMail MCP facade.
//!
//! This crate keeps the public MCP surface explicit. Account setup, secret
//! changes, OAuth setup, and permission edits stay on the GUI/IPC side.

mod keychain;
mod policy_document;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use torromail_core::{
    AccountId, Capability, CoreError, CoreResult, FixtureMailProvider, ImapAuth,
    ImapProviderConfig, MailAccessService, MailProvider, MarkChange, PermissionSet, Policy,
    PolicyEngine, ReadAccess, SearchSessionStore, SearchWindow, StoredMessage,
};
use torromail_oauth::TokenSet;

use crate::policy_document::{DocumentAccount, OAuthFacts, parse_policy_document};

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
    /// Test seam: when set, this makes the mailbox instead of a TLS session.
    connect_override: Option<ConnectOverride>,
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
            connect_override: None,
        }
    }

    /// A server that enforces the policy document at `path`, reloading it on
    /// every tool call so permission changes in the app apply immediately.
    pub fn with_policy_path(path: impl Into<PathBuf>) -> Self {
        Self {
            policy_path: Some(path.into()),
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
            "initialize" => json_rpc_result(
                &id,
                &json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "TorroMail", "version": "0.1.0"}
                }),
            ),
            "tools/list" => format!(
                r#"{{"jsonrpc":"2.0","id":{id},"result":{}}}"#,
                self.catalog.to_mcp_tools_json()
            ),
            "tools/call" => self.handle_tool_call(&request, &id),
            _ => json_rpc_error(&id, -32601, "method not found"),
        })
    }

    fn handle_tool_call(&self, request: &Value, id: &Value) -> String {
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

        let account_id = AccountId::new(arguments["account_id"].as_str().unwrap_or_default());
        if name == ToolName::MailGetPolicy.as_str() {
            return self.handle_mail_get_policy(&account_id, id);
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
                handle_mail_create_draft(arguments, &account_id, provider, engine, &from)
            });
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
            let identity = match ConnIdentity::from_facts(&facts) {
                Ok(identity) => identity,
                Err(error) => return json_rpc_error(id, -32000, &error.to_string()),
            };

            let mut pool = self.connections.borrow_mut();
            let stale = pool
                .get(account_id)
                .is_none_or(|connection| connection.identity != identity);
            if stale {
                let provider = match self.open_connection(account_id, facts) {
                    Ok(provider) => provider,
                    Err(error) => return json_rpc_error(id, -32000, &error.to_string()),
                };
                pool.insert(account_id.clone(), PooledConnection { identity, provider });
            }
            let connection = pool
                .get_mut(account_id)
                .expect("a connection was just ensured");

            match run(connection.provider.as_mut(), engine) {
                Ok(payload) => return json_rpc_text_result(id, &payload),
                Err(failure) if failure.is_connection() && !rebuilt => {
                    // The session is suspect: drop it so the retry opens a
                    // fresh one, and do not loop forever.
                    pool.remove(account_id);
                    rebuilt = true;
                }
                Err(failure) => return failure.into_response(id),
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

    /// The accounts the document describes, reloaded per call. `None` means
    /// there is no document at all — fixture mode, or an app that has not
    /// published yet. An empty list is a different answer entirely: a
    /// document that names no accounts grants nothing.
    fn document_accounts(&self) -> Result<Option<Vec<DocumentAccount>>, String> {
        let Some(path) = &self.policy_path else {
            return Ok(None);
        };

        match std::fs::read_to_string(path) {
            Ok(text) => parse_policy_document(&text).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("policy document unreadable: {error}")),
        }
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
    /// The identity a pool entry must still match to be reused. Unconfigured
    /// accounts have no connection at all, so there is nothing to key.
    fn from_facts(facts: &ConnectionFacts) -> CoreResult<Self> {
        match facts {
            ConnectionFacts::FixtureMode => Ok(Self::Fixture),
            ConnectionFacts::Configured(config, _) => {
                Ok(Self::Configured((**config).clone()))
            }
            ConnectionFacts::Unconfigured => Err(CoreError::ProviderFailure(
                "account has no connection configured — finish setting it up in TorroMail"
                    .to_owned(),
            )),
        }
    }
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
        ConnectionFacts::Unconfigured => Err(CoreError::ProviderFailure(format!(
            "account {account_id} has no connection configured — finish setting it up in TorroMail"
        ))),
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
pub fn check_account(account_id: &str, policy_path: Option<PathBuf>) -> Result<String, String> {
    let path = policy_path.ok_or("no policy document path available")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("policy document unreadable: {error}"))?;
    let accounts = parse_policy_document(&text)?;
    let account = accounts
        .iter()
        .find(|account| {
            account
                .imap
                .as_ref()
                .is_some_and(|config| config.account_id.as_str() == account_id)
        })
        .ok_or(format!("no IMAP configuration for account {account_id}"))?;
    let config = account
        .imap
        .as_ref()
        .ok_or(format!("no IMAP configuration for account {account_id}"))?;

    // The same credential path the tools take, token renewal included — so a
    // green dot in the app means an assistant would get in too.
    let secret =
        resolve_credential(config, account.oauth.as_ref()).map_err(|error| error.to_string())?;
    let provider =
        torromail_imap_tls::connect_account(config, &secret).map_err(|error| error.to_string())?;
    let mailboxes = provider
        .list_mailboxes(&config.account_id)
        .map_err(|error| error.to_string())?;

    Ok(format!("login ok, {} mailboxes visible", mailboxes.len()))
}

/// The product default until the app has published permissions: read and
/// drafts, nothing else.
fn default_engine(account_id: &AccountId) -> PolicyEngine {
    PolicyEngine::new([Policy::new(account_id.clone(), PermissionSet::default())])
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

fn handle_mail_create_draft(
    arguments: &Value,
    account_id: &AccountId,
    provider: &mut dyn MailProvider,
    engine: PolicyEngine,
    from: &str,
) -> ToolResult {
    let to = string_array(&arguments["to"]);
    let cc = string_array(&arguments["cc"]);
    let bcc = string_array(&arguments["bcc"]);
    let subject = arguments["subject"].as_str().unwrap_or_default();
    let body = arguments["body"].as_str().unwrap_or_default();

    if to.is_empty() {
        return Err(ToolFailure::InvalidParams(
            "to needs at least one recipient".to_owned(),
        ));
    }

    let message = torromail_core::compose_message(from, &to, &cc, &bcc, subject, body);

    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    // The drafts folder is wherever the account keeps it — its name is the
    // last path segment, so `INBOX.Drafts` and a plain `Drafts` both match.
    let mailboxes = service.list_mailboxes(account_id).map_err(ToolFailure::Core)?;
    let mailbox = drafts_mailbox(&mailboxes);
    service
        .create_draft(account_id, &mailbox, &message)
        .map_err(ToolFailure::Core)?;

    Ok(json!({ "status": "draft_created", "mailbox": mailbox }))
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
    mailboxes
        .iter()
        .find(|mailbox| {
            mailbox
                .rsplit(['.', '/'])
                .next()
                .unwrap_or(mailbox)
                .eq_ignore_ascii_case("Drafts")
        })
        .cloned()
        .unwrap_or_else(|| "Drafts".to_owned())
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
