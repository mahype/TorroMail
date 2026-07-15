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
//!     "read": "full_message",
//!     "write": {"drafts": true, "mark": false, "move": false,
//!               "trash": false, "permanent_delete": false},
//!     "send": false,
//!     "per_folder": true,
//!     "folder_rules": {"Private": {"read": false, "write": false}}
//!   }]
//! }
//! ```
//!
//! Anything unreadable fails closed: a broken document grants nothing.

use std::collections::BTreeMap;

use serde_json::Value;
use torromail_core::{
    AccountId, FolderRule, ImapProviderConfig, PermissionSet, Policy, ReadAccess, SecretRef,
    WriteAccess,
};

/// One account as the document describes it: its permissions, and — once
/// the app has connection facts — how to reach the real mailbox.
pub(crate) struct DocumentAccount {
    pub(crate) policy: Policy,
    pub(crate) imap: Option<ImapProviderConfig>,
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
    let imap = match account.get("imap") {
        None | Some(Value::Null) => None,
        Some(imap) => Some(parse_imap_config(account, imap)?),
    };
    Ok(DocumentAccount { policy, imap })
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

    Ok(ImapProviderConfig::new(
        AccountId::new(id),
        host,
        port,
        username,
        SecretRef::new(secret_ref),
    ))
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
