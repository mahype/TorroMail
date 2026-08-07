//! Portable TorroMail core.

pub mod imap_provider;
mod mime;
pub mod smtp;

pub use imap_provider::{
    ConnectionSecurity, FetchedMessage, ImapAuth, ImapClient, ImapMailProvider, ImapProviderConfig,
    ImapTransport, SecretRef, StreamImapTransport, TcpImapTransport,
};
/// Standard padded base64, shared so the MCP layer can inline small
/// attachment downloads without a second implementation.
pub use mime::encode_base64;

use std::collections::BTreeMap;
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
    MessageNotFound(String),
    AttachmentNotFound {
        message_id: String,
        attachment_id: String,
    },
    /// The mail provider could not be reached or could not do as it was
    /// asked. Everything transient lives here — see `CredentialRejected` for
    /// the one failure that will not fix itself on its own.
    ProviderFailure(String),
    /// The server spoke a refusal to the login command itself: a wrong
    /// password, an expired app password, a rejected OAuth token. Kept apart
    /// from `ProviderFailure` because the two need opposite handling — this
    /// one will not fix itself, and retrying it is pointless.
    CredentialRejected(String),
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
            Self::MessageNotFound(id) => write!(f, "message not found: {id}"),
            Self::AttachmentNotFound {
                message_id,
                attachment_id,
            } => write!(
                f,
                "attachment {attachment_id} not found on message {message_id}"
            ),
            Self::ProviderFailure(message) => write!(f, "mail provider failure: {message}"),
            Self::CredentialRejected(message) => {
                write!(f, "credentials rejected: {message}")
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

/// How much of an account may rest on this Mac, in the language of the read
/// permissions: nothing, the envelope, whole messages, or messages plus their
/// attachment files. One decision — indexing always covers exactly what is
/// stored, so there is no separate index switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CacheLevel {
    Off,
    Headers,
    Bodies,
    Attachments,
}

impl CacheLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Headers => "headers",
            Self::Bodies => "bodies",
            Self::Attachments => "attachments",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(Self::Off),
            "headers" => Some(Self::Headers),
            "bodies" => Some(Self::Bodies),
            "attachments" => Some(Self::Attachments),
            _ => None,
        }
    }

    /// The old five-switch shape, mapped: a disabled cache is off, the two
    /// header-ish modes are `Headers`, and both body modes are `Bodies` —
    /// "full text" as a storage level never meant anything more.
    pub fn parse_legacy(local_cache_enabled: bool, mode: &str) -> Self {
        if !local_cache_enabled {
            return Self::Off;
        }
        match mode {
            "body" | "fullText" => Self::Bodies,
            _ => Self::Headers,
        }
    }

    /// The most the read permission justifies keeping on disk: nothing is
    /// stored that no assistant may read.
    pub fn ceiling(read: ReadAccess) -> Self {
        match read {
            ReadAccess::None => Self::Off,
            ReadAccess::Headers => Self::Headers,
            ReadAccess::FullMessage => Self::Bodies,
            ReadAccess::WithAttachments => Self::Attachments,
        }
    }
}

/// The level that actually applies: the chosen one, capped by the read
/// permission's ceiling.
pub fn effective_cache_level(chosen: CacheLevel, read: ReadAccess) -> CacheLevel {
    chosen.min(CacheLevel::ceiling(read))
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

/// How deep assistants may read. The levels build on each other: the message
/// implies its header, attachments imply the message. Mirrors `ReadAccess`
/// in TorroMailKit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReadAccess {
    None,
    Headers,
    FullMessage,
    WithAttachments,
}

/// Mailbox mutations — everything that changes the mailbox but stays on the
/// server. Sending is deliberately not in here: it is the one right that
/// acts on the outside world. Mirrors `WriteAccess` in TorroMailKit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct WriteAccess {
    pub drafts: bool,
    pub mark: bool,
    /// `move` in the Swift model; renamed here because of the keyword.
    pub move_messages: bool,
    pub trash: bool,
    /// Escalation of `trash`; meaningless without it, so `sanitized` clears
    /// it when the trash right falls. Never part of a preset.
    pub permanent_delete: bool,
}

impl WriteAccess {
    pub const NOTHING: Self = Self {
        drafts: false,
        mark: false,
        move_messages: false,
        trash: false,
        permanent_delete: false,
    };

    pub fn is_empty(&self) -> bool {
        !(self.drafts || self.mark || self.move_messages || self.trash || self.permanent_delete)
    }

    /// Permanent delete cannot outlive the trash right it escalates.
    pub fn sanitized(mut self) -> Self {
        if !self.trash {
            self.permanent_delete = false;
        }
        self
    }
}

/// Per-mailbox exception to the account-wide groups. `true` means "the group
/// applies here" — the effective right is always the intersection with the
/// account, so a folder can never allow more than the account does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FolderRule {
    pub read: bool,
    pub write: bool,
}

impl FolderRule {
    /// What every folder starts as: follow the account.
    pub const STANDARD: Self = Self {
        read: true,
        write: true,
    };
}

impl Default for FolderRule {
    fn default() -> Self {
        Self::STANDARD
    }
}

/// The named starting points. A preset is a fact about the current values —
/// derived, never stored — so a "custom" state can never drift out of sync
/// with the switches. Mirrors `PermissionPreset` in TorroMailKit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionPreset {
    /// Read everything, change nothing.
    ReadOnly,
    /// The everyday default: read mail and prepare drafts, send nothing.
    ReadAndDrafts,
    /// Keep the mailbox in shape: mark, move, delete — no composing.
    TidyUp,
    /// Everything except permanent deletion, which is never preset.
    FullAccess,
}

impl PermissionPreset {
    pub const ALL: [Self; 4] = [
        Self::ReadOnly,
        Self::ReadAndDrafts,
        Self::TidyUp,
        Self::FullAccess,
    ];

    pub fn read(self) -> ReadAccess {
        if self == Self::FullAccess {
            ReadAccess::WithAttachments
        } else {
            ReadAccess::FullMessage
        }
    }

    pub fn write(self) -> WriteAccess {
        match self {
            Self::ReadOnly => WriteAccess::NOTHING,
            Self::ReadAndDrafts => WriteAccess {
                drafts: true,
                ..WriteAccess::NOTHING
            },
            Self::TidyUp => WriteAccess {
                mark: true,
                move_messages: true,
                trash: true,
                ..WriteAccess::NOTHING
            },
            Self::FullAccess => WriteAccess {
                drafts: true,
                mark: true,
                move_messages: true,
                trash: true,
                ..WriteAccess::NOTHING
            },
        }
    }

    pub fn send(self) -> bool {
        self == Self::FullAccess
    }

    pub fn permission_set(self) -> PermissionSet {
        PermissionSet {
            read: self.read(),
            write: self.write(),
            send: self.send(),
            per_folder: false,
            folder_rules: BTreeMap::new(),
        }
    }
}

/// What connected assistants may do with an account, in three groups modelled
/// on the file system: read (changes nothing), write (changes the mailbox but
/// stays on the server), send (leaves the house). Folder exceptions scope the
/// first two; sending is account-wide because SMTP is not bound to a mailbox.
/// Mirrors `PermissionSet` in TorroMailKit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionSet {
    pub read: ReadAccess,
    pub write: WriteAccess,
    pub send: bool,
    /// Whether folder exceptions apply. Off means the groups rule every
    /// folder; stored exceptions are kept, just inert.
    pub per_folder: bool,
    /// Exceptions by mailbox name. No entry = standard (follow the account).
    pub folder_rules: BTreeMap<String, FolderRule>,
}

impl Default for PermissionSet {
    /// The everyday default: `PermissionPreset::ReadAndDrafts`.
    fn default() -> Self {
        PermissionPreset::ReadAndDrafts.permission_set()
    }
}

impl PermissionSet {
    pub fn rule_for(&self, mailbox: &str) -> FolderRule {
        if !self.per_folder {
            return FolderRule::STANDARD;
        }
        self.folder_rules
            .get(mailbox)
            .copied()
            .unwrap_or(FolderRule::STANDARD)
    }

    pub fn read_access_in(&self, mailbox: &str) -> ReadAccess {
        if self.rule_for(mailbox).read {
            self.read
        } else {
            ReadAccess::None
        }
    }

    pub fn write_access_in(&self, mailbox: &str) -> WriteAccess {
        if self.rule_for(mailbox).write {
            self.write.sanitized()
        } else {
            WriteAccess::NOTHING
        }
    }

    /// A mailbox with no effective rights is invisible to assistants.
    pub fn can_access(&self, mailbox: &str) -> bool {
        self.read_access_in(mailbox) != ReadAccess::None
            || !self.write_access_in(mailbox).is_empty()
    }

    /// The preset these values match, if any. Folder exceptions do not
    /// count: presets decide the groups, exceptions only scope them.
    pub fn matching_preset(&self) -> Option<PermissionPreset> {
        PermissionPreset::ALL.into_iter().find(|preset| {
            preset.read() == self.read && preset.write() == self.write && preset.send() == self.send
        })
    }

    /// Applies the preset's groups. Folder exceptions survive — switching
    /// the profile is not meant to throw away per-folder decisions.
    pub fn apply(&mut self, preset: PermissionPreset) {
        self.read = preset.read();
        self.write = preset.write();
        self.send = preset.send();
    }
}

/// The questions tools ask of a policy. Every variant is backed by the
/// permission groups; cache and index decisions live in `CachePolicy`.
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    account_id: AccountId,
    permissions: PermissionSet,
}

impl Policy {
    pub fn new(account_id: AccountId, permissions: PermissionSet) -> Self {
        Self {
            account_id,
            permissions,
        }
    }

    pub fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    pub fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }

    /// Account-wide answer, before any folder exception.
    pub fn allows(&self, capability: Capability) -> bool {
        granted(
            capability,
            self.permissions.read,
            self.permissions.write.sanitized(),
            self.permissions.send,
        )
    }

    /// Folder-scoped answer: the account groups cut down by the mailbox rule.
    pub fn allows_in(&self, mailbox: &str, capability: Capability) -> bool {
        granted(
            capability,
            self.permissions.read_access_in(mailbox),
            self.permissions.write_access_in(mailbox),
            self.permissions.send,
        )
    }
}

fn granted(capability: Capability, read: ReadAccess, write: WriteAccess, send: bool) -> bool {
    match capability {
        Capability::Search => read > ReadAccess::None,
        Capability::ReadHeaders => read >= ReadAccess::Headers,
        Capability::ReadBody => read >= ReadAccess::FullMessage,
        Capability::DownloadAttachments => read >= ReadAccess::WithAttachments,
        Capability::Draft => write.drafts,
        Capability::Send => send,
        Capability::Mark => write.mark,
        Capability::Move => write.move_messages,
        Capability::DeleteSoft => write.trash,
        Capability::DeletePermanent => write.permanent_delete,
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
        let policy = self.policy(account_id)?;
        Self::verdict(policy.allows(capability), account_id, capability)
    }

    /// Folder-scoped authorization for operations that happen inside one
    /// mailbox: reading, marking, deleting.
    pub fn authorize_in(
        &self,
        account_id: &AccountId,
        mailbox: &str,
        capability: Capability,
    ) -> CoreResult<()> {
        let policy = self.policy(account_id)?;
        Self::verdict(
            policy.allows_in(mailbox, capability),
            account_id,
            capability,
        )
    }

    /// Moving needs write on both ends: the message leaves one mailbox and
    /// lands in another.
    pub fn authorize_move(
        &self,
        account_id: &AccountId,
        source_mailbox: &str,
        target_mailbox: &str,
    ) -> CoreResult<()> {
        self.authorize_in(account_id, source_mailbox, Capability::Move)?;
        self.authorize_in(account_id, target_mailbox, Capability::Move)
    }

    fn policy(&self, account_id: &AccountId) -> CoreResult<&Policy> {
        self.policies
            .get(account_id)
            .ok_or_else(|| CoreError::AccountNotFound(account_id.clone()))
    }

    fn verdict(allowed: bool, account_id: &AccountId, capability: Capability) -> CoreResult<()> {
        if allowed {
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
    /// Where the message lives — folder rules decide per mailbox, so every
    /// hit has to say which one it came from.
    mailbox: String,
    subject: String,
    sender: String,
    snippet: String,
    date: String,
}

impl SearchHit {
    pub fn new(
        message_id: impl Into<String>,
        mailbox: impl Into<String>,
        subject: impl Into<String>,
        sender: impl Into<String>,
        snippet: impl Into<String>,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            mailbox: mailbox.into(),
            subject: subject.into(),
            sender: sender.into(),
            snippet: snippet.into(),
            date: String::new(),
        }
    }

    /// The `Date` header as the server states it, verbatim. Not every
    /// provider has one, so it is a separate step and empty means unknown —
    /// never a guessed timestamp.
    pub fn with_date(mut self, date: impl Into<String>) -> Self {
        self.date = date.into();
        self
    }

    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    pub fn mailbox(&self) -> &str {
        &self.mailbox
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn date(&self) -> &str {
        &self.date
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

/// The four flag changes assistants may ask for — the two IMAP flags the
/// mark permission covers, each in both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkChange {
    Seen,
    Unseen,
    Flagged,
    Unflagged,
}

impl MarkChange {
    /// The wire names used in tool arguments.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "seen" => Some(Self::Seen),
            "unseen" => Some(Self::Unseen),
            "flagged" => Some(Self::Flagged),
            "unflagged" => Some(Self::Unflagged),
            _ => None,
        }
    }
}

/// One message as a provider stores it. Message data exists for MCP
/// responses, fixtures, and pending-action previews only — never for a
/// human-facing mail surface.
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
    date: String,
    seen: bool,
    flagged: bool,
    attachments: Vec<AttachmentInfo>,
}

/// One attachment as the message lists it: enough for an assistant to decide
/// whether to download, never the bytes. The id is the MIME part path from
/// the provider's walk ("2", "3.1") — the handle `mail_get_attachment` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentInfo {
    id: String,
    filename: String,
    media_type: String,
    size_bytes: usize,
    inline: bool,
}

impl AttachmentInfo {
    pub fn new(
        id: impl Into<String>,
        filename: impl Into<String>,
        media_type: impl Into<String>,
        size_bytes: usize,
        inline: bool,
    ) -> Self {
        Self {
            id: id.into(),
            filename: filename.into(),
            media_type: media_type.into(),
            size_bytes,
            inline,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn size_bytes(&self) -> usize {
        self.size_bytes
    }

    pub fn inline(&self) -> bool {
        self.inline
    }
}

/// The decoded bytes of one downloaded attachment, with the mailbox the
/// message lives in — the fact the folder-scoped policy check needs before
/// the bytes may leave the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentPayload {
    mailbox: String,
    filename: String,
    media_type: String,
    content: Vec<u8>,
}

impl AttachmentPayload {
    pub fn new(
        mailbox: impl Into<String>,
        filename: impl Into<String>,
        media_type: impl Into<String>,
        content: Vec<u8>,
    ) -> Self {
        Self {
            mailbox: mailbox.into(),
            filename: filename.into(),
            media_type: media_type.into(),
            content,
        }
    }

    pub fn mailbox(&self) -> &str {
        &self.mailbox
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn content(&self) -> &[u8] {
        &self.content
    }
}

impl StoredMessage {
    #[allow(clippy::too_many_arguments)]
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
            date: String::new(),
            seen: false,
            flagged: false,
            attachments: Vec::new(),
        }
    }

    /// The `Date` header as the server states it, verbatim. Empty means the
    /// provider has none — never a guessed timestamp.
    pub fn with_date(mut self, date: impl Into<String>) -> Self {
        self.date = date.into();
        self
    }

    /// The attachments the provider's MIME walk found, metadata only.
    pub fn with_attachment_infos(mut self, attachments: Vec<AttachmentInfo>) -> Self {
        self.attachments = attachments;
        self
    }

    pub fn attachments(&self) -> &[AttachmentInfo] {
        &self.attachments
    }

    pub fn mailbox(&self) -> &str {
        &self.mailbox
    }

    pub fn date(&self) -> &str {
        &self.date
    }

    pub fn seen(&self) -> bool {
        self.seen
    }

    pub fn flagged(&self) -> bool {
        self.flagged
    }

    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn sender(&self) -> &str {
        &self.sender
    }

    pub fn snippet(&self) -> &str {
        &self.snippet
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    fn matches_query(&self, query: &str) -> bool {
        let haystack = format!(
            "{} {} {} {}",
            self.subject, self.sender, self.snippet, self.body
        )
        .to_lowercase();
        query
            .split_whitespace()
            .map(str::to_lowercase)
            .all(|token| haystack.contains(&token))
    }

    fn apply(&mut self, change: MarkChange) {
        match change {
            MarkChange::Seen => self.seen = true,
            MarkChange::Unseen => self.seen = false,
            MarkChange::Flagged => self.flagged = true,
            MarkChange::Unflagged => self.flagged = false,
        }
    }
}

/// An optional date window on a search, carried in IMAP's own `DD-Mon-YYYY`
/// form (`08-Jul-2026`). Empty means unbounded. The dates are validated and
/// converted at the MCP boundary, so anything reaching a provider is already
/// well-formed. IMAP compares against each message's internal date, not the
/// `Date` header a sender can fake.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchWindow {
    /// `SINCE` — on or after this date.
    pub since: Option<String>,
    /// `BEFORE` — strictly before this date.
    pub before: Option<String>,
}

impl SearchWindow {
    pub fn is_unbounded(&self) -> bool {
        self.since.is_none() && self.before.is_none()
    }
}

/// One binary file carried by an outgoing MIME message. Construction and
/// limits live at the MCP boundary; the core only needs the already-validated
/// facts required to serialize it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingAttachment {
    filename: String,
    media_type: String,
    content: Vec<u8>,
}

impl OutgoingAttachment {
    pub fn new(
        filename: impl Into<String>,
        media_type: impl Into<String>,
        content: Vec<u8>,
    ) -> Self {
        Self {
            filename: filename.into(),
            media_type: media_type.into(),
            content,
        }
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn content(&self) -> &[u8] {
        &self.content
    }
}

/// A plain-text RFC 5322 message from its parts. The subject is encoded when
/// it leaves ASCII; the body is normalised to CRLF line endings. `Bcc` is
/// deliberately absent from the headers — a blind copy must not be visible to
/// the other recipients; the bcc addresses travel only in the SMTP envelope.
/// Kept as the source-compatible no-attachment doorway.
pub fn compose_message(
    from: &str,
    to: &[String],
    cc: &[String],
    subject: &str,
    body: &str,
) -> String {
    compose_message_with_attachments(from, to, cc, subject, body, &[])
}

/// Compose the complete RFC 5322/MIME message that both IMAP APPEND and SMTP
/// DATA consume. With attachments this becomes `multipart/mixed`; without
/// them it stays byte-for-byte in the original plain-text shape.
pub fn compose_message_with_attachments(
    from: &str,
    to: &[String],
    cc: &[String],
    subject: &str,
    body: &str,
    attachments: &[OutgoingAttachment],
) -> String {
    let mut message = String::new();
    message.push_str(&format!("From: {from}\r\n"));
    message.push_str(&format!("To: {}\r\n", to.join(", ")));
    if !cc.is_empty() {
        message.push_str(&format!("Cc: {}\r\n", cc.join(", ")));
    }
    message.push_str(&format!("Subject: {}\r\n", mime::encode_rfc2047(subject)));
    message.push_str("MIME-Version: 1.0\r\n");

    if attachments.is_empty() {
        append_plain_text_part(&mut message, body);
        return message;
    }

    let boundary = attachment_boundary(body);
    message.push_str(&format!(
        "Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"
    ));

    message.push_str(&format!("--{boundary}\r\n"));
    append_plain_text_part(&mut message, body);

    for attachment in attachments {
        message.push_str(&format!("--{boundary}\r\n"));
        let filename = mime_parameter_value(attachment.filename());
        message.push_str(&format!(
            "Content-Type: {}; name*=UTF-8''{filename}\r\n",
            attachment.media_type()
        ));
        message.push_str(&format!(
            "Content-Disposition: attachment; filename*=UTF-8''{filename}\r\n"
        ));
        message.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
        append_wrapped_base64(&mut message, attachment.content());
    }

    message.push_str(&format!("--{boundary}--\r\n"));
    message
}

fn append_plain_text_part(message: &mut String, body: &str) {
    message.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    message.push_str("Content-Transfer-Encoding: 8bit\r\n");
    message.push_str("\r\n");
    for line in body.split('\n') {
        message.push_str(line.strip_suffix('\r').unwrap_or(line));
        message.push_str("\r\n");
    }
}

/// Pick a deterministic boundary that cannot occur as a delimiter in the
/// only unencoded part. Attachment bodies are base64 and therefore cannot
/// contain the hyphens in this marker.
fn attachment_boundary(body: &str) -> String {
    let mut suffix = 1_u64;
    loop {
        let boundary = format!("=_TorroMail_mixed_{suffix}");
        if !body.contains(&format!("--{boundary}")) {
            return boundary;
        }
        suffix += 1;
    }
}

/// RFC 2231 extended parameter value. The MCP boundary has already rejected
/// controls and path separators; percent encoding makes every remaining UTF-8
/// filename safe inside a MIME header without lossy ASCII fallbacks.
fn mime_parameter_value(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn append_wrapped_base64(message: &mut String, content: &[u8]) {
    let encoded = mime::encode_base64(content);
    if encoded.is_empty() {
        message.push_str("\r\n");
        return;
    }
    for line in encoded.as_bytes().chunks(76) {
        for &byte in line {
            message.push(char::from(byte));
        }
        message.push_str("\r\n");
    }
}

/// The boundary fixture-backed tests and real IMAP retrieval share: search
/// and single-message fetch, nothing that smells like an inbox.
pub trait MailProvider {
    /// A blank query means "no filter": the newest messages the mailbox has.
    /// That is how "what came in lately?" is answered without a browsing
    /// tool — the caller's `limit` cuts off the oldest, never the newest. A
    /// `window` narrows the same search to a date range.
    fn search(
        &self,
        account_id: &AccountId,
        query: &str,
        mailbox: Option<&str>,
        limit: usize,
        window: &SearchWindow,
    ) -> CoreResult<Vec<SearchHit>>;

    fn get_message(&self, account_id: &AccountId, message_id: &str) -> CoreResult<StoredMessage>;

    /// The decoded bytes of one attachment, id as listed on the stored
    /// message. Providers answer `AttachmentNotFound` for an id no part
    /// carries.
    fn get_attachment(
        &self,
        account_id: &AccountId,
        message_id: &str,
        attachment_id: &str,
    ) -> CoreResult<AttachmentPayload>;

    /// Every message in the conversation `thread_id` names, oldest first.
    /// Plain IMAP has no thread identity, so a provider assembles this from
    /// the `References`/`In-Reply-To` headers; a mailbox that cannot relate
    /// two messages answers with the one it was asked about.
    fn get_thread(
        &self,
        account_id: &AccountId,
        thread_id: &str,
    ) -> CoreResult<Vec<StoredMessage>>;

    fn mark(
        &mut self,
        account_id: &AccountId,
        message_id: &str,
        change: MarkChange,
    ) -> CoreResult<()>;

    fn list_mailboxes(&self, account_id: &AccountId) -> CoreResult<Vec<String>>;

    /// Store a ready-made message in `mailbox` as a draft — an IMAP APPEND
    /// with the `\Draft` flag, no sending involved.
    fn append_draft(
        &mut self,
        account_id: &AccountId,
        mailbox: &str,
        message: &str,
    ) -> CoreResult<()>;

    /// Move each message to `target`. A soft delete is one of these — a move
    /// to the Trash folder.
    fn move_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
        target: &str,
    ) -> CoreResult<()>;

    /// Delete each message for good — flagged and expunged, no Trash.
    fn expunge_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
    ) -> CoreResult<()>;
}

/// A `&mut` to a provider is itself a provider. This is what lets a caller
/// keep a long-lived connection in a pool and lend it to `MailAccessService`
/// for one operation without giving up ownership — `?Sized` so it covers
/// `&mut dyn MailProvider`, the form a boxed, pooled connection takes.
impl<T: MailProvider + ?Sized> MailProvider for &mut T {
    fn search(
        &self,
        account_id: &AccountId,
        query: &str,
        mailbox: Option<&str>,
        limit: usize,
        window: &SearchWindow,
    ) -> CoreResult<Vec<SearchHit>> {
        (**self).search(account_id, query, mailbox, limit, window)
    }

    fn get_message(&self, account_id: &AccountId, message_id: &str) -> CoreResult<StoredMessage> {
        (**self).get_message(account_id, message_id)
    }

    fn get_attachment(
        &self,
        account_id: &AccountId,
        message_id: &str,
        attachment_id: &str,
    ) -> CoreResult<AttachmentPayload> {
        (**self).get_attachment(account_id, message_id, attachment_id)
    }

    fn get_thread(
        &self,
        account_id: &AccountId,
        thread_id: &str,
    ) -> CoreResult<Vec<StoredMessage>> {
        (**self).get_thread(account_id, thread_id)
    }

    fn mark(
        &mut self,
        account_id: &AccountId,
        message_id: &str,
        change: MarkChange,
    ) -> CoreResult<()> {
        (**self).mark(account_id, message_id, change)
    }

    fn list_mailboxes(&self, account_id: &AccountId) -> CoreResult<Vec<String>> {
        (**self).list_mailboxes(account_id)
    }

    fn append_draft(
        &mut self,
        account_id: &AccountId,
        mailbox: &str,
        message: &str,
    ) -> CoreResult<()> {
        (**self).append_draft(account_id, mailbox, message)
    }

    fn move_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
        target: &str,
    ) -> CoreResult<()> {
        (**self).move_messages(account_id, message_ids, target)
    }

    fn expunge_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
    ) -> CoreResult<()> {
        (**self).expunge_messages(account_id, message_ids)
    }
}

#[derive(Debug, Clone, Default)]
pub struct FixtureMailProvider {
    messages: Vec<StoredMessage>,
    /// Attachment bytes by message id and attachment id: the fixture's stand-in
    /// for what a real provider extracts from the raw MIME body.
    attachment_payloads: BTreeMap<(String, String), (String, String, Vec<u8>)>,
}

impl FixtureMailProvider {
    pub fn new(messages: impl IntoIterator<Item = StoredMessage>) -> Self {
        Self {
            messages: messages.into_iter().collect(),
            attachment_payloads: BTreeMap::new(),
        }
    }

    /// Seed the bytes behind one listed attachment.
    pub fn with_attachment(
        mut self,
        message_id: &str,
        attachment_id: &str,
        filename: &str,
        media_type: &str,
        content: Vec<u8>,
    ) -> Self {
        self.attachment_payloads.insert(
            (message_id.to_owned(), attachment_id.to_owned()),
            (filename.to_owned(), media_type.to_owned(), content),
        );
        self
    }
}

impl MailProvider for FixtureMailProvider {
    /// The date window is a real IMAP server concern; the fixture models
    /// message content, not internal dates, so it searches the whole set and
    /// leaves date filtering to the live provider the tests below exercise
    /// separately.
    fn search(
        &self,
        account_id: &AccountId,
        query: &str,
        mailbox: Option<&str>,
        limit: usize,
        _window: &SearchWindow,
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
                    message.mailbox(),
                    message.subject(),
                    message.sender(),
                    message.snippet(),
                )
                .with_date(message.date())
            })
            .collect())
    }

    fn get_message(&self, account_id: &AccountId, message_id: &str) -> CoreResult<StoredMessage> {
        self.messages
            .iter()
            .find(|message| &message.account_id == account_id && message.message_id == message_id)
            .cloned()
            .ok_or_else(|| CoreError::MessageNotFound(message_id.to_owned()))
    }

    fn get_attachment(
        &self,
        account_id: &AccountId,
        message_id: &str,
        attachment_id: &str,
    ) -> CoreResult<AttachmentPayload> {
        // The message decides existence and mailbox; the payload map only
        // holds the bytes.
        let message = self.get_message(account_id, message_id)?;
        let (filename, media_type, content) = self
            .attachment_payloads
            .get(&(message_id.to_owned(), attachment_id.to_owned()))
            .cloned()
            .ok_or_else(|| CoreError::AttachmentNotFound {
                message_id: message_id.to_owned(),
                attachment_id: attachment_id.to_owned(),
            })?;
        Ok(AttachmentPayload::new(
            message.mailbox(),
            filename,
            media_type,
            content,
        ))
    }

    fn get_thread(
        &self,
        account_id: &AccountId,
        thread_id: &str,
    ) -> CoreResult<Vec<StoredMessage>> {
        let messages: Vec<StoredMessage> = self
            .messages
            .iter()
            .filter(|message| &message.account_id == account_id && message.thread_id == thread_id)
            .cloned()
            .collect();

        if messages.is_empty() {
            return Err(CoreError::MessageNotFound(thread_id.to_owned()));
        }
        Ok(messages)
    }

    fn mark(
        &mut self,
        account_id: &AccountId,
        message_id: &str,
        change: MarkChange,
    ) -> CoreResult<()> {
        let message = self
            .messages
            .iter_mut()
            .find(|message| &message.account_id == account_id && message.message_id == message_id)
            .ok_or_else(|| CoreError::MessageNotFound(message_id.to_owned()))?;
        message.apply(change);
        Ok(())
    }

    fn list_mailboxes(&self, account_id: &AccountId) -> CoreResult<Vec<String>> {
        let mut mailboxes = Vec::new();
        for message in &self.messages {
            if &message.account_id == account_id && !mailboxes.contains(&message.mailbox) {
                mailboxes.push(message.mailbox.clone());
            }
        }
        Ok(mailboxes)
    }

    /// The fixture keeps the appended draft so a later search or list can see
    /// it — the raw message becomes the body, enough to prove it landed.
    fn append_draft(
        &mut self,
        account_id: &AccountId,
        mailbox: &str,
        message: &str,
    ) -> CoreResult<()> {
        let id = format!("draft-{}", self.messages.len() + 1);
        self.messages.push(StoredMessage::new(
            account_id.clone(),
            mailbox,
            id.clone(),
            id,
            "Draft",
            "",
            "",
            message,
        ));
        Ok(())
    }

    fn move_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
        target: &str,
    ) -> CoreResult<()> {
        for message_id in message_ids {
            let message = self
                .messages
                .iter_mut()
                .find(|message| {
                    &message.account_id == account_id && &message.message_id == message_id
                })
                .ok_or_else(|| CoreError::MessageNotFound(message_id.clone()))?;
            message.mailbox = target.to_owned();
        }
        Ok(())
    }

    fn expunge_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
    ) -> CoreResult<()> {
        for message_id in message_ids {
            let before = self.messages.len();
            self.messages
                .retain(|message| !(&message.account_id == account_id && &message.message_id == message_id));
            if self.messages.len() == before {
                return Err(CoreError::MessageNotFound(message_id.clone()));
            }
        }
        Ok(())
    }
}

/// The policy-checked doorway between MCP tools and a mail provider. Every
/// read passes the permission groups twice: account-wide, and again for the
/// mailbox the data actually lives in.
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
        window: &SearchWindow,
        now: u64,
    ) -> CoreResult<SearchResultSet> {
        match mailbox {
            Some(name) => self
                .policy_engine
                .authorize_in(account_id, name, Capability::Search)?,
            None => self
                .policy_engine
                .authorize(account_id, Capability::Search)?,
        }

        // A search across everything must not leak hits from folders the
        // policy blocks, so every hit answers for its own mailbox.
        let policy = self.policy_engine.policy(account_id)?;
        let hits = self
            .provider
            .search(account_id, query, mailbox, limit, window)?
            .into_iter()
            .filter(|hit| policy.allows_in(hit.mailbox(), Capability::Search))
            .collect();

        Ok(self
            .sessions
            .create(account_id.clone(), query, hits, now, 7200))
    }

    pub fn get_message(
        &self,
        account_id: &AccountId,
        message_id: &str,
        include_body: bool,
    ) -> CoreResult<StoredMessage> {
        self.policy_engine
            .authorize(account_id, Capability::ReadHeaders)?;
        let message = self.provider.get_message(account_id, message_id)?;

        // The folder decides again: a blocked mailbox hides even headers.
        self.policy_engine
            .authorize_in(account_id, message.mailbox(), Capability::ReadHeaders)?;

        if include_body {
            self.policy_engine
                .authorize_in(account_id, message.mailbox(), Capability::ReadBody)?;
            return Ok(message);
        }

        // Below ReadBody the message keeps its metadata and loses its
        // content.
        let mut header_only = message;
        header_only.body = String::new();
        Ok(header_only)
    }

    /// One attachment's bytes, authorized twice like every read: the cheap
    /// account-wide check refuses before any fetch, and the mailbox the
    /// provider reports is checked again before the bytes leave the service.
    pub fn get_attachment(
        &self,
        account_id: &AccountId,
        message_id: &str,
        attachment_id: &str,
    ) -> CoreResult<AttachmentPayload> {
        self.policy_engine
            .authorize(account_id, Capability::DownloadAttachments)?;
        let payload = self
            .provider
            .get_attachment(account_id, message_id, attachment_id)?;
        self.policy_engine.authorize_in(
            account_id,
            payload.mailbox(),
            Capability::DownloadAttachments,
        )?;
        Ok(payload)
    }

    /// Whether a download from `mailbox` would be allowed — the fact the
    /// attachment listing carries so an assistant can ask for the right
    /// permission instead of guessing.
    pub fn download_allowed(&self, account_id: &AccountId, mailbox: &str) -> bool {
        self.policy_engine
            .authorize_in(account_id, mailbox, Capability::DownloadAttachments)
            .is_ok()
    }

    /// A thread, folder-checked message by message. A conversation can span
    /// folders — a reply filed away, the original in the inbox — so any
    /// message in a blocked folder simply drops out rather than sinking the
    /// whole thread, and bodies follow the same `include_bodies` rule as a
    /// single fetch.
    pub fn get_thread(
        &self,
        account_id: &AccountId,
        thread_id: &str,
        include_bodies: bool,
    ) -> CoreResult<Vec<StoredMessage>> {
        self.policy_engine
            .authorize(account_id, Capability::ReadHeaders)?;
        let messages = self.provider.get_thread(account_id, thread_id)?;

        let mut visible = Vec::new();
        for mut message in messages {
            if self
                .policy_engine
                .authorize_in(account_id, message.mailbox(), Capability::ReadHeaders)
                .is_err()
            {
                continue;
            }

            let keep_body = include_bodies
                && self
                    .policy_engine
                    .authorize_in(account_id, message.mailbox(), Capability::ReadBody)
                    .is_ok();
            if !keep_body {
                message.body = String::new();
            }
            visible.push(message);
        }
        Ok(visible)
    }

    /// Store a composed message as a draft, if the account may draft at all.
    /// Drafting is an account-wide right, not folder-scoped: it always lands
    /// in the drafts mailbox, which the caller names.
    pub fn create_draft(
        &mut self,
        account_id: &AccountId,
        mailbox: &str,
        message: &str,
    ) -> CoreResult<()> {
        self.policy_engine
            .authorize(account_id, Capability::Draft)?;
        self.provider.append_draft(account_id, mailbox, message)
    }

    /// Move messages, each authorized on both ends: a message can only leave a
    /// folder the account may write and land in one it may write too. The
    /// source folder is the message's own, found by a lookup, not whatever the
    /// caller claims.
    pub fn move_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
        target: &str,
    ) -> CoreResult<()> {
        for message_id in message_ids {
            let message = self.provider.get_message(account_id, message_id)?;
            self.policy_engine
                .authorize_move(account_id, message.mailbox(), target)?;
        }
        self.provider.move_messages(account_id, message_ids, target)
    }

    /// Soft-delete messages: a move to the Trash folder, gated by the trash
    /// right in each message's own folder.
    pub fn trash_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
        trash: &str,
    ) -> CoreResult<()> {
        for message_id in message_ids {
            let message = self.provider.get_message(account_id, message_id)?;
            self.policy_engine
                .authorize_in(account_id, message.mailbox(), Capability::DeleteSoft)?;
        }
        self.provider.move_messages(account_id, message_ids, trash)
    }

    /// Permanently delete messages, gated by the permanent-delete right — which
    /// cannot outlive the trash right it escalates — in each message's folder.
    pub fn expunge_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
    ) -> CoreResult<()> {
        for message_id in message_ids {
            let message = self.provider.get_message(account_id, message_id)?;
            self.policy_engine
                .authorize_in(account_id, message.mailbox(), Capability::DeletePermanent)?;
        }
        self.provider.expunge_messages(account_id, message_ids)
    }

    /// Only folders the policy grants anything on exist for assistants —
    /// even the existence of a blocked folder is nobody's business.
    pub fn list_mailboxes(&self, account_id: &AccountId) -> CoreResult<Vec<String>> {
        let policy = self.policy_engine.policy(account_id)?;
        Ok(self
            .provider
            .list_mailboxes(account_id)?
            .into_iter()
            .filter(|mailbox| policy.permissions().can_access(mailbox))
            .collect())
    }

    /// Marking is folder-scoped like every mailbox mutation — the message's
    /// own mailbox decides, not whatever mailbox the caller claims.
    pub fn mark(
        &mut self,
        account_id: &AccountId,
        message_id: &str,
        change: MarkChange,
    ) -> CoreResult<StoredMessage> {
        let message = self.provider.get_message(account_id, message_id)?;
        self.policy_engine
            .authorize_in(account_id, message.mailbox(), Capability::Mark)?;
        self.provider.mark(account_id, message_id, change)?;
        self.provider.get_message(account_id, message_id)
    }
}
