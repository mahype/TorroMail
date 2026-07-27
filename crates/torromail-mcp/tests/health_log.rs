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
