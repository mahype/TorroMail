//! The Secret Service behind `secret-tool`. The rules are exercised against a
//! stand-in program that behaves as the real one was observed to; the real
//! one is met in an ignored test, because it writes to the keyring of whoever
//! runs it.

#![cfg(unix)]

use std::path::PathBuf;

use torromail_control::secrets::{MemoryStore, SecretError, SecretStore, SecretToolStore};

/// The stand-ins are scripts written moments before they run. A fork in
/// another test thread while one is still open for writing makes the kernel
/// refuse to execute it ("text file busy"), so the tests that write scripts
/// take turns.
static SCRIPTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    SCRIPTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A `secret-tool` that keeps its items as files: silent exit 1 when nothing
/// is found, exactly the bytes it was given when something is.
fn stand_in(name: &str, body: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let directory = std::env::temp_dir().join(format!("torromail-secrets-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    let program = directory.join("secret-tool");
    std::fs::write(&program, format!("#!/bin/sh\nSTORE='{}'\n{body}\n", directory.display())).expect("writable");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    (program, directory)
}

const FILE_BACKED: &str = r#"
command="$1"; shift
[ "$command" = store ] && shift 2
item="$STORE/$2--$4"
case "$command" in
  store) cat > "$item" ;;
  lookup) [ -f "$item" ] && cat "$item" || exit 1 ;;
  clear) rm -f "$item" ;;
esac
"#;

#[test]
fn a_secret_comes_back_byte_for_byte_and_is_gone_after_delete() {
    let _turn = one_at_a_time();
    let (program, _) = stand_in("roundtrip", FILE_BACKED);
    let store = SecretToolStore::with_program(program);
    let secret = "pass word\nwith a second line and no trailing newline";

    assert_eq!(store.get("TorroMail", "work"), Ok(None), "nothing stored is an answer, not an error");
    store.set("TorroMail", "work", secret).expect("stores");
    assert_eq!(store.get("TorroMail", "work"), Ok(Some(secret.to_owned())));
    assert_eq!(store.get("TorroMail", "other"), Ok(None));
    store.delete("TorroMail", "work").expect("deletes");
    store.delete("TorroMail", "work").expect("deleting what is gone succeeds");
    assert_eq!(store.get("TorroMail", "work"), Ok(None));
}

#[test]
fn the_secret_never_appears_on_a_command_line() {
    let _turn = one_at_a_time();
    let (program, directory) = stand_in(
        "argv",
        r#"echo "$@" >> "$STORE/argv.log"; [ "$1" = store ] && cat > /dev/null; exit 0"#,
    );
    SecretToolStore::with_program(program).set("TorroMail", "work", "hunter2-very-secret").expect("stores");
    let arguments = std::fs::read_to_string(directory.join("argv.log")).expect("the stand-in logged its arguments");
    assert!(arguments.contains("service TorroMail account work"), "got: {arguments}");
    assert!(!arguments.contains("hunter2"), "got: {arguments}");
}

#[test]
fn an_unreachable_service_is_a_state_of_the_machine_not_a_missing_secret() {
    let _turn = one_at_a_time();
    let (program, _) = stand_in("no-bus", r#"echo "secret-tool: Could not connect: No such file or directory" >&2; exit 1"#);
    let store = SecretToolStore::with_program(program);
    assert!(matches!(store.get("TorroMail", "work"), Err(SecretError::Unavailable(detail)) if detail.contains("Could not connect")));
    assert!(matches!(store.set("TorroMail", "work", "x"), Err(SecretError::Unavailable(_))));
    assert!(matches!(store.delete("TorroMail", "work"), Err(SecretError::Unavailable(_))));

    let missing = SecretToolStore::with_program("/nonexistent/secret-tool");
    assert!(matches!(missing.get("TorroMail", "work"), Err(SecretError::Unavailable(_))));
}

#[test]
fn the_memory_store_keeps_services_and_accounts_apart() {
    let store = MemoryStore::default();
    store.set("TorroMail", "work", "a").expect("stores");
    store.set("Other", "work", "b").expect("stores");
    assert_eq!(store.get("TorroMail", "work"), Ok(Some("a".to_owned())));
    assert_eq!(store.get("Other", "work"), Ok(Some("b".to_owned())));
    store.delete("TorroMail", "work").expect("deletes");
    assert_eq!(store.get("TorroMail", "work"), Ok(None));
}

/// `cargo test -p torromail-control --test secrets -- --ignored`
#[test]
#[ignore = "writes to (and removes from) the real keyring of whoever runs it"]
fn the_real_secret_service_round_trips() {
    let store = SecretToolStore::default();
    let service = format!("torromail-selftest-{}", std::process::id());
    store.set(&service, "probe", "zwei\nzeilen").expect("the keyring accepts an item");
    assert_eq!(store.get(&service, "probe"), Ok(Some("zwei\nzeilen".to_owned())));
    store.delete(&service, "probe").expect("and gives it up again");
    assert_eq!(store.get(&service, "probe"), Ok(None));
}
