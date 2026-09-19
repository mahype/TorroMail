//! Saving a change: state and policy move together, the allowlist survives,
//! and a pairing file nobody can read stops the save rather than emptying it.

use std::path::PathBuf;

use serde_json::{Value, json};
use torromail_control::{AppState, JsonStateStore, MailAccount, StateStore, paths, save};

fn directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("torromail-save-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

fn stored_account() -> MailAccount {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/policy-document.json");
    let file: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("cases")).expect("JSON");
    MailAccount::from_json(&file["cases"][0]["accounts"][0]).expect("reads")
}

fn read(path: PathBuf) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("exists")).expect("JSON")
}

fn seed(directory: &std::path::Path, account: &MailAccount) {
    let state = AppState { accounts: vec![account.clone()], settings: json!({ "showDockIcon": false }), ..AppState::default() };
    JsonStateStore::new(directory.join(paths::STATE_FILE)).save(&state).expect("seeds");
}

#[test]
fn a_changed_right_reaches_the_state_and_the_policy_together() {
    let directory = directory("together");
    let mut account = stored_account();
    seed(&directory, &account);

    account.permissions.write.r#move = true;
    account.permissions.send = true;
    save::save_account(&directory, &account, &save::default_context()).expect("saves");

    let state = JsonStateStore::new(directory.join(paths::STATE_FILE)).load();
    assert!(state.accounts[0].permissions.send);
    assert_eq!(state.settings, json!({ "showDockIcon": false }), "another surface's settings survive");
    let policy = read(directory.join(paths::POLICY_FILE));
    assert_eq!(policy["accounts"][0]["send"], true);
    assert_eq!(policy["accounts"][0]["write"]["move"], true);
    assert_eq!(policy["accounts"][0]["imap"]["secret_ref"], "keychain://TorroMail/work");
}

#[test]
fn whoever_was_allowed_in_stays_allowed_in() {
    let directory = directory("allowlist");
    let account = stored_account();
    seed(&directory, &account);
    // A policy document from before this surface kept a pairing file.
    std::fs::write(
        directory.join(paths::POLICY_FILE),
        json!({ "version": 1, "accounts": [], "clients": [
            { "id": "claude-code", "name": "Claude Code", "token_sha256": "aa11", "account_access": { "mode": "selected", "account_ids": ["work"] } }
        ] })
        .to_string(),
    )
    .expect("writable");

    save::save_account(&directory, &account, &save::default_context()).expect("saves");

    let policy = read(directory.join(paths::POLICY_FILE));
    assert_eq!(policy["clients"][0]["token_sha256"], "aa11");
    assert_eq!(policy["clients"][0]["account_access"]["account_ids"], json!(["work"]));
    let pairings = read(directory.join(paths::PAIRINGS_FILE));
    assert_eq!(pairings["clients"][0]["id"], "claude-code", "and from now on they have a file of their own");
}

#[test]
fn an_unreadable_pairing_file_stops_the_save_before_anything_is_written() {
    let directory = directory("unreadable");
    let mut account = stored_account();
    seed(&directory, &account);
    std::fs::write(directory.join(paths::PAIRINGS_FILE), r#"{"clients":[{"id":"x","name":"X","token_sha256":"aa","account_access":{"mode":"everything"}}]}"#)
        .expect("writable");

    account.permissions.send = true;
    let error = save::save_account(&directory, &account, &save::default_context()).expect_err("must not save");
    assert!(error.to_string().contains("nothing was saved"), "got: {error}");
    assert!(!JsonStateStore::new(directory.join(paths::STATE_FILE)).load().accounts[0].permissions.send);
    assert!(!directory.join(paths::POLICY_FILE).exists());
}

#[test]
fn an_account_removed_meanwhile_is_not_resurrected() {
    let directory = directory("removed");
    JsonStateStore::new(directory.join(paths::STATE_FILE)).save(&AppState::default()).expect("seeds");
    let error = save::save_account(&directory, &stored_account(), &save::default_context()).expect_err("gone");
    assert!(error.to_string().contains("no longer exists"));
}
