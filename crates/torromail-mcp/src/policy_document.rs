//! The internal policy document the app publishes.
//!
//! The SwiftUI app writes this JSON whenever account permissions change;
//! every `torromail-mcp` instance — no matter who spawned it — reloads it
//! per tool call, so a switch flipped in the UI applies immediately — and a
//! client key revoked there locks that client out mid-session. The format
//! mirrors `PermissionSet` on both sides:
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
//!              "security": "tls",
//!              "username": "me@example.com",
//!              "secret_ref": "keychain://TorroMail/work",
//!              "auth": "xoauth2",
//!              "token_endpoint": "https://oauth2.googleapis.com/token",
//!              "client_id": "…apps.googleusercontent.com"}
//!   }],
//!   "clients": [{
//!     "id": "claude-desktop",
//!     "name": "Claude Desktop",
//!     "token_sha256": "9f86d081884c7d65…"
//!   }]
//! }
//! ```
//!
//! `clients` is the pairing allowlist: the app hands every connected
//! assistant an access key (`TORROMAIL_TOKEN` in its MCP config) and
//! publishes only the key's SHA-256 here — the document never holds a
//! usable secret. A document *without* the key predates pairing (or is
//! hand-managed) and enforces no client auth; a document *with* it, even an
//! empty list, admits only the clients it names.
//!
//! Anything unreadable fails closed: a broken document grants nothing.

use std::collections::BTreeMap;

use serde_json::Value;
use torromail_core::{
    AccountId, CacheLevel, ConnectionSecurity, FolderRule, ImapAuth, ImapProviderConfig,
    PermissionSet, Policy, ReadAccess, SecretRef, WriteAccess,
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
    /// How to submit mail for this account. Reuses the connection config type
    /// — host, port, username, secret, auth are the same shape as IMAP.
    pub(crate) smtp: Option<ImapProviderConfig>,
    /// Present only for `xoauth2` accounts: what it takes to renew the token
    /// when the stored one has gone stale. Shared by IMAP and SMTP.
    pub(crate) oauth: Option<OAuthFacts>,
}

/// What the app decided to keep on disk for this account: one level in the
/// language of the read permissions. The server acts on it — it is the
/// write-through ceiling — and reports it through `mail_get_cache_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheFacts {
    pub(crate) level: CacheLevel,
}

impl Default for CacheFacts {
    /// What an account looks like before the app has published cache facts:
    /// the product default — headers only.
    fn default() -> Self {
        Self {
            level: CacheLevel::Headers,
        }
    }
}

/// The whole document, parsed: the accounts it serves and — when the app
/// has published one — the client allowlist guarding them.
pub(crate) struct ParsedDocument {
    pub(crate) accounts: Vec<DocumentAccount>,
    pub(crate) clients: Option<Vec<DocumentClient>>,
}

/// One paired assistant: which one (labels for attribution) and the SHA-256
/// of the access key it must present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentClient {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) token_sha256: String,
}

pub(crate) fn parse_policy_document(text: &str) -> Result<ParsedDocument, String> {
    let document: Value =
        serde_json::from_str(text).map_err(|error| format!("policy document invalid: {error}"))?;
    let accounts = document["accounts"]
        .as_array()
        .ok_or("policy document invalid: accounts must be an array")?;

    Ok(ParsedDocument {
        accounts: accounts
            .iter()
            .map(parse_account)
            .collect::<Result<_, _>>()?,
        clients: parse_clients(&document)?,
    })
}

/// A missing key means "no pairing published" and is legal; a present but
/// malformed one is an error — guessing at an allowlist would either lock
/// every client out or let the wrong one in.
fn parse_clients(document: &Value) -> Result<Option<Vec<DocumentClient>>, String> {
    let clients = match document.get("clients") {
        None | Some(Value::Null) => return Ok(None),
        Some(clients) => clients
            .as_array()
            .ok_or("policy document invalid: clients must be an array")?,
    };

    clients
        .iter()
        .map(|client| {
            let id = client["id"]
                .as_str()
                .ok_or("policy document invalid: client id must be a string")?;
            let token_sha256 = client["token_sha256"]
                .as_str()
                .ok_or("policy document invalid: client token_sha256 must be a string")?;
            Ok(DocumentClient {
                id: id.to_owned(),
                // A label, not a right — an entry from a build that wrote
                // no name still guards its key.
                name: client["name"].as_str().unwrap_or(id).to_owned(),
                token_sha256: token_sha256.to_ascii_lowercase(),
            })
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

fn parse_account(account: &Value) -> Result<DocumentAccount, String> {
    let policy = parse_account_policy(account)?;
    let (imap, oauth) = match account.get("imap") {
        None | Some(Value::Null) => (None, None),
        Some(imap) => (
            Some(parse_connection_config(
                account,
                imap,
                993,
                ConnectionSecurity::implied_by_imap_port,
            )?),
            parse_oauth_facts(imap)?,
        ),
    };
    let smtp = match account.get("smtp") {
        None | Some(Value::Null) => None,
        // 587 matches what the app assumes for a record written before ports
        // travelled; 465 here would have the two disagree about the same
        // account.
        Some(smtp) => Some(parse_connection_config(
            account,
            smtp,
            587,
            ConnectionSecurity::implied_by_smtp_port,
        )?),
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
        smtp,
        oauth,
    })
}

/// The new shape (`{"level": "bodies"}`) first; a document written by an
/// older app maps its five switches through `CacheLevel::parse_legacy`, so
/// nothing has to be reconfigured. No cache block at all means the default.
fn parse_cache_facts(cache: &Value) -> CacheFacts {
    if !cache.is_object() {
        return CacheFacts::default();
    }

    if let Some(level) = cache["level"].as_str().and_then(CacheLevel::parse) {
        return CacheFacts { level };
    }

    let enabled = cache["local_cache_enabled"].as_bool().unwrap_or(true);
    let mode = cache["mode"].as_str().unwrap_or("metadata");
    CacheFacts {
        level: CacheLevel::parse_legacy(enabled, mode),
    }
}

/// A present-but-broken connection block is an error, never a silent
/// fixture fallback — that would misreport whose data is being served. Shared
/// by the IMAP and SMTP blocks, which have the same shape.
fn parse_connection_config(
    account: &Value,
    block: &Value,
    default_port: u16,
    implied_security: fn(u16) -> ConnectionSecurity,
) -> Result<ImapProviderConfig, String> {
    let id = account["id"]
        .as_str()
        .ok_or("policy document invalid: account id must be a string")?;
    let host = block["host"]
        .as_str()
        .ok_or("policy document invalid: connection host must be a string")?;
    let username = block["username"]
        .as_str()
        .ok_or("policy document invalid: connection username must be a string")?;
    let secret_ref = block["secret_ref"]
        .as_str()
        .ok_or("policy document invalid: connection secret_ref must be a string")?;
    let port = u16::try_from(block["port"].as_u64().unwrap_or(u64::from(default_port)))
        .map_err(|_| "policy document invalid: connection port out of range".to_owned())?;

    // Absent means password: every account written before OAuth existed used
    // one, and an unknown mechanism is refused rather than guessed at — a
    // wrong guess would send the secret over the wire the wrong way.
    let auth = match block["auth"].as_str().unwrap_or("password") {
        "password" => ImapAuth::Password,
        "xoauth2" => ImapAuth::XOAuth2,
        other => {
            return Err(format!(
                "policy document invalid: unknown connection auth {other:?}"
            ));
        }
    };

    // Absent means "whatever the port used to imply" — documents written
    // before the app could say it out loud keep connecting as they did. An
    // unknown value is refused rather than downgraded: guessing wrong here
    // would put the secret on the wire in the clear.
    let security = match block["security"].as_str() {
        None => implied_security(port),
        Some("tls") => ConnectionSecurity::Tls,
        Some("starttls") => ConnectionSecurity::StartTls,
        Some(other) => {
            return Err(format!(
                "policy document invalid: unknown connection security {other:?}"
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
    .with_auth(auth)
    .with_security(security))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn account_with(connection: &str) -> ParsedDocument {
        parse_policy_document(&format!(
            r#"{{"accounts":[{{"id":"a","read":"headers",{connection}}}]}}"#
        ))
        .expect("document parses")
    }

    #[test]
    fn stated_security_wins_over_the_port() {
        // The point of the field: implicit TLS on a port that is not 993, and
        // STARTTLS on one that is not 587. Inferring from the port made both
        // of these servers unreachable.
        let parsed = account_with(
            r#""imap":{"host":"h","port":1993,"security":"tls","username":"u","secret_ref":"keychain://s/a"},
               "smtp":{"host":"h","port":465,"security":"starttls","username":"u","secret_ref":"keychain://s/a"}"#,
        );
        let account = &parsed.accounts[0];
        assert_eq!(
            account.imap.as_ref().expect("the IMAP block parsed").security,
            ConnectionSecurity::Tls
        );
        assert_eq!(
            account.smtp.as_ref().expect("the SMTP block parsed").security,
            ConnectionSecurity::StartTls
        );
    }

    #[test]
    fn a_document_without_security_keeps_connecting_as_it_did() {
        // Written by an app build that could not state it. The ports are all
        // such a document ever had, so they still decide.
        let parsed = account_with(
            r#""imap":{"host":"h","port":993,"username":"u","secret_ref":"keychain://s/a"},
               "smtp":{"host":"h","port":587,"username":"u","secret_ref":"keychain://s/a"}"#,
        );
        let account = &parsed.accounts[0];
        assert_eq!(
            account.imap.as_ref().expect("the IMAP block parsed").security,
            ConnectionSecurity::Tls
        );
        assert_eq!(
            account.smtp.as_ref().expect("the SMTP block parsed").security,
            ConnectionSecurity::StartTls
        );
    }

    #[test]
    fn an_unknown_security_is_refused_rather_than_downgraded() {
        // Guessing here would be guessing whether the secret goes over the
        // wire in the clear.
        let outcome = parse_policy_document(
            r#"{"accounts":[{"id":"a","read":"headers","imap":{"host":"h","port":993,"security":"plain","username":"u","secret_ref":"keychain://s/a"}}]}"#,
        );
        let error = outcome.err().expect("an unknown security is an error");
        assert!(error.contains("unknown connection security"), "{error}");
    }
}
