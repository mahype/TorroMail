//! Portable TorroMail core.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    GuiOnlyMutation,
    DuplicateAccount(AccountId),
    AccountNotFound(AccountId),
    CapabilityDenied {
        account_id: AccountId,
        capability: Capability,
    },
    PendingActionNotFound(String),
    PendingActionExpired(String),
    PendingActionAlreadyConfirmed(String),
    SearchResultSetNotFound(String),
    SearchResultSetExpired(String),
}

impl Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GuiOnlyMutation => write!(f, "account mutations are only allowed from the GUI"),
            Self::DuplicateAccount(id) => write!(f, "account already exists: {id}"),
            Self::AccountNotFound(id) => write!(f, "account not found: {id}"),
            Self::CapabilityDenied {
                account_id,
                capability,
            } => {
                write!(f, "{capability:?} is not allowed for account {account_id}")
            }
            Self::PendingActionNotFound(id) => write!(f, "pending action not found: {id}"),
            Self::PendingActionExpired(id) => write!(f, "pending action expired: {id}"),
            Self::PendingActionAlreadyConfirmed(id) => {
                write!(f, "pending action already confirmed: {id}")
            }
            Self::SearchResultSetNotFound(id) => write!(f, "search result set not found: {id}"),
            Self::SearchResultSetExpired(id) => write!(f, "search result set expired: {id}"),
        }
    }
}

impl Error for CoreError {}

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountId(String);

impl AccountId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Gui,
    McpTool,
    Cli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    Password,
    OAuth2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDraft {
    id: AccountId,
    display_name: String,
    email: String,
    provider: ProviderKind,
    auth_method: AuthMethod,
    imap_host: Option<String>,
    smtp_host: Option<String>,
    cache_policy: CachePolicy,
}

impl AccountDraft {
    pub fn imap_password(
        id: AccountId,
        display_name: impl Into<String>,
        email: impl Into<String>,
        imap_host: impl Into<String>,
        smtp_host: impl Into<String>,
    ) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            email: email.into(),
            provider: ProviderKind::ImapSmtp,
            auth_method: AuthMethod::Password,
            imap_host: Some(imap_host.into()),
            smtp_host: Some(smtp_host.into()),
            cache_policy: CachePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    id: AccountId,
    display_name: String,
    email: String,
    provider: ProviderKind,
    auth_method: AuthMethod,
    imap_host: Option<String>,
    smtp_host: Option<String>,
    cache_policy: CachePolicy,
}

impl Account {
    pub fn id(&self) -> &AccountId {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    pub fn cache_policy(&self) -> &CachePolicy {
        &self.cache_policy
    }
}

impl From<AccountDraft> for Account {
    fn from(value: AccountDraft) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name,
            email: value.email,
            provider: value.provider,
            auth_method: value.auth_method,
            imap_host: value.imap_host,
            smtp_host: value.smtp_host,
            cache_policy: value.cache_policy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    ImapSmtp,
    GmailProfile,
    MicrosoftProfile,
    JmapProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePolicy {
    pub persist_metadata: bool,
    pub persist_headers: bool,
    pub persist_bodies: bool,
    pub index_bodies: bool,
    pub index_attachments: bool,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            persist_metadata: true,
            persist_headers: true,
            persist_bodies: false,
            index_bodies: false,
            index_attachments: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct AccountRegistry {
    accounts: BTreeMap<AccountId, Account>,
}

impl AccountRegistry {
    pub fn add_account(&mut self, channel: Channel, draft: AccountDraft) -> CoreResult<&Account> {
        if channel != Channel::Gui {
            return Err(CoreError::GuiOnlyMutation);
        }

        let account_id = draft.id.clone();
        if self.accounts.contains_key(&account_id) {
            return Err(CoreError::DuplicateAccount(account_id));
        }

        self.accounts.insert(account_id.clone(), draft.into());
        self.accounts
            .get(&account_id)
            .ok_or(CoreError::AccountNotFound(account_id))
    }

    pub fn list_accounts(&self) -> Vec<&Account> {
        self.accounts.values().collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    ReadHeaders,
    ReadBody,
    Search,
    DownloadAttachments,
    Draft,
    Send,
    Mark,
    Move,
    DeleteSoft,
    DeletePermanent,
    CacheMetadata,
    IndexBody,
    WatchIdle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    account_id: AccountId,
    capabilities: BTreeSet<Capability>,
}

impl Policy {
    pub fn new(account_id: AccountId, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            account_id,
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn allows(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

#[derive(Debug, Default)]
pub struct PolicyEngine {
    policies: BTreeMap<AccountId, Policy>,
}

impl PolicyEngine {
    pub fn new(policies: impl IntoIterator<Item = Policy>) -> Self {
        Self {
            policies: policies
                .into_iter()
                .map(|policy| (policy.account_id.clone(), policy))
                .collect(),
        }
    }

    pub fn authorize(&self, account_id: &AccountId, capability: Capability) -> CoreResult<()> {
        let policy = self
            .policies
            .get(account_id)
            .ok_or_else(|| CoreError::AccountNotFound(account_id.clone()))?;

        if policy.allows(capability) {
            Ok(())
        } else {
            Err(CoreError::CapabilityDenied {
                account_id: account_id.clone(),
                capability,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Send,
    Move,
    DeleteSoft,
    DeletePermanent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingActionRequest {
    account_id: AccountId,
    kind: ActionKind,
    preview: String,
    affected_messages: usize,
    ttl_seconds: u64,
}

impl PendingActionRequest {
    pub fn new(
        account_id: AccountId,
        kind: ActionKind,
        preview: impl Into<String>,
        affected_messages: usize,
        ttl_seconds: u64,
    ) -> Self {
        Self {
            account_id,
            kind,
            preview: preview.into(),
            affected_messages,
            ttl_seconds,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAction {
    id: String,
    request: PendingActionRequest,
    created_at: u64,
    expires_at: u64,
    confirmed_at: Option<u64>,
}

impl PendingAction {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub fn is_confirmed(&self) -> bool {
        self.confirmed_at.is_some()
    }

    pub fn preview(&self) -> &str {
        &self.request.preview
    }
}

#[derive(Debug, Default)]
pub struct PendingActionStore {
    next_id: u64,
    actions: BTreeMap<String, PendingAction>,
}

impl PendingActionStore {
    pub fn prepare(&mut self, request: PendingActionRequest, now: u64) -> PendingAction {
        self.next_id += 1;
        let id = format!("pending-{}", self.next_id);
        let action = PendingAction {
            id: id.clone(),
            expires_at: now + request.ttl_seconds,
            request,
            created_at: now,
            confirmed_at: None,
        };
        self.actions.insert(id, action.clone());
        action
    }

    pub fn confirm(&mut self, id: &str, now: u64) -> CoreResult<PendingAction> {
        let action = self
            .actions
            .get_mut(id)
            .ok_or_else(|| CoreError::PendingActionNotFound(id.to_owned()))?;

        if action.confirmed_at.is_some() {
            return Err(CoreError::PendingActionAlreadyConfirmed(id.to_owned()));
        }

        if now > action.expires_at {
            return Err(CoreError::PendingActionExpired(id.to_owned()));
        }

        action.confirmed_at = Some(now);
        Ok(action.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    message_id: String,
    subject: String,
    sender: String,
    snippet: String,
}

impl SearchHit {
    pub fn new(
        message_id: impl Into<String>,
        subject: impl Into<String>,
        sender: impl Into<String>,
        snippet: impl Into<String>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            subject: subject.into(),
            sender: sender.into(),
            snippet: snippet.into(),
        }
    }

    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    fn searchable_text(&self) -> String {
        format!("{} {} {}", self.subject, self.sender, self.snippet).to_lowercase()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResultSet {
    id: String,
    account_id: AccountId,
    query: String,
    hits: Vec<SearchHit>,
    created_at: u64,
    expires_at: u64,
}

impl SearchResultSet {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }
}

#[derive(Debug, Default)]
pub struct SearchSessionStore {
    next_id: u64,
    result_sets: BTreeMap<String, SearchResultSet>,
}

impl SearchSessionStore {
    pub fn create(
        &mut self,
        account_id: AccountId,
        query: impl Into<String>,
        hits: Vec<SearchHit>,
        now: u64,
        ttl_seconds: u64,
    ) -> SearchResultSet {
        self.next_id += 1;
        let id = format!("result-set-{}", self.next_id);
        let result_set = SearchResultSet {
            id: id.clone(),
            account_id,
            query: query.into(),
            hits,
            created_at: now,
            expires_at: now + ttl_seconds,
        };
        self.result_sets.insert(id, result_set.clone());
        result_set
    }

    pub fn refine(
        &mut self,
        result_set_id: &str,
        refinement: &str,
        now: u64,
    ) -> CoreResult<SearchResultSet> {
        let existing = self
            .result_sets
            .get(result_set_id)
            .ok_or_else(|| CoreError::SearchResultSetNotFound(result_set_id.to_owned()))?;

        if now > existing.expires_at {
            return Err(CoreError::SearchResultSetExpired(result_set_id.to_owned()));
        }

        let query = format!("{} {}", existing.query, refinement)
            .trim()
            .to_owned();
        let tokens = query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        let hits = existing
            .hits
            .iter()
            .filter(|hit| {
                let text = hit.searchable_text();
                tokens.iter().all(|token| text.contains(token))
            })
            .cloned()
            .collect();

        Ok(self.create(
            existing.account_id.clone(),
            query,
            hits,
            now,
            existing.expires_at - now,
        ))
    }
}
