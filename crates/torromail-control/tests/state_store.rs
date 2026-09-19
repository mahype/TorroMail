//! `state.json`: what is kept, what an older record falls back to, and what
//! happens to a file that cannot be read.

use serde_json::json;
use torromail_control::{AppState, CacheLevel, ConnectionSecurity, JsonStateStore, MailAccount, StateStore};

fn temp_path(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("torromail-control-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory.join("state.json")
}

fn contract_accounts() -> Vec<serde_json::Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/policy-document.json");
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("the shared cases exist")).expect("JSON");
    file["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .flat_map(|case| case["accounts"].as_array().expect("accounts").clone())
        .collect()
}

#[test]
fn every_account_survives_a_round_trip_unchanged() {
    for stored in contract_accounts() {
        let account = MailAccount::from_json(&stored).expect("a contract account reads");
        let again = MailAccount::from_json(&account.to_json()).expect("what was written reads back");
        assert_eq!(account, again, "{}", account.id);
    }
}

#[test]
fn a_current_record_is_written_back_exactly_as_it_was_read() {
    // The macOS app reads this file too: a key renamed or dropped here would
    // cost it an account on its next launch.
    let stored = contract_accounts().into_iter().next().expect("a first account");
    let account = MailAccount::from_json(&stored).expect("it reads");
    assert_eq!(account.to_json(), stored);
}

#[test]
fn a_record_without_ports_or_encryption_gets_what_its_era_implied() {
    let account = MailAccount::from_json(&json!({
        "id": "old", "name": "Alt", "email": "alt@example.com", "provider": "IMAP/SMTP",
        "loginMethod": "password", "imapHost": "mail.example.com", "smtpHost": "mail.example.com",
        "smtpPort": 465, "username": "alt", "knownMailboxes": ["INBOX"],
        "permissions": { "read": 2, "write": { "drafts": true }, "send": false, "perFolder": false, "folderRules": {} },
        "searchCache": { "localCacheEnabled": true, "cacheMode": "metadata" }
    }))
    .expect("a legacy record reads");

    assert_eq!((account.imap_port, account.imap_security), (993, ConnectionSecurity::Tls));
    assert_eq!((account.smtp_port, account.smtp_security), (465, ConnectionSecurity::Tls));
    assert_eq!(account.cache_level, CacheLevel::Headers);
    assert!(!account.is_verified, "never verified unless the record says so");
    assert!(!account.special_mailboxes.manual);
}

#[test]
fn a_record_that_cannot_be_understood_says_where() {
    let error = MailAccount::from_json(&json!({ "id": "work", "name": "Torro" })).expect_err("incomplete");
    assert!(error.to_string().contains("account `work`"), "got: {error}");
}

#[test]
fn a_missing_file_is_an_empty_state() {
    assert_eq!(JsonStateStore::new(temp_path("absent")).load(), AppState::default());
}

#[test]
fn settings_another_surface_wrote_survive_a_save() {
    let path = temp_path("settings");
    let store = JsonStateStore::new(&path);
    let state = AppState {
        settings: json!({ "launchAtLogin": true, "showDockIcon": false, "somethingNewer": [1, 2] }),
        accounts: contract_accounts()
            .iter()
            .map(|stored| MailAccount::from_json(stored).expect("reads"))
            .collect(),
        ..AppState::default()
    };
    store.save(&state).expect("the state saves");

    assert_eq!(store.load(), state);
}

#[test]
fn an_unreadable_file_is_moved_aside_not_overwritten() {
    let path = temp_path("broken");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory exists");
    std::fs::write(&path, "{ not json").expect("writable");

    let store = JsonStateStore::new(&path);
    assert_eq!(store.load(), AppState::default());
    assert!(!path.exists(), "the broken file no longer sits where a save would land");
    let mut broken = path.clone().into_os_string();
    broken.push(".broken");
    assert_eq!(std::fs::read_to_string(broken).expect("kept for recovery"), "{ not json");
}

#[cfg(unix)]
#[test]
fn the_saved_file_is_owner_only_and_leaves_no_temporary_behind() {
    use std::os::unix::fs::PermissionsExt;
    let path = temp_path("mode");
    JsonStateStore::new(&path).save(&AppState::default()).expect("saves");

    let mode = std::fs::metadata(&path).expect("exists").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let siblings = std::fs::read_dir(path.parent().expect("a parent")).expect("listable").count();
    assert_eq!(siblings, 1, "only state.json remains");
}

#[test]
fn a_preset_is_recognised_from_the_values_and_lost_when_one_switch_moves() {
    use torromail_control::{PermissionPreset, PermissionSet};
    let mut permissions = PermissionSet::default();
    assert_eq!(permissions.matching_preset(), Some(PermissionPreset::ReadAndDrafts));
    for preset in PermissionPreset::ALL {
        permissions.read = preset.read();
        permissions.write = preset.write();
        permissions.send = preset.send();
        assert_eq!(permissions.matching_preset(), Some(preset));
    }
    permissions.write.permanent_delete = true;
    assert_eq!(permissions.matching_preset(), None, "permanent delete is never part of a preset");
}
