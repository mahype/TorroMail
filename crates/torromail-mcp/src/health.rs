//! The account health log: `health.jsonl`, a sibling of `audit.jsonl` and
//! `connections.jsonl` in the app's Application Support folder.
//!
//! Both the server and the app append to it, and the app derives every
//! account's status dot from its tail. Writing is best-effort throughout — a
//! mail action must never fail because a health line could not be written,
//! and a missed sample costs at most one interval.

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

/// How many lines from the end are read. Far more than any rule needs — the
/// app's three-strikes window is three — but cheap, and it keeps a long file
/// from being parsed in full on every tick.
const TAIL_LINES: usize = 200;

/// How recently an account must have been checked for a starting server to
/// leave it alone. Five minutes: long enough that a burst of client launches
/// produces one login per account, short enough that the first check after a
/// quiet period is genuinely current.
pub const SERVER_START_WINDOW: u64 = 300;

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

/// The tail of the log, oldest first. A missing file is no records; a line
/// that does not parse, or carries an outcome word we do not know, is skipped
/// rather than taken as fatal.
#[must_use]
pub fn load(path: &Path) -> Vec<HealthRecord> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    lines[start..]
        .iter()
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
        .collect()
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
