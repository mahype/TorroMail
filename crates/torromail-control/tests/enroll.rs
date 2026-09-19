//! Adding an account: proven against a throwaway document first, stored only
//! when it passed, and leaving nothing behind when it did not.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use serde_json::Value;
use torromail_control::enroll::{self, APP_CLIENT_ID, CheckOutcome, EnrollError};
use torromail_control::secrets::{MemoryStore, SERVICE, SecretStore, client_key_account};
use torromail_control::{JsonStateStore, StateStore, clients, paths, providers, save};

fn directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("torromail-enroll-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

fn candidate() -> torromail_control::MailAccount {
    let config = providers::lookup("mailbox.org").expect("in the table");
    enroll::password_account("CANDIDATE-1".to_owned(), "Sven", "sven@mailbox.org", &config)
}

fn read(path: PathBuf) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("exists")).expect("JSON")
}

#[test]
fn the_candidate_is_checked_alone_with_the_surface_as_the_only_client() {
    let directory = directory("trial");
    let secrets = MemoryStore::default();
    let seen: RefCell<Option<(Value, String, String)>> = RefCell::new(None);
    let check = |policy: &Path, token: &str, account: &str| {
        *seen.borrow_mut() = Some((read(policy.to_path_buf()), token.to_owned(), account.to_owned()));
        // The password has to be retrievable while the check runs.
        assert_eq!(secrets.get(SERVICE, "CANDIDATE-1"), Ok(Some("hunter2".to_owned())));
        CheckOutcome::Ok
    };

    let account = enroll::enroll(&directory, &secrets, &save::default_context(), candidate(), "hunter2", &check)
        .expect("enrolls");

    let (trial, token, asked) = seen.into_inner().expect("the check ran");
    assert_eq!(asked, "CANDIDATE-1");
    assert_eq!(trial["accounts"].as_array().expect("accounts").len(), 1);
    assert_eq!(trial["accounts"][0]["imap"]["host"], "imap.mailbox.org");
    assert_eq!(trial["clients"].as_array().expect("clients").len(), 1);
    assert_eq!(trial["clients"][0]["token_sha256"], clients::sha256_hex(&token));
    assert_eq!(secrets.get(SERVICE, &client_key_account(APP_CLIENT_ID)), Ok(Some(token)), "the surface keeps its key");

    assert!(account.is_verified, "a proven account is remembered as proven");
    assert!(!directory.join("trial-CANDIDATE-1.json").exists(), "the throwaway document is gone");
    let state = JsonStateStore::new(directory.join(paths::STATE_FILE)).load();
    assert_eq!(state.accounts, vec![account]);
    let policy = read(directory.join(paths::POLICY_FILE));
    assert_eq!(policy["accounts"][0]["id"], "CANDIDATE-1");
    assert_eq!(policy["clients"][0]["id"], APP_CLIENT_ID, "so a later connection test still passes the gate");
}

#[test]
fn a_candidate_that_fails_leaves_nothing_behind() {
    let directory = directory("fails");
    let secrets = MemoryStore::default();
    let refuse = |_: &Path, _: &str, _: &str| CheckOutcome::Rejected("[AUTHENTICATIONFAILED] no".to_owned());

    let error = enroll::enroll(&directory, &secrets, &save::default_context(), candidate(), "wrong", &refuse)
        .expect_err("must not enroll");

    assert_eq!(error, EnrollError::Check(CheckOutcome::Rejected("[AUTHENTICATIONFAILED] no".to_owned())));
    assert_eq!(secrets.get(SERVICE, "CANDIDATE-1"), Ok(None), "the wrong password is not kept");
    assert!(JsonStateStore::new(directory.join(paths::STATE_FILE)).load().accounts.is_empty());
    assert!(!directory.join(paths::POLICY_FILE).exists(), "an unproven account never reaches assistants");
    assert!(!directory.join("trial-CANDIDATE-1.json").exists());
}

#[test]
fn a_second_account_joins_the_first_and_the_paired_assistants_stay() {
    let directory = directory("second");
    let secrets = MemoryStore::default();
    let ok = |_: &Path, _: &str, _: &str| CheckOutcome::Ok;
    enroll::enroll(&directory, &secrets, &save::default_context(), candidate(), "a", &ok).expect("first");

    let mut second = candidate();
    second.id = "CANDIDATE-2".to_owned();
    enroll::enroll(&directory, &secrets, &save::default_context(), second, "b", &ok).expect("second");
    let again = enroll::enroll(&directory, &secrets, &save::default_context(), candidate(), "a", &ok);
    assert!(matches!(again, Err(EnrollError::Other(reason)) if reason.contains("already exists")));

    let policy = read(directory.join(paths::POLICY_FILE));
    assert_eq!(policy["accounts"].as_array().expect("accounts").len(), 2);
    assert_eq!(policy["clients"].as_array().expect("clients").len(), 1, "the surface is listed once, not once per account");
}

#[test]
fn account_ids_look_like_the_ones_the_macos_app_makes() {
    let id = enroll::new_account_id().expect("randomness");
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.iter().map(|part| part.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
    assert!(id.chars().all(|character| character == '-' || character.is_ascii_hexdigit() && !character.is_ascii_lowercase()));
    assert!(parts[2].starts_with('4'), "version 4");
    assert_ne!(id, enroll::new_account_id().expect("randomness"));
}
