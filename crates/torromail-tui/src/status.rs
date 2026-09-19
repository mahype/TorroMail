//! `torromail status`: everything a glance needs, without a screen. The
//! desktop integration reads the JSON form; the plain form is for a shell.
//!
//! What assistants searched for is deliberately not in here. The log's detail
//! column carries search terms and recipients — fine inside the surface,
//! which someone opened on purpose, and out of place in a bar widget that
//! anyone walking past can read.

use serde_json::{Value, json};

use crate::data::{AccountHealth, Snapshot};

/// How many of the newest log entries travel.
const ACTIVITY: usize = 10;

#[must_use]
pub fn json(snapshot: &Snapshot, version: &str, language: crate::i18n::Lang) -> Value {
    let accounts: Vec<Value> = snapshot
        .accounts
        .iter()
        .map(|view| {
            let (state, reason) = match &view.health {
                AccountHealth::Connected => ("connected", None),
                AccountHealth::Failed(reason) => ("failed", Some(reason.as_str())),
                AccountHealth::NeedsTest => ("needs_test", None),
                AccountHealth::NotConfigured => ("not_configured", None),
            };
            json!({
                "id": view.account.id,
                "name": view.account.name,
                "email": view.account.email,
                "state": state,
                "reason": reason,
                "last_checked": view.last_checked,
            })
        })
        .collect();
    // Only what is, or could be, connected: a machine without Windsurf has
    // nothing to say about Windsurf.
    let clients: Vec<Value> = snapshot
        .clients
        .iter()
        .filter(|client| client.is_paired() || client.installed)
        .map(|client| {
            json!({
                "id": client.descriptor.id,
                "name": client.descriptor.display_name,
                "installed": client.installed,
                "paired": client.is_paired(),
                "has_current_key": client.has_current_key,
                "last_connected": client.connection.as_ref().map(|connection| connection.last_connected as u64),
                "version": client.connection.as_ref().map(|connection| connection.reported_version.clone()).filter(|version| !version.is_empty()),
            })
        })
        .collect();
    let activity: Vec<Value> = snapshot
        .audit
        .iter()
        .take(ACTIVITY)
        .map(|entry| {
            json!({
                "ts": entry.timestamp as u64,
                "client": entry.client,
                "account": if entry.account.is_empty() { Value::Null } else { json!(snapshot.account_name(&entry.account)) },
                // Several accounts often share a sender name; the address is
                // what tells them apart.
                "account_email": snapshot
                    .accounts
                    .iter()
                    .find(|view| view.account.id == entry.account)
                    .map(|view| view.account.email.clone()),
                "tool": entry.tool,
                "result": entry.result,
            })
        })
        .collect();
    let broken = snapshot.accounts.iter().filter(|view| view.health.is_broken()).count();
    json!({
        "schema": 1,
        "version": version,
        // The language chosen in TorroMail's settings, so a widget speaks the
        // same one rather than guessing from the desktop's locale.
        "language": match language {
            crate::i18n::Lang::De => "de",
            crate::i18n::Lang::En => "en",
        },
        "taken_at": snapshot.taken_at,
        "ready": snapshot.server_binary.is_some(),
        "broken_accounts": broken,
        "connected_clients": snapshot.connected_clients().len(),
        "autocheck": snapshot.autocheck,
        "accounts": accounts,
        "clients": clients,
        "activity": activity,
    })
}

/// One line per account and client, for a terminal.
#[must_use]
pub fn plain(snapshot: &Snapshot) -> String {
    let mut lines = Vec::new();
    for view in &snapshot.accounts {
        let state = match &view.health {
            AccountHealth::Connected => "ok".to_owned(),
            AccountHealth::Failed(reason) => format!("BROKEN  {reason}"),
            AccountHealth::NeedsTest => "not tested".to_owned(),
            AccountHealth::NotConfigured => "not configured".to_owned(),
        };
        lines.push(format!("account  {:<20} {:<32} {state}", view.account.name, view.account.email));
    }
    for client in snapshot.clients.iter().filter(|client| client.is_paired()) {
        let seen = if client.connection.is_some() { "connected" } else { "never connected" };
        lines.push(format!("client   {:<20} {seen}", client.descriptor.display_name));
    }
    if lines.is_empty() {
        lines.push("nothing set up yet — start torromail and press n".to_owned());
    }
    lines.join("\n")
}
