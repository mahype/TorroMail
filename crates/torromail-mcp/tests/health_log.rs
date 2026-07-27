//! The health log: what gets written, what gets read back, and when a fresh
//! record makes a check unnecessary.

use std::time::{SystemTime, UNIX_EPOCH};

use torromail_core::CoreError;
use torromail_mcp::health::{self, HealthOutcome, HealthThrottle};

/// Scoped by process id like the helpers in `tool_contract.rs`: two concurrent
/// test processes sharing one path would each see the other's records, and the
/// per-account count assertions would break for reasons having nothing to do
/// with the code under test.
fn temp_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "torromail-health-{}-{name}.jsonl",
        std::process::id()
    ));
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

/// A long `detail` is cut, so the short-line assumption the append's atomicity
/// rests on holds by construction. Multi-byte throughout, because cutting by
/// bytes would slice a character in half and the line would not parse back.
#[test]
fn an_overlong_detail_is_truncated_and_still_reads_back() {
    let path = temp_path("long-detail");
    let shouted = "ä".repeat(5_000);
    health::append(
        &path,
        "work",
        HealthOutcome::Rejected,
        "tool-call",
        &shouted,
    );

    let records = health::load(&path);
    assert_eq!(records.len(), 1, "the truncated line is still valid JSON");
    assert!(
        records[0].detail.chars().count() < shouted.chars().count(),
        "the detail was cut down"
    );
    assert!(
        shouted.starts_with(&records[0].detail),
        "what survives is the front of the message, not a mangled slice"
    );
}

// --- the throttle rule -------------------------------------------------

#[test]
fn a_repeat_of_the_same_outcome_is_throttled() {
    let mut throttle = HealthThrottle::default();

    assert!(
        throttle.admit("work", HealthOutcome::Ok, 1_000),
        "the first record for an account is always admitted"
    );
    assert!(
        !throttle.admit("work", HealthOutcome::Ok, 1_030),
        "a second 'still fine' inside the window says nothing new"
    );
    assert!(
        throttle.admit("other", HealthOutcome::Ok, 1_030),
        "the throttle is per account, not global"
    );
}

/// The failure case the reviewer found: before this rule, only successes were
/// capped, so an account failing inside a retry loop wrote a line per call.
#[test]
fn a_repeated_failure_is_throttled_too() {
    let mut throttle = HealthThrottle::default();

    assert!(throttle.admit("work", HealthOutcome::Unreachable, 1_000));
    assert!(
        !throttle.admit("work", HealthOutcome::Unreachable, 1_030),
        "a retry loop against a down server does not fill the log"
    );
}

#[test]
fn a_repeat_past_the_throttle_is_admitted_again() {
    let mut throttle = HealthThrottle::default();

    assert!(throttle.admit("work", HealthOutcome::Ok, 1_000));
    assert!(
        throttle.admit("work", HealthOutcome::Ok, 1_000 + health::REPEAT_THROTTLE),
        "past the window the same outcome is a fresh sample again"
    );
}

/// Every transition, in both directions and inside the throttle window. This
/// is the property that makes the throttle safe: it caps steady states without
/// ever delaying news.
#[test]
fn a_change_of_outcome_is_never_throttled() {
    let mut throttle = HealthThrottle::default();

    assert!(throttle.admit("work", HealthOutcome::Ok, 1_000));
    assert!(
        throttle.admit("work", HealthOutcome::Rejected, 1_001),
        "the first failure reaches the app on the call it happened"
    );
    assert!(
        throttle.admit("work", HealthOutcome::Unreachable, 1_002),
        "one failure kind changing to another is still news"
    );
    assert!(
        throttle.admit("work", HealthOutcome::Ok, 1_003),
        "and so is the recovery that turns the dot green again"
    );
}

/// `admit` records the decision itself, so a caller cannot admit twice and
/// silently defeat the throttle by forgetting to write down what it did.
#[test]
fn admitting_records_the_decision() {
    let mut throttle = HealthThrottle::default();

    assert!(throttle.admit("work", HealthOutcome::Ok, 1_000));
    assert!(!throttle.admit("work", HealthOutcome::Ok, 1_000));
}

/// The same rule read off a loaded log rather than off in-process state. Both
/// forms must agree, since a restarted process falls back to this one.
#[test]
fn the_log_form_of_the_rule_agrees_with_the_throttle() {
    let path = temp_path("throttle-log");
    health::append(&path, "work", HealthOutcome::Ok, "tool-call", "fine");
    let records = health::load(&path);

    assert!(
        !health::is_worth_recording(&records, "work", HealthOutcome::Ok, now()),
        "a repeat seconds later is throttled"
    );
    assert!(
        health::is_worth_recording(&records, "work", HealthOutcome::Rejected, now()),
        "a change is not"
    );
    assert!(
        health::is_worth_recording(&records, "other", HealthOutcome::Ok, now()),
        "an account with no record at all is always worth recording"
    );
}

/// Pins a deliberate choice: "last" means the most recent record, not the most
/// recent record *of this outcome*. With an `Ok` still inside the window but a
/// failure after it, the recovery is news and must be written — the variant
/// that searches back for the newest `Ok` would suppress it and leave the dot
/// red for up to a minute after the account came back.
#[test]
fn a_recovery_is_recorded_even_with_an_earlier_success_still_in_the_window() {
    let mut throttle = HealthThrottle::default();

    assert!(throttle.admit("work", HealthOutcome::Ok, 1_000));
    assert!(throttle.admit("work", HealthOutcome::Unreachable, 1_005));
    assert!(
        throttle.admit("work", HealthOutcome::Ok, 1_010),
        "the recovery is written though the earlier success is 10s old"
    );
}

/// The same deviation, against a log. It needs its own test: `HealthThrottle`
/// keeps only the single last outcome, so it *cannot* express the "newest
/// `Ok`" variant, whereas the log form has every record to hand and could.
#[test]
fn the_log_form_records_a_recovery_with_an_earlier_success_still_in_the_window() {
    let path = temp_path("recovery-log");
    health::append(&path, "work", HealthOutcome::Ok, "tool-call", "fine");
    health::append(&path, "work", HealthOutcome::Unreachable, "tool-call", "no");
    let records = health::load(&path);

    assert!(
        health::is_worth_recording(&records, "work", HealthOutcome::Ok, now()),
        "the last record is the outage, so the recovery is a change and is news"
    );
}

// --- what a failure says about an account ------------------------------

/// The single place deciding between "accuse the user's password immediately"
/// and "grant three strikes". Swapping these two arms is the kind of mistake
/// that produces a red dot for a train tunnel.
#[test]
fn an_error_maps_to_the_outcome_its_handling_requires() {
    assert_eq!(
        HealthOutcome::from_error(&CoreError::CredentialRejected("bad password".to_owned())),
        Some(HealthOutcome::Rejected),
        "a refused login is the account's own problem and will not fix itself"
    );
    assert_eq!(
        HealthOutcome::from_error(&CoreError::ProviderFailure("connection reset".to_owned())),
        Some(HealthOutcome::Unreachable),
        "a connection that failed on the way may well fix itself"
    );
}

/// A stored password the process cannot read is on the accusing side of that
/// line, and the OS error it came from must not be what the user reads first.
/// Pinned through the classifier rather than through a keychain, because a
/// keychain that fails on demand is not something a test can arrange — the
/// ruling is the part that can be wrong.
#[test]
fn a_secret_that_cannot_be_read_accuses_the_password_rather_than_the_server() {
    // The Security framework's own sentence for the real-world case, in the
    // language it was seen in.
    let error = torromail_mcp::keychain::unreadable_secret(
        "Der eingegebene Benutzername oder das Passwort ist ungültig.",
    );

    assert_eq!(
        HealthOutcome::from_error(&error),
        Some(HealthOutcome::Rejected),
        "an unreadable keychain item never heals on its own, so it must not wait out a grace period"
    );

    let detail = error.to_string();
    let ours = detail
        .find("enter this account's password again in TorroMail")
        .expect("the repair is in the message");
    let theirs = detail.find("Benutzername").expect("the cause is kept");
    assert!(
        ours < theirs,
        "the repair must come before the OS sentence it would otherwise be mistaken for: {detail}"
    );
}

/// The `None` arm carries as much weight as the other two: an ordinary refusal
/// is not a health signal, and recording one would turn a policy denial into
/// an alarm about the user's password.
#[test]
fn an_ordinary_refusal_says_nothing_about_health() {
    assert_eq!(
        HealthOutcome::from_error(&CoreError::MessageNotFound("42".to_owned())),
        None,
        "a missing message is not an account problem"
    );
    assert_eq!(
        HealthOutcome::from_error(&CoreError::GuiOnlyMutation),
        None,
        "a policy denial is not an account problem"
    );
}

// --- the server-start debounce -----------------------------------------

#[test]
fn a_fresh_record_makes_a_check_unnecessary() {
    let path = temp_path("debounce");
    health::append(&path, "work", HealthOutcome::Ok, "server-start", "fine");

    let records = health::load(&path);
    assert!(
        !health::needs_check(&records, "work", now(), health::SERVER_START_WINDOW),
        "a record written seconds ago is fresh enough"
    );
    assert!(
        health::needs_check(&records, "other", now(), health::SERVER_START_WINDOW),
        "an account with no record at all is always due"
    );
}

#[test]
fn a_stale_record_makes_a_check_due_again() {
    let path = temp_path("stale");
    health::append(&path, "work", HealthOutcome::Ok, "server-start", "fine");
    let records = health::load(&path);

    assert!(
        health::needs_check(
            &records,
            "work",
            now() + health::SERVER_START_WINDOW + 1,
            health::SERVER_START_WINDOW
        ),
        "past the window the account is due again"
    );
}
