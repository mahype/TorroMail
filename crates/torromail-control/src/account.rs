//! One configured mail account, in the shape `state.json` stores it.
//!
//! The field names and the legacy fallbacks mirror `MailAccount`'s `Codable`
//! conformance in TorroMailKit exactly: both sides read and write the same
//! file, so a record one of them wrote must round-trip through the other
//! untouched. Secrets are never here — only the keychain holds those.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::FormatError;

/// How deep assistants may read. The levels build on each other; stored as
/// its rank, published by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadAccess {
    None,
    Headers,
    FullMessage,
    WithAttachments,
}

impl ReadAccess {
    #[must_use]
    pub fn rank(self) -> u64 {
        match self {
            Self::None => 0,
            Self::Headers => 1,
            Self::FullMessage => 2,
            Self::WithAttachments => 3,
        }
    }

    #[must_use]
    pub fn from_rank(rank: u64) -> Option<Self> {
        match rank {
            0 => Some(Self::None),
            1 => Some(Self::Headers),
            2 => Some(Self::FullMessage),
            3 => Some(Self::WithAttachments),
            _ => None,
        }
    }

    /// The word the policy document uses.
    #[must_use]
    pub fn policy_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Headers => "headers",
            Self::FullMessage => "full_message",
            Self::WithAttachments => "with_attachments",
        }
    }
}

/// Mailbox mutations that stay on the server. Sending is not in here: it is
/// the one right that acts on the outside world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteAccess {
    pub drafts: bool,
    pub mark: bool,
    pub r#move: bool,
    pub trash: bool,
    /// Escalation of `trash`, never part of a preset.
    pub permanent_delete: bool,
}

/// Per-mailbox exception to the account-wide groups. `true` means "the group
/// applies here", so a folder can never allow more than the account does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderRule {
    pub read: bool,
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionSet {
    pub read: ReadAccess,
    pub write: WriteAccess,
    pub send: bool,
    /// Whether folder exceptions apply. Off keeps stored exceptions, inert.
    pub per_folder: bool,
    pub folder_rules: BTreeMap<String, FolderRule>,
}

impl Default for PermissionSet {
    /// The everyday default: read whole messages, draft, nothing else.
    fn default() -> Self {
        Self {
            read: ReadAccess::FullMessage,
            write: WriteAccess { drafts: true, ..WriteAccess::default() },
            send: false,
            per_folder: false,
            folder_rules: BTreeMap::new(),
        }
    }
}

/// How much of an account may rest on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CacheLevel {
    Off,
    Headers,
    Bodies,
    Attachments,
}

impl CacheLevel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Headers => "headers",
            Self::Bodies => "bodies",
            Self::Attachments => "attachments",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "off" => Some(Self::Off),
            "headers" => Some(Self::Headers),
            "bodies" => Some(Self::Bodies),
            "attachments" => Some(Self::Attachments),
            _ => None,
        }
    }

    /// The most the read permission justifies keeping on disk.
    #[must_use]
    pub fn ceiling(read: ReadAccess) -> Self {
        match read {
            ReadAccess::None => Self::Off,
            ReadAccess::Headers => Self::Headers,
            ReadAccess::FullMessage => Self::Bodies,
            ReadAccess::WithAttachments => Self::Attachments,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionSecurity {
    /// TLS from the first byte.
    Tls,
    /// Plaintext until the `STARTTLS` upgrade, before any credential moves.
    StartTls,
}

impl ConnectionSecurity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "tls" => Some(Self::Tls),
            "starttls" => Some(Self::StartTls),
            _ => None,
        }
    }

    /// What the port used to imply, for records written before the
    /// encryption was stated. Keeps an old account connecting as it did.
    #[must_use]
    pub fn implied_by_imap_port(port: u16) -> Self {
        if port == 993 { Self::Tls } else { Self::StartTls }
    }

    #[must_use]
    pub fn implied_by_smtp_port(port: u16) -> Self {
        if port == 465 { Self::Tls } else { Self::StartTls }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMethod {
    Password,
    OAuth,
}

impl LoginMethod {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::OAuth => "oauth",
        }
    }
}

/// The two token issuers TorroMail speaks to, both as public clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthIssuer {
    Google,
    Microsoft,
}

impl OAuthIssuer {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "google" => Some(Self::Google),
            "microsoft" => Some(Self::Microsoft),
            _ => None,
        }
    }

    #[must_use]
    pub fn token_endpoint(self) -> &'static str {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        }
    }
}

/// Manual choices for the five special-use folders. An empty name keeps that
/// role automatic even while `manual` is on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpecialMailboxes {
    pub manual: bool,
    pub drafts: String,
    pub sent: String,
    pub archive: String,
    pub junk: String,
    pub trash: String,
}

impl SpecialMailboxes {
    /// What the policy document carries: only roles chosen by hand.
    #[must_use]
    pub fn policy_overrides(&self) -> BTreeMap<&'static str, &str> {
        if !self.manual {
            return BTreeMap::new();
        }
        [
            ("drafts", self.drafts.as_str()),
            ("sent", self.sent.as_str()),
            ("archive", self.archive.as_str()),
            ("junk", self.junk.as_str()),
            ("trash", self.trash.as_str()),
        ]
        .into_iter()
        .filter(|(_, name)| !name.is_empty())
        .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailAccount {
    pub id: String,
    pub name: String,
    pub email: String,
    /// The provider label as stored (`"IMAP/SMTP"`, `"Gmail"`, …). Kept as
    /// text: nothing here decides on it, and a label this build has never
    /// heard of must survive a round trip.
    pub provider: String,
    pub login_method: LoginMethod,
    pub oauth_issuer: Option<OAuthIssuer>,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: ConnectionSecurity,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: ConnectionSecurity,
    pub username: String,
    /// Whether the last connection test succeeded. The live status comes from
    /// the health log; this is only what a fresh start falls back to.
    pub is_verified: bool,
    pub known_mailboxes: Vec<String>,
    pub permissions: PermissionSet,
    pub special_mailboxes: SpecialMailboxes,
    pub cache_level: CacheLevel,
}

impl MailAccount {
    /// Whether this account carries the facts a mailbox can be opened from —
    /// the line between an account the server can serve and one it refuses as
    /// unconfigured.
    #[must_use]
    pub fn has_imap_connection(&self) -> bool {
        !self.imap_host.is_empty() && !self.username.is_empty()
    }

    #[must_use]
    pub fn has_smtp_connection(&self) -> bool {
        !self.smtp_host.is_empty() && !self.username.is_empty()
    }

    pub fn from_json(value: &Value) -> Result<Self, FormatError> {
        let object = value
            .as_object()
            .ok_or_else(|| FormatError("an account is not an object".to_owned()))?;
        let id = required_str(object, "id", "account")?;
        let context = format!("account `{id}`");

        // Ports first: a record from before the encryption was stated falls
        // back to what its port implied, or a 587 account would suddenly be
        // asked for implicit TLS.
        let imap_port = optional_port(object, "imapPort", &context)?.unwrap_or(993);
        let smtp_port = optional_port(object, "smtpPort", &context)?.unwrap_or(587);

        Ok(Self {
            name: required_str(object, "name", &context)?,
            email: required_str(object, "email", &context)?,
            provider: required_str(object, "provider", &context)?,
            login_method: match required_str(object, "loginMethod", &context)?.as_str() {
                "password" => LoginMethod::Password,
                "oauth" => LoginMethod::OAuth,
                other => return Err(FormatError(format!("{context}: unknown loginMethod `{other}`"))),
            },
            oauth_issuer: match object.get("oauthIssuer").and_then(Value::as_str) {
                None => None,
                Some(word) => Some(
                    OAuthIssuer::parse(word)
                        .ok_or_else(|| FormatError(format!("{context}: unknown oauthIssuer `{word}`")))?,
                ),
            },
            imap_host: required_str(object, "imapHost", &context)?,
            imap_port,
            imap_security: optional_security(object, "imapSecurity", &context)?
                .unwrap_or_else(|| ConnectionSecurity::implied_by_imap_port(imap_port)),
            smtp_host: required_str(object, "smtpHost", &context)?,
            smtp_port,
            smtp_security: optional_security(object, "smtpSecurity", &context)?
                .unwrap_or_else(|| ConnectionSecurity::implied_by_smtp_port(smtp_port)),
            username: required_str(object, "username", &context)?,
            is_verified: object.get("isVerified").and_then(Value::as_bool).unwrap_or(false),
            known_mailboxes: object
                .get("knownMailboxes")
                .and_then(Value::as_array)
                .ok_or_else(|| FormatError(format!("{context}: knownMailboxes is missing")))?
                .iter()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect(),
            permissions: permissions_from_json(
                object
                    .get("permissions")
                    .ok_or_else(|| FormatError(format!("{context}: permissions is missing")))?,
                &context,
            )?,
            special_mailboxes: object
                .get("specialMailboxes")
                .map(special_mailboxes_from_json)
                .unwrap_or_default(),
            cache_level: cache_level_from_json(
                object
                    .get("searchCache")
                    .ok_or_else(|| FormatError(format!("{context}: searchCache is missing")))?,
                &context,
            )?,
            id,
        })
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut object = Map::new();
        object.insert("id".into(), json!(self.id));
        object.insert("name".into(), json!(self.name));
        object.insert("email".into(), json!(self.email));
        object.insert("provider".into(), json!(self.provider));
        object.insert("loginMethod".into(), json!(self.login_method.as_str()));
        if let Some(issuer) = self.oauth_issuer {
            object.insert("oauthIssuer".into(), json!(issuer.as_str()));
        }
        object.insert("imapHost".into(), json!(self.imap_host));
        object.insert("imapPort".into(), json!(self.imap_port));
        object.insert("imapSecurity".into(), json!(self.imap_security.as_str()));
        object.insert("smtpHost".into(), json!(self.smtp_host));
        object.insert("smtpPort".into(), json!(self.smtp_port));
        object.insert("smtpSecurity".into(), json!(self.smtp_security.as_str()));
        object.insert("username".into(), json!(self.username));
        object.insert("knownMailboxes".into(), json!(self.known_mailboxes));
        object.insert("permissions".into(), permissions_to_json(&self.permissions));
        object.insert(
            "specialMailboxes".into(),
            json!({
                "manual": self.special_mailboxes.manual,
                "drafts": self.special_mailboxes.drafts,
                "sent": self.special_mailboxes.sent,
                "archive": self.special_mailboxes.archive,
                "junk": self.special_mailboxes.junk,
                "trash": self.special_mailboxes.trash,
            }),
        );
        object.insert("searchCache".into(), json!({ "level": self.cache_level.as_str() }));
        object.insert("isVerified".into(), json!(self.is_verified));
        Value::Object(object)
    }
}

fn required_str(object: &Map<String, Value>, key: &str, context: &str) -> Result<String, FormatError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| FormatError(format!("{context}: {key} is missing or not text")))
}

fn optional_port(object: &Map<String, Value>, key: &str, context: &str) -> Result<Option<u16>, FormatError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .map(Some)
            .ok_or_else(|| FormatError(format!("{context}: {key} is not a port"))),
    }
}

fn optional_security(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Option<ConnectionSecurity>, FormatError> {
    match object.get(key).and_then(Value::as_str) {
        None => Ok(None),
        Some(word) => ConnectionSecurity::parse(word)
            .map(Some)
            .ok_or_else(|| FormatError(format!("{context}: unknown {key} `{word}`"))),
    }
}

fn permissions_from_json(value: &Value, context: &str) -> Result<PermissionSet, FormatError> {
    let flag = |object: &Value, key: &str| object.get(key).and_then(Value::as_bool).unwrap_or(false);
    let read = value
        .get("read")
        .and_then(Value::as_u64)
        .and_then(ReadAccess::from_rank)
        .ok_or_else(|| FormatError(format!("{context}: permissions.read is not a known level")))?;
    let write = value.get("write").cloned().unwrap_or(Value::Null);
    let folder_rules = value
        .get("folderRules")
        .and_then(Value::as_object)
        .map(|rules| {
            rules
                .iter()
                .map(|(mailbox, rule)| {
                    (
                        mailbox.clone(),
                        FolderRule {
                            read: rule.get("read").and_then(Value::as_bool).unwrap_or(true),
                            write: rule.get("write").and_then(Value::as_bool).unwrap_or(true),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(PermissionSet {
        read,
        write: WriteAccess {
            drafts: flag(&write, "drafts"),
            mark: flag(&write, "mark"),
            r#move: flag(&write, "move"),
            trash: flag(&write, "trash"),
            permanent_delete: flag(&write, "permanentDelete"),
        },
        send: flag(value, "send"),
        per_folder: flag(value, "perFolder"),
        folder_rules,
    })
}

fn permissions_to_json(permissions: &PermissionSet) -> Value {
    let rules: Map<String, Value> = permissions
        .folder_rules
        .iter()
        .map(|(mailbox, rule)| (mailbox.clone(), json!({ "read": rule.read, "write": rule.write })))
        .collect();
    json!({
        "read": permissions.read.rank(),
        "write": {
            "drafts": permissions.write.drafts,
            "mark": permissions.write.mark,
            "move": permissions.write.r#move,
            "trash": permissions.write.trash,
            "permanentDelete": permissions.write.permanent_delete,
        },
        "send": permissions.send,
        "perFolder": permissions.per_folder,
        "folderRules": rules,
    })
}

fn special_mailboxes_from_json(value: &Value) -> SpecialMailboxes {
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    SpecialMailboxes {
        manual: value.get("manual").and_then(Value::as_bool).unwrap_or(false),
        drafts: text("drafts"),
        sent: text("sent"),
        archive: text("archive"),
        junk: text("junk"),
        trash: text("trash"),
    }
}

/// `{"level": …}` today; an earlier build stored two switches, mapped here the
/// way the server maps a legacy policy document so an upgrade keeps the
/// user's decision.
fn cache_level_from_json(value: &Value, context: &str) -> Result<CacheLevel, FormatError> {
    if let Some(word) = value.get("level").and_then(Value::as_str) {
        return CacheLevel::parse(word)
            .ok_or_else(|| FormatError(format!("{context}: unknown cache level `{word}`")));
    }
    let enabled = value.get("localCacheEnabled").and_then(Value::as_bool).unwrap_or(true);
    let mode = value.get("cacheMode").and_then(Value::as_str).unwrap_or("metadata");
    Ok(if !enabled {
        CacheLevel::Off
    } else if mode == "body" || mode == "fullText" {
        CacheLevel::Bodies
    } else {
        CacheLevel::Headers
    })
}

/// The named starting points. A preset is a fact about the current values —
/// derived, never stored — so a "custom" state can never drift out of sync
/// with the switches. Folder exceptions do not count: presets decide the
/// groups, exceptions only scope them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPreset {
    ReadOnly,
    ReadAndDrafts,
    TidyUp,
    FullAccess,
}

impl PermissionPreset {
    pub const ALL: [Self; 4] = [Self::ReadOnly, Self::ReadAndDrafts, Self::TidyUp, Self::FullAccess];

    #[must_use]
    pub fn read(self) -> ReadAccess {
        if self == Self::FullAccess { ReadAccess::WithAttachments } else { ReadAccess::FullMessage }
    }

    #[must_use]
    pub fn write(self) -> WriteAccess {
        match self {
            Self::ReadOnly => WriteAccess::default(),
            Self::ReadAndDrafts => WriteAccess { drafts: true, ..WriteAccess::default() },
            Self::TidyUp => WriteAccess { mark: true, r#move: true, trash: true, ..WriteAccess::default() },
            // Everything except permanent deletion, which is never preset.
            Self::FullAccess => WriteAccess { drafts: true, mark: true, r#move: true, trash: true, permanent_delete: false },
        }
    }

    #[must_use]
    pub fn send(self) -> bool {
        self == Self::FullAccess
    }
}

impl PermissionSet {
    #[must_use]
    pub fn matching_preset(&self) -> Option<PermissionPreset> {
        PermissionPreset::ALL
            .into_iter()
            .find(|preset| preset.read() == self.read && preset.write() == self.write && preset.send() == self.send)
    }
}
