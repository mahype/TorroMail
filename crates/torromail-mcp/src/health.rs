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

/// How long a repeat of the same outcome is silenced for one account. Health
/// is a status, not a transcript — `audit.jsonl` already holds every call, and
/// a second identical line thirty seconds later says nothing new. Only
/// *repeats* are throttled: a change of outcome is written the moment it
/// happens, so the first failure still reaches the app on the call it
/// happened, and so does the recovery.
///
/// Clock skew is tolerated rather than corrected. Elapsed time is a
/// `saturating_sub`, so a record dated in the future — a corrected clock, a
/// log copied between machines — reads as zero elapsed and suppresses further
/// records of that same outcome until real time catches up. Bounded by how far
/// ahead the stamp was, self-clearing, and it never suppresses a *change* of
/// outcome, which is the part that matters.
pub const REPEAT_THROTTLE: u64 = 60;

/// How much of `detail` is kept. The atomicity the append relies on holds for
/// short lines; error strings are short in practice, but truncating makes that
/// true by construction rather than by luck. Long enough for any provider
/// message worth reading.
const MAX_DETAIL_CHARS: usize = 300;

/// Append one record. Every error is swallowed: this is a sample, not a
/// transaction.
pub fn append(path: &Path, account: &str, outcome: HealthOutcome, source: &str, detail: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    // Taken by characters, not bytes, so a multi-byte message is never cut
    // mid-character into something that will not round-trip as JSON.
    let detail: String = detail.chars().take(MAX_DETAIL_CHARS).collect();
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

/// The whole throttle rule, over nothing but the last outcome recorded for the
/// account. Both entry points below funnel through here so the rule cannot
/// drift between the in-memory path a tool call takes and the on-disk one.
///
/// Note what "last" means: the single most recent record, not the most recent
/// record *of this outcome*. So a success arriving shortly after a failure is
/// written even though an earlier success is still inside the window — the
/// account is working again, and making the dot wait a minute to go green
/// would be a bug dressed as an optimisation.
fn worth_recording(last: Option<(u64, HealthOutcome)>, outcome: HealthOutcome, now: u64) -> bool {
    match last {
        None => true,
        Some((ts, previous)) => previous != outcome || now.saturating_sub(ts) >= REPEAT_THROTTLE,
    }
}

/// Whether this outcome is worth writing, judged against what is already in
/// the log. The cold-path form, for a caller that has a loaded log in hand;
/// anything on the path of a tool call wants [`HealthThrottle`] instead, which
/// answers the same question without reading the file.
#[must_use]
pub fn is_worth_recording(
    records: &[HealthRecord],
    account: &str,
    outcome: HealthOutcome,
    now: u64,
) -> bool {
    let last = records
        .iter()
        .rev()
        .find(|record| record.account == account)
        .map(|record| (record.ts, record.outcome));
    worth_recording(last, outcome, now)
}

/// The last outcome recorded for each account, so the throttle needs no file
/// read on the path a tool call takes. Empty in a fresh process, which costs
/// exactly one unthrottled record per account per process — cheaper than
/// parsing the log to learn something the process is about to know anyway.
///
/// This exists because the obvious alternative does not work. Consulting the
/// log meant reading all of it on every eligible call — measured at 65 ms
/// against a 100 000-line file, growing without bound, and quadratic in the
/// case that matters most: an account failing inside a retry loop writes a
/// line per call and re-reads everything each time. Reading only the file's
/// tail would fix the cost and reintroduce the crowding-out bug, since a busy
/// account floods any fixed window and a quiet account falls outside it.
#[derive(Debug, Default)]
pub struct HealthThrottle {
    last: HashMap<String, (u64, HealthOutcome)>,
}

impl HealthThrottle {
    /// Whether this outcome is worth writing. Records the decision when it
    /// answers `true`, so a caller cannot forget to and silently defeat the
    /// throttle.
    pub fn admit(&mut self, account: &str, outcome: HealthOutcome, now: u64) -> bool {
        if !worth_recording(self.last.get(account).copied(), outcome, now) {
            return false;
        }
        self.last.insert(account.to_owned(), (now, outcome));
        true
    }
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
