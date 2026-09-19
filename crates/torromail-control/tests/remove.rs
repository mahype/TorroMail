use std::path::PathBuf;

use serde_json::{Value, json};
use torromail_control::enroll::{self, CheckOutcome};
use torromail_control::secrets::{MemoryStore, SERVICE, SecretStore};
use torromail_control::{JsonStateStore, StateStore, paths, providers, remove, save};

fn directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("torromail-remove-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

fn enrolled(directory: &std::path::Path, secrets: &MemoryStore, id: &str) {
    let config = providers::lookup("posteo.de").expect("in the table");
    let account = enroll::password_account(id.to_owned(), id, "x@posteo.de", &config);
    enroll::enroll(directory, secrets, &save::default_context(), account, "pw", &|_, _, _| CheckOutcome::Ok).expect("enrolls");
}

#[test]
fn everything_local_goes_and_the_other_account_is_untouched() {
    let directory = directory("all");
    let secrets = MemoryStore::default();
    enrolled(&directory, &secrets, "KEEP");
    enrolled(&directory, &secrets, "GONE");
    std::fs::create_dir_all(directory.join("cache")).expect("cache");
    for file in ["GONE.sqlite", "GONE.sqlite-wal", "KEEP.sqlite"] {
        std::fs::write(directory.join("cache").join(file), "x").expect("writable");
    }
    std::fs::create_dir_all(directory.join("attachments/GONE/m1")).expect("attachments");
    std::fs::write(directory.join("attachments/GONE/m1/a.pdf"), "x").expect("writable");
    // A client that was allowed to see exactly the two.
    let mut pairings = save::load_pairings(&directory).expect("readable");
    pairings.push(torromail_control::ClientPairing {
        id: "cursor".to_owned(),
        name: "Cursor".to_owned(),
        token_sha256: "aa".to_owned(),
        account_access: torromail_control::ClientAccountAccess::Selected(["KEEP".to_owned(), "GONE".to_owned()].into()),
    });
    save::publish(&directory, &pairings, &save::default_context()).expect("publishes");

    let leftovers = remove::remove_account(&directory, &secrets, &save::default_context(), "GONE").expect("removes");
    assert!(leftovers.is_empty(), "got: {leftovers:?}");

    let state = JsonStateStore::new(directory.join(paths::STATE_FILE)).load();
    assert_eq!(state.accounts.iter().map(|account| account.id.as_str()).collect::<Vec<_>>(), ["KEEP"]);
    let policy: Value = serde_json::from_str(&std::fs::read_to_string(directory.join(paths::POLICY_FILE)).expect("published")).expect("JSON");
    assert_eq!(policy["accounts"].as_array().expect("accounts").len(), 1);
    let cursor = policy["clients"].as_array().expect("clients").iter().find(|client| client["id"] == "cursor").expect("cursor");
    assert_eq!(cursor["account_access"]["account_ids"], json!(["KEEP"]), "a grant does not outlive its account");
    assert_eq!(secrets.get(SERVICE, "GONE"), Ok(None));
    assert_eq!(secrets.get(SERVICE, "KEEP"), Ok(Some("pw".to_owned())));
    assert!(!directory.join("cache/GONE.sqlite").exists() && !directory.join("cache/GONE.sqlite-wal").exists());
    assert!(directory.join("cache/KEEP.sqlite").exists());
    assert!(!directory.join("attachments/GONE").exists());
}

#[test]
fn an_account_that_is_not_there_is_an_error_and_an_odd_id_never_leaves_the_directory() {
    let directory = directory("odd");
    let secrets = MemoryStore::default();
    assert!(remove::remove_account(&directory, &secrets, &save::default_context(), "nobody").is_err());

    // Such an id cannot be enrolled here, but a state file is only a file.
    let config = providers::lookup("posteo.de").expect("in the table");
    let odd = enroll::password_account("../outside".to_owned(), "Odd", "x@posteo.de", &config);
    assert!(enroll::enroll(&directory, &secrets, &save::default_context(), odd.clone(), "pw", &|_, _, _| CheckOutcome::Ok).is_err());
    let state = torromail_control::AppState { accounts: vec![odd], ..Default::default() };
    JsonStateStore::new(directory.join(paths::STATE_FILE)).save(&state).expect("seeds");
    std::fs::write(directory.join("../outside.sqlite"), "precious").expect("writable");
    let leftovers = remove::remove_account(&directory, &secrets, &save::default_context(), "../outside").expect("removes");
    assert!(leftovers.iter().any(|note| note.contains("not a plain name")), "got: {leftovers:?}");
    assert!(directory.join("../outside.sqlite").exists(), "nothing outside the directory was touched");
    std::fs::remove_file(directory.join("../outside.sqlite")).ok();
}

#[test]
fn the_footprint_counts_the_cache_its_wal_files_and_the_kept_attachments() {
    use torromail_control::cache_files;
    let directory = directory("footprint");
    std::fs::create_dir_all(directory.join("cache")).expect("cache");
    std::fs::create_dir_all(directory.join("attachments/ACC/m1")).expect("attachments");
    std::fs::write(directory.join("cache/ACC.sqlite"), vec![0_u8; 1000]).expect("writable");
    std::fs::write(directory.join("cache/ACC.sqlite-wal"), vec![0_u8; 200]).expect("writable");
    std::fs::write(directory.join("cache/OTHER.sqlite"), vec![0_u8; 5000]).expect("writable");
    std::fs::write(directory.join("attachments/ACC/m1/a.pdf"), vec![0_u8; 34]).expect("writable");

    assert_eq!(cache_files::size_bytes(&directory, "ACC"), 1234);
    assert_eq!(cache_files::size_bytes(&directory, "NOBODY"), 0);
    assert_eq!(cache_files::size_bytes(&directory, "../cache"), 0, "an odd id measures nothing");
    assert_eq!(
        [0, 999, 1234, 3_400_000, 250_000_000, 7_300_000_000].map(cache_files::label),
        ["0 B", "999 B", "1.2 KB", "3.4 MB", "250 MB", "7.3 GB"]
    );
}
