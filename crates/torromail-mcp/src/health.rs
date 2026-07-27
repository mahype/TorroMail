//! The account health log: `health.jsonl`, a sibling of `audit.jsonl` and
//! `connections.jsonl` in the app's Application Support folder.
//!
//! Both the server and the app append to it, and the app derives every
//! account's status dot from its tail. Writing is best-effort throughout — a
//! mail action must never fail because a health line could not be written,
//! and a missed sample costs at most one interval.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};
use torromail_core::CoreError;

/// What one login attempt proved. Deliberately three-valued: "we could not
/// get there" and "it said no" need opposite handling, and collapsing them is
/// what made every account look green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthOutcome {
    Ok,
    Rejected,
    Unreachable,
}

impl HealthOutcome {
    /// The word on the wire — in `health.jsonl` and on `--check-account`'s
    /// stderr.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Rejected => "rejected",
            Self::Unreachable => "unreachable",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "ok" => Some(Self::Ok),
            "rejected" => Some(Self::Rejected),
            "unreachable" => Some(Self::Unreachable),
            _ => None,
        }
    }

    /// What a failure says about the account's health — `None` when it says
    /// nothing. A policy denial or a missing message is not a health signal,
    /// and recording one would turn an ordinary refusal into an alarm.
    #[must_use]
    pub fn from_error(error: &CoreError) -> Option<Self> {
        match error {
            CoreError::CredentialRejected(_) => Some(Self::Rejected),
            CoreError::ProviderFailure(_) => Some(Self::Unreachable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthRecord {
    pub ts: u64,
    pub account: String,
    pub outcome: HealthOutcome,
    pub source: String,
    pub detail: String,
}

/// How many records are kept **per account**. A global cap would let one busy
/// account's records push another's off the end, and an account with no
/// records falls back to the stale setup-time flag this log exists to replace
/// — so a shared cap quietly restores the original bug under load. Twenty is
/// well clear of the deepest rule downstream: the app looks three back for its
/// three-strikes window, and further for the most recent non-`unreachable`
/// record, which can sit a good way back during an outage.
const RECORDS_PER_ACCOUNT: usize = 20;

/// How recently an account must have been checked for a starting server to
/// leave it alone. Five minutes: long enough that a burst of client launches
/// produces one login per account, short enough that the first check after a
/// quiet period is genuinely current.
pub const SERVER_START_WINDOW: u64 = 300;

/// How long a success silences further successes for the same account.
/// Health is a status, not a transcript — `audit.jsonl` already records
/// every call. Without this, one busy assistant session writes hundreds of
/// identical "still fine" lines and crowds every other account out of the
/// log.
pub const OK_THROTTLE: u64 = 60;

/// Append one record. Every error is swallowed: this is a sample, not a
/// transaction.
pub fn append(path: &Path, account: &str, outcome: HealthOutcome, source: &str, detail: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let line = json!({
        "ts": ts,
        "account": account,
        "outcome": outcome.as_str(),
        "source": source,
        "detail": detail,
    })
    .to_string();

    // Append mode is atomic per write for lines this short, so the app and
    // several server processes never interleave their records.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes());
    }
}

/// The log's recent history, oldest first and still in file order, trimmed to
/// the last [`RECORDS_PER_ACCOUNT`] records of each account. A missing file is
/// no records; a line that does not parse, or carries an outcome word we do
/// not know, is skipped rather than taken as fatal.
///
/// The whole file is read to get there. That is what the app's `AuditLog`
/// already does with a busier log, and it is the only way to bound retention
/// per account rather than per file.
#[must_use]
pub fn load(path: &Path) -> Vec<HealthRecord> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let parsed: Vec<HealthRecord> = text
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            Some(HealthRecord {
                ts: value["ts"].as_u64()?,
                account: value["account"].as_str()?.to_owned(),
                outcome: HealthOutcome::parse(value["outcome"].as_str()?)?,
                source: value["source"].as_str().unwrap_or_default().to_owned(),
                detail: value["detail"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .collect();

    // Walked newest-first so each account's own last few are the ones kept,
    // then flipped back: callers read file order, newest of an account last.
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut kept: Vec<HealthRecord> = parsed
        .iter()
        .rev()
        .filter(|record| {
            let count = seen.entry(record.account.as_str()).or_default();
            *count += 1;
            *count <= RECORDS_PER_ACCOUNT
        })
        .cloned()
        .collect();
    kept.reverse();
    kept
}

/// Whether this outcome is worth writing, given what is already logged.
/// Failures are never throttled: the first one is the entire point of the
/// feature, and it must reach the app on the call it happened.
#[must_use]
pub fn is_worth_recording(
    records: &[HealthRecord],
    account: &str,
    outcome: HealthOutcome,
    now: u64,
) -> bool {
    if outcome != HealthOutcome::Ok {
        return true;
    }
    // Only a success silences a success — throttling against a failure would
    // delay the recovery that turns the account's dot green again.
    !records
        .iter()
        .rev()
        .find(|record| record.account == account)
        .is_some_and(|latest| {
            latest.outcome == HealthOutcome::Ok && now.saturating_sub(latest.ts) < OK_THROTTLE
        })
}

/// Whether `account` is due for a check. Without this, every MCP client that
/// spawns its own server process would log in again at launch — five paired
/// clients means five logins per account, every time.
#[must_use]
pub fn needs_check(records: &[HealthRecord], account: &str, now: u64, window: u64) -> bool {
    !records
        .iter()
        .filter(|record| record.account == account)
        .any(|record| now.saturating_sub(record.ts) < window)
}
