//! The policy document: what the accounts and the paired clients add up to,
//! in the shape every `torromail-mcp` process reloads per tool call.
//!
//! Derived, never edited — the accounts are the truth and this is their
//! publication. Secrets do not travel: an account names where its secret
//! lives, and the server fetches it there.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::FormatError;
use crate::account::{LoginMethod, MailAccount, OAuthIssuer};

pub const POLICY_VERSION: u64 = 1;

/// Which accounts a paired client may see. Account permissions still apply on
/// top; this only narrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAccountAccess {
    /// Every account, including ones added later.
    All,
    Selected(BTreeSet<String>),
}

impl ClientAccountAccess {
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::All => json!({ "mode": "all" }),
            // A set, so the ids are already sorted and the document is stable.
            Self::Selected(ids) => json!({ "mode": "selected", "account_ids": ids }),
        }
    }
}

impl ClientAccountAccess {
    /// Strict on purpose: a grant that cannot be understood must fail the
    /// publication rather than quietly widen to "all".
    pub fn from_json(value: &Value) -> Result<Self, FormatError> {
        match value.get("mode").and_then(Value::as_str) {
            Some("all") if value.get("account_ids").is_none() => Ok(Self::All),
            Some("all") => Err(FormatError("account_access: all accounts cannot also select ids".to_owned())),
            Some("selected") => {
                let ids: BTreeSet<String> = value
                    .get("account_ids")
                    .and_then(Value::as_array)
                    .ok_or_else(|| FormatError("account_access: selected needs account_ids".to_owned()))?
                    .iter()
                    .map(|id| id.as_str().map(str::to_owned))
                    .collect::<Option<_>>()
                    .ok_or_else(|| FormatError("account_access: an account id is not text".to_owned()))?;
                if ids.contains("") {
                    return Err(FormatError("account_access: empty account id".to_owned()));
                }
                Ok(Self::Selected(ids))
            }
            _ => Err(FormatError("account_access: unknown mode".to_owned())),
        }
    }
}

/// One client allowed to spawn the server. Only the hash of its key is ever
/// written down; the key itself stays with the client and the keychain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPairing {
    pub id: String,
    pub name: String,
    pub token_sha256: String,
    pub account_access: ClientAccountAccess,
}

impl ClientPairing {
    pub fn from_json(value: &Value) -> Result<Self, FormatError> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| FormatError(format!("client: {key} is missing or not text")))
        };
        Ok(Self {
            id: text("id")?,
            name: text("name")?,
            token_sha256: text("token_sha256")?,
            account_access: ClientAccountAccess::from_json(
                value.get("account_access").ok_or_else(|| FormatError("client: account_access is missing".to_owned()))?,
            )?,
        })
    }
}

impl ClientPairing {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "token_sha256": self.token_sha256,
            "account_access": self.account_access.to_json(),
        })
    }
}

/// What the writer cannot know from the accounts alone, because it differs by
/// platform or by build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyContext {
    /// Prepended to the account id to name where its secret lives —
    /// `keychain://TorroMail/` on macOS.
    pub secret_ref_prefix: String,
    /// The OAuth client ids this build was registered with. They travel in
    /// the document because a client can spawn the server while no
    /// configuration surface is open to renew a token for it.
    pub google_client_id: String,
    pub microsoft_client_id: String,
}

impl PolicyContext {
    fn client_id(&self, issuer: OAuthIssuer) -> &str {
        match issuer {
            OAuthIssuer::Google => &self.google_client_id,
            OAuthIssuer::Microsoft => &self.microsoft_client_id,
        }
    }
}

/// `clients` is deliberately not optional: the allowlist is what stands
/// between the accounts and any process that spawns the server, so every
/// caller has to say who is allowed — an empty list means "nobody yet".
#[must_use]
pub fn document(accounts: &[MailAccount], clients: &[ClientPairing], context: &PolicyContext) -> Value {
    json!({
        "version": POLICY_VERSION,
        "accounts": accounts.iter().map(|account| account_object(account, context)).collect::<Vec<_>>(),
        "clients": clients
            .iter()
            .map(ClientPairing::to_json)
            .collect::<Vec<_>>(),
    })
}

/// Writes atomically and owner-only: a reloading server never sees a half
/// document, and connection facts plus the allowlist are nobody else's read.
pub fn publish(
    path: &Path,
    accounts: &[MailAccount],
    clients: &[ClientPairing],
    context: &PolicyContext,
) -> std::io::Result<()> {
    crate::state::write_atomically(path, document(accounts, clients, context).to_string().as_bytes())
}

fn account_object(account: &MailAccount, context: &PolicyContext) -> Value {
    let permissions = &account.permissions;
    let folder_rules: Map<String, Value> = permissions
        .folder_rules
        .iter()
        .map(|(mailbox, rule)| (mailbox.clone(), json!({ "read": rule.read, "write": rule.write })))
        .collect();

    let mut object = Map::new();
    object.insert("id".into(), json!(account.id));
    // Labels, not rights: `mail_list_accounts` has to name the account the way
    // the user does, or nobody can pick one.
    object.insert("name".into(), json!(account.name));
    object.insert("email".into(), json!(account.email));
    object.insert("read".into(), json!(permissions.read.policy_name()));
    object.insert(
        "write".into(),
        json!({
            "drafts": permissions.write.drafts,
            "mark": permissions.write.mark,
            "move": permissions.write.r#move,
            "trash": permissions.write.trash,
            "permanent_delete": permissions.write.permanent_delete,
        }),
    );
    object.insert("send".into(), json!(permissions.send));
    object.insert("per_folder".into(), json!(permissions.per_folder));
    object.insert("folder_rules".into(), Value::Object(folder_rules));
    // The server acts on this: the write-through ceiling for the account's
    // local cache, capped again by the read level.
    object.insert("cache".into(), json!({ "level": account.cache_level.as_str() }));

    let overrides = account.special_mailboxes.policy_overrides();
    if !overrides.is_empty() {
        object.insert("mailbox_overrides".into(), json!(overrides));
    }

    // Connection facts travel once the account has them, whatever the login
    // method. An account without this block has no mail to give: the server
    // refuses it rather than inventing any.
    if account.has_imap_connection() {
        object.insert(
            "imap".into(),
            connection_object(account, &account.imap_host, account.imap_port, account.imap_security.as_str(), context),
        );
    }
    // The encryption is stated rather than inferred from the port — the
    // server is what decides it.
    if account.has_smtp_connection() {
        object.insert(
            "smtp".into(),
            connection_object(account, &account.smtp_host, account.smtp_port, account.smtp_security.as_str(), context),
        );
    }
    Value::Object(object)
}

fn connection_object(
    account: &MailAccount,
    host: &str,
    port: u16,
    security: &str,
    context: &PolicyContext,
) -> Value {
    let mut object = Map::new();
    object.insert("host".into(), json!(host));
    object.insert("port".into(), json!(port));
    object.insert("security".into(), json!(security));
    object.insert("username".into(), json!(account.username));
    object.insert(
        "secret_ref".into(),
        json!(format!("{}{}", context.secret_ref_prefix, account.id)),
    );
    // OAuth accounts keep a token set behind that reference instead of a
    // password. The server renews it on its own, which is why the endpoint and
    // client id travel with the facts.
    if let (LoginMethod::OAuth, Some(issuer)) = (account.login_method, account.oauth_issuer) {
        object.insert("auth".into(), json!("xoauth2"));
        object.insert("token_endpoint".into(), json!(issuer.token_endpoint()));
        object.insert("client_id".into(), json!(context.client_id(issuer)));
    }
    Value::Object(object)
}

/// The document for a request as a configuration surface in another language
/// sends it: `{"accounts": [...], "clients": [...], "context": {...}}`, with
/// accounts in the shape `state.json` stores them. This is what
/// `torromail-mcp --policy-document` answers, so that surface needs no writer
/// of its own.
pub fn document_for_request(request: &Value) -> Result<Value, FormatError> {
    let list = |key: &str| {
        request
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| FormatError(format!("request: {key} is missing or not a list")))
    };
    let accounts = list("accounts")?
        .iter()
        .map(MailAccount::from_json)
        .collect::<Result<Vec<_>, _>>()?;
    let clients = list("clients")?
        .iter()
        .map(ClientPairing::from_json)
        .collect::<Result<Vec<_>, _>>()?;
    let context_text = |key: &str| {
        request
            .get("context")
            .and_then(|context| context.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| FormatError(format!("request: context.{key} is missing or not text")))
    };
    let context = PolicyContext {
        secret_ref_prefix: context_text("secret_ref_prefix")?,
        google_client_id: context_text("google_client_id")?,
        microsoft_client_id: context_text("microsoft_client_id")?,
    };
    Ok(document(&accounts, &clients, &context))
}
