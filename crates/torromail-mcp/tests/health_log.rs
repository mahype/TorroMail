//! The health log: what gets written, what gets read back, and when a fresh
//! record makes a check unnecessary.

use std::time::{SystemTime, UNIX_EPOCH};

use torromail_mcp::health::{self, HealthOutcome};

fn temp_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("torromail-health-{name}.jsonl"));
    let _ = std::fs::remove_file(&path);
    path
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[test]
fn an_appended_record_reads_back_with_its_fields() {
    let path = temp_path("roundtrip");
    health::append(&path, "work", HealthOutcome::Rejected, "tool-call", "nope");

    let records = health::load(&path);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].account, "work");
    assert_eq!(records[0].outcome, HealthOutcome::Rejected);
    assert_eq!(records[0].source, "tool-call");
    assert_eq!(records[0].detail, "nope");
}

#[test]
fn a_missing_file_reads_as_no_records() {
    assert!(health::load(&temp_path("absent")).is_empty());
}

#[test]
fn a_broken_line_is_skipped_rather_than_taken_as_fatal() {
    let path = temp_path("broken");
    health::append(&path, "work", HealthOutcome::Ok, "periodic", "fine");
    std::fs::write(
        &path,
        format!(
            "{}{{ not json\n",
            std::fs::read_to_string(&path).expect("the appended line reads back")
        ),
    )
    .expect("the log is writable");

    let records = health::load(&path);
    assert_eq!(
        records.len(),
        1,
        "the good line survives its broken neighbour"
    );
}

/// A later build may learn an outcome word this one has never heard of. Losing
/// that one line is fine; refusing to read the log — and so taking every status
/// dot down — is not.
#[test]
fn an_unknown_outcome_word_is_skipped_rather_than_taken_as_fatal() {
    let path = temp_path("unknown-outcome");
    health::append(&path, "work", HealthOutcome::Ok, "periodic", "fine");
    // Written by hand: `append` can only produce words this build knows.
    std::fs::write(
        &path,
        format!(
            "{}{}\n",
            std::fs::read_to_string(&path).expect("the appended line reads back"),
            r#"{"ts":1,"account":"work","outcome":"quarantined","source":"periodic","detail":""}"#,
        ),
    )
    .expect("the log is writable");

    let records = health::load(&path);
    assert_eq!(records.len(), 1, "only the word we understand survives");
    assert_eq!(records[0].outcome, HealthOutcome::Ok);
}

/// The crowding-out bug. A busy session against one account must never push
/// another account's records out of the log — downstream, an account with no
/// records falls back to the stale `isVerified` bit this whole feature exists
/// to replace, so losing them silently restores the original bug under load.
#[test]
fn a_chatty_account_does_not_crowd_out_a_quiet_one() {
    let path = temp_path("crowding");
    health::append(&path, "quiet", HealthOutcome::Ok, "server-start", "fine");
    for _ in 0..250 {
        health::append(&path, "chatty", HealthOutcome::Ok, "tool-call", "fine");
    }

    let records = health::load(&path);
    assert!(
        records.iter().any(|record| record.account == "quiet"),
        "the quiet account survives 250 lines from a busy neighbour"
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.account == "chatty")
            .count(),
        20,
        "the chatty account is capped at its own retention, not the file's"
    );
}

/// Retention is per account, but the order callers see is still the file's:
/// the app reads the newest record for an account off the end.
#[test]
fn records_come_back_in_file_order() {
    let path = temp_path("order");
    health::append(&path, "work", HealthOutcome::Ok, "server-start", "fine");
    health::append(&path, "other", HealthOutcome::Unreachable, "periodic", "no");
    health::append(&path, "work", HealthOutcome::Rejected, "tool-call", "nope");

    let records = health::load(&path);
    let accounts: Vec<&str> = records
        .iter()
        .map(|record| record.account.as_str())
        .collect();
    assert_eq!(accounts, ["work", "other", "work"]);
    assert_eq!(
        records[2].outcome,
        HealthOutcome::Rejected,
        "the newest record for an account sits last, where the app looks"
    );
}

#[test]
fn a_success_soon_after_a_success_is_not_worth_recording() {
    let path = temp_path("throttle-ok");
    health::append(&path, "work", HealthOutcome::Ok, "tool-call", "fine");
    let records = health::load(&path);

    assert!(
        !health::is_worth_recording(&records, "work", HealthOutcome::Ok, now()),
        "a second 'still fine' seconds later says nothing new"
    );
    assert!(
        health::is_worth_recording(&records, "other", HealthOutcome::Ok, now()),
        "an account with no record at all is always worth recording"
    );
}

#[test]
fn a_success_past_the_throttle_is_worth_recording_again() {
    let path = temp_path("throttle-expired");
    health::append(&path, "work", HealthOutcome::Ok, "tool-call", "fine");
    let records = health::load(&path);

    assert!(
        health::is_worth_recording(
            &records,
            "work",
            HealthOutcome::Ok,
            now() + health::OK_THROTTLE + 1
        ),
        "past the throttle a success is a fresh sample again"
    );
}

/// The case where throttling would be a bug rather than an optimisation: the
/// first failure is the entire point of the feature and must reach the app on
/// the call it happened, however recently we said the account was fine.
#[test]
fn a_failure_is_never_throttled_by_a_recent_success() {
    let path = temp_path("throttle-failure");
    health::append(&path, "work", HealthOutcome::Ok, "tool-call", "fine");
    let records = health::load(&path);

    assert!(
        health::is_worth_recording(&records, "work", HealthOutcome::Rejected, now()),
        "a rejection seconds after a success still gets written"
    );
    assert!(
        health::is_worth_recording(&records, "work", HealthOutcome::Unreachable, now()),
        "an outage seconds after a success still gets written"
    );
}

/// Only a *success* silences a success. Throttling against a failure would
/// delay the recovery that turns the dot green again.
#[test]
fn a_success_soon_after_a_failure_is_worth_recording() {
    let path = temp_path("throttle-after-failure");
    health::append(&path, "work", HealthOutcome::Unreachable, "tool-call", "no");
    let records = health::load(&path);

    assert!(
        health::is_worth_recording(&records, "work", HealthOutcome::Ok, now()),
        "the recovery is news even though it arrives seconds later"
    );
}

#[test]
fn a_fresh_record_makes_a_check_unnecessary() {
    let path = temp_path("debounce");
    health::append(&path, "work", HealthOutcome::Ok, "server-start", "fine");

    let records = health::load(&path);
    assert!(
        !health::needs_check(&records, "work", now(), 300),
        "a record written seconds ago is fresh enough"
    );
    assert!(
        health::needs_check(&records, "other", now(), 300),
        "an account with no record at all is always due"
    );
}

#[test]
fn a_stale_record_makes_a_check_due_again() {
    let path = temp_path("stale");
    health::append(&path, "work", HealthOutcome::Ok, "server-start", "fine");
    let records = health::load(&path);

    assert!(
        health::needs_check(&records, "work", now() + 400, 300),
        "past the window the account is due again"
    );
}
