//! Reading what the server wrote down: `audit.jsonl`, one line per finished
//! tool call, and `connections.jsonl`, one line per client handshake. Both are
//! read-only here and share a never-fail contract — a missing file is "nothing
//! happened yet", and a half-written trailing line is skipped, because a
//! broken log must not take a surface down.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

/// One finished tool call, in machine terms. Labels and account names are the
/// surface's business: it knows the language and what the user called things.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    /// Unix time, fractional as written.
    pub timestamp: f64,
    pub client: String,
    /// The account id, empty for calls that name none (`mail_list_accounts`).
    pub account: String,
    pub tool: String,
    pub detail: String,
    /// `ok` or `error`.
    pub result: String,
}

fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter(|line| !line.is_empty())
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The most recent `limit` entries, oldest first. A field an older server
/// build omitted defaults rather than dropping the whole line.
#[must_use]
pub fn load_audit(path: &Path, limit: usize) -> Vec<AuditEntry> {
    let text = |value: &Value, key: &str| value[key].as_str().unwrap_or_default().to_owned();
    let mut entries: Vec<AuditEntry> = lines(path)
        .iter()
        .filter_map(|value| {
            Some(AuditEntry {
                timestamp: value["ts"].as_f64()?,
                client: text(value, "client"),
                account: text(value, "account"),
                tool: text(value, "tool"),
                detail: text(value, "detail"),
                result: text(value, "result"),
            })
        })
        .collect();
    entries.drain(..entries.len().saturating_sub(limit));
    entries
}

/// When a client last completed a handshake, and what it said it was.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientConnection {
    pub client_id: String,
    pub last_connected: f64,
    pub reported_name: String,
    pub reported_version: String,
}

/// Each client's most recent handshake — the proof that it really connected,
/// which no reading of its config can give.
#[must_use]
pub fn latest_connections(path: &Path) -> HashMap<String, ClientConnection> {
    let mut latest: HashMap<String, ClientConnection> = HashMap::new();
    for value in lines(path) {
        let (Some(timestamp), Some(id)) = (value["ts"].as_f64(), value["client_id"].as_str()) else {
            continue;
        };
        if id.is_empty() || latest.get(id).is_some_and(|known| known.last_connected >= timestamp) {
            continue;
        }
        latest.insert(
            id.to_owned(),
            ClientConnection {
                client_id: id.to_owned(),
                last_connected: timestamp,
                reported_name: value["client_name"].as_str().unwrap_or_default().to_owned(),
                reported_version: value["client_version"].as_str().unwrap_or_default().to_owned(),
            },
        );
    }
    latest
}

/// A JSON array of strings, as the server prints a folder list.
#[must_use]
pub fn parse_string_array(text: &str) -> Option<Vec<String>> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// A flat JSON object as text-valued pairs — numbers and booleans in their
/// JSON spelling. What a progress line is, and all a reader of one needs.
#[must_use]
pub fn parse_flat_object(text: &str) -> Option<HashMap<String, String>> {
    Some(
        serde_json::from_str::<Value>(text)
            .ok()?
            .as_object()?
            .iter()
            .map(|(key, value)| (key.clone(), value.as_str().map_or_else(|| value.to_string(), str::to_owned)))
            .collect(),
    )
}
