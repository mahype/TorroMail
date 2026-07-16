//! The internal policy document the app publishes.
//!
//! The SwiftUI app writes this JSON whenever account permissions change;
//! every `torromail-mcp` instance — no matter who spawned it — reloads it
//! per tool call, so a switch flipped in the UI applies immediately. The
//! format mirrors `PermissionSet` on both sides:
//!
//! ```json
//! {
//!   "version": 1,
//!   "accounts": [{
//!     "id": "work",
//!     "name": "Work",
//!     "email": "me@example.com",
//!     "read": "full_message",
//!     "write": {"drafts": true, "mark": false, "move": false,
//!               "trash": false, "permanent_delete": false},
//!     "send": false,
//!     "per_folder": true,
//!     "folder_rules": {"Private": {"read": false, "write": false}},
//!     "cache": {"local_cache_enabled": true, "mode": "metadata",
//!               "index_bodies": false, "index_attachments": false,
//!               "storage": "0 MB"},
//!     "imap": {"host": "imap.gmail.com", "port": 993,
//!              "username": "me@example.com",
//!              "secret_ref": "keychain://TorroMail/work",
//!              "auth": "xoauth2",
//!              "token_endpoint": "https://oauth2.googleapis.com/token",
//!              "client_id": "…apps.googleusercontent.com"}
//!   }]
//! }
//! ```
//!
//! Anything unreadable fails closed: a broken document grants nothing.

use std::collections::BTreeMap;

use serde_json::Value;
use torromail_core::{
    AccountId, FolderRule, ImapAuth, ImapProviderConfig, PermissionSet, Policy, ReadAccess,
    SecretRef, WriteAccess,
};

/// One account as the document describes it: its permissions, how the admin
/// tools name it, what it caches, and — once the app has connection facts —
/// how to reach the real mailbox.
pub(crate) struct DocumentAccount {
    pub(crate) policy: Policy,
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) cache: CacheFacts,
    pub(crate) imap: Option<ImapProviderConfig>,
    /// Present only for `xoauth2` accounts: what it takes to renew the token
    /// when the stored one has gone stale.
    pub(crate) oauth: Option<OAuthFacts>,
}

/// What the app decided to keep on disk for this account. The server does
/// not act on these — it reports them, so an assistant can say why a search
/// is slow or a body is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheFacts {
    pub(crate) local_cache_enabled: bool,
    pub(crate) mode: String,
    pub(crate) index_bodies: bool,
    pub(crate) index_attachments: bool,
    pub(crate) storage: String,
}

impl Default for CacheFacts {
    /// What an account looks like before the app has published cache facts:
    /// the product default from `CachePolicy` — metadata only, no index.
    fn default() -> Self {
        Self {
            local_cache_enabled: true,
            mode: "metadata".to_owned(),
            index_bodies: false,
            index_attachments: false,
            storage: "unknown".to_owned(),
        }
    }
}

pub(crate) fn parse_policy_document(text: &str) -> Result<Vec<DocumentAccount>, String> {
    let document: Value =
        serde_json::from_str(text).map_err(|error| format!("policy document invalid: {error}"))?;
    let accounts = document["accounts"]
        .as_array()
        .ok_or("policy document invalid: accounts must be an array")?;

    accounts.iter().map(parse_account).collect()
}

fn parse_account(account: &Value) -> Result<DocumentAccount, String> {
    let policy = parse_account_policy(account)?;
    let (imap, oauth) = match account.get("imap") {
        None | Some(Value::Null) => (None, None),
        Some(imap) => (
            Some(parse_imap_config(account, imap)?),
            parse_oauth_facts(imap)?,
        ),
    };

    // Name and email are labels, not rights: a document from an older app
    // build simply has none, and the account still works.
    let name = account["name"].as_str().unwrap_or_default().to_owned();
    let email = account["email"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| imap.as_ref().map(|config| config.username.clone()))
        .unwrap_or_default();

    Ok(DocumentAccount {
        policy,
        name,
        email,
        cache: parse_cache_facts(&account["cache"]),
        imap,
        oauth,
    })
}

fn parse_cache_facts(cache: &Value) -> CacheFacts {
    let default = CacheFacts::default();
    let Some(cache) = cache.as_object() else {
        return default;
    };

    CacheFacts {
        local_cache_enabled: cache["local_cache_enabled"]
            .as_bool()
            .unwrap_or(default.local_cache_enabled),
        mode: cache["mode"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or(default.mode),
        index_bodies: cache["index_bodies"]
            .as_bool()
            .unwrap_or(default.index_bodies),
        index_attachments: cache["index_attachments"]
            .as_bool()
            .unwrap_or(default.index_attachments),
        storage: cache["storage"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or(default.storage),
    }
}

/// A present-but-broken connection block is an error, never a silent
/// fixture fallback — that would misreport whose data is being served.
fn parse_imap_config(account: &Value, imap: &Value) -> Result<ImapProviderConfig, String> {
    let id = account["id"]
        .as_str()
        .ok_or("policy document invalid: account id must be a string")?;
    let host = imap["host"]
        .as_str()
        .ok_or("policy document invalid: imap host must be a string")?;
    let username = imap["username"]
        .as_str()
        .ok_or("policy document invalid: imap username must be a string")?;
    let secret_ref = imap["secret_ref"]
        .as_str()
        .ok_or("policy document invalid: imap secret_ref must be a string")?;
    let port = u16::try_from(imap["port"].as_u64().unwrap_or(993))
        .map_err(|_| "policy document invalid: imap port out of range".to_owned())?;

    // Absent means password: every account written before OAuth existed used
    // one, and an unknown mechanism is refused rather than guessed at — a
    // wrong guess would send the secret over the wire the wrong way.
    let auth = match imap["auth"].as_str().unwrap_or("password") {
        "password" => ImapAuth::Password,
        "xoauth2" => ImapAuth::XOAuth2,
        other => {
            return Err(format!(
                "policy document invalid: unknown imap auth {other:?}"
            ));
        }
    };

    Ok(ImapProviderConfig::new(
        AccountId::new(id),
        host,
        port,
        username,
        SecretRef::new(secret_ref),
    )
    .with_auth(auth))
}

/// What renewing an access token takes. Deliberately not part of
/// `ImapProviderConfig`: the IMAP session only cares that its secret is a
/// bearer token, never where a replacement comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OAuthFacts {
    pub(crate) token_endpoint: String,
    pub(crate) client_id: String,
}

fn parse_oauth_facts(imap: &Value) -> Result<Option<OAuthFacts>, String> {
    if imap["auth"].as_str() != Some("xoauth2") {
        return Ok(None);
    }
    let token_endpoint = imap["token_endpoint"]
        .as_str()
        .ok_or("policy document invalid: an xoauth2 account needs a token_endpoint")?;
    let client_id = imap["client_id"]
        .as_str()
        .ok_or("policy document invalid: an xoauth2 account needs a client_id")?;

    Ok(Some(OAuthFacts {
        token_endpoint: token_endpoint.to_owned(),
        client_id: client_id.to_owned(),
    }))
}

fn parse_account_policy(account: &Value) -> Result<Policy, String> {
    let id = account["id"]
        .as_str()
        .ok_or("policy document invalid: account id must be a string")?;

    // A missing read level means no read access — restrictive by default.
    let read = match account["read"].as_str().unwrap_or("none") {
        "none" => ReadAccess::None,
        "headers" => ReadAccess::Headers,
        "full_message" => ReadAccess::FullMessage,
        "with_attachments" => ReadAccess::WithAttachments,
        other => return Err(format!("policy document invalid: read level {other}")),
    };

    let write_value = &account["write"];
    let write = WriteAccess {
        drafts: write_value["drafts"].as_bool().unwrap_or(false),
        mark: write_value["mark"].as_bool().unwrap_or(false),
        move_messages: write_value["move"].as_bool().unwrap_or(false),
        trash: write_value["trash"].as_bool().unwrap_or(false),
        permanent_delete: write_value["permanent_delete"].as_bool().unwrap_or(false),
    };

    let mut folder_rules = BTreeMap::new();
    if let Some(rules) = account["folder_rules"].as_object() {
        for (mailbox, rule) in rules {
            folder_rules.insert(
                mailbox.clone(),
                FolderRule {
                    read: rule["read"].as_bool().unwrap_or(true),
                    write: rule["write"].as_bool().unwrap_or(true),
                },
            );
        }
    }

    let permissions = PermissionSet {
        read,
        write,
        send: account["send"].as_bool().unwrap_or(false),
        per_folder: account["per_folder"].as_bool().unwrap_or(false),
        folder_rules,
    };

    Ok(Policy::new(AccountId::new(id), permissions))
}
