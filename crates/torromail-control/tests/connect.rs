//! Pairing an assistant end to end, against a scratch home, a scratch data
//! directory and an in-memory secret store.

use std::path::{Path, PathBuf};

use serde_json::Value;
use torromail_control::clients::{self, Environment, Platform, SetupError};
use torromail_control::connect::Pairing;
use torromail_control::policy::ClientAccountAccess;
use torromail_control::secrets::{MemoryStore, SERVICE, SecretStore, client_key_account};
use torromail_control::{paths, save};

struct Scene {
    home: PathBuf,
    data: PathBuf,
    environment: Environment,
    secrets: MemoryStore,
    context: torromail_control::PolicyContext,
}

fn scene(name: &str) -> Scene {
    let root = std::env::temp_dir().join(format!("torromail-connect-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".cursor")).expect("cursor is installed");
    let data = root.join("data");
    std::fs::create_dir_all(&data).expect("a data directory");
    Scene {
        environment: Environment { platform: Platform::Linux, home: home.clone(), executable_directories: vec![] },
        home,
        data,
        secrets: MemoryStore::default(),
        context: save::default_context(),
    }
}

fn no_tool(_: &Path, _: &[String]) -> Result<(), SetupError> {
    panic!("no CLI-owned client in these tests")
}

impl Scene {
    fn pairing(&self) -> Pairing<'_> {
        Pairing {
            data_directory: &self.data,
            environment: &self.environment,
            secrets: &self.secrets,
            context: &self.context,
            run: &no_tool,
        }
    }

    fn policy(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.data.join(paths::POLICY_FILE)).expect("published")).expect("JSON")
    }

    fn cursor_config(&self) -> clients::ClientSetup {
        clients::installed_client(&self.environment, "cursor").expect("installed").setup
    }
}

#[test]
fn connecting_puts_one_key_in_three_places_that_agree() {
    let scene = scene("agree");
    scene.pairing().connect("cursor", "/usr/bin/torromail-mcp").expect("connects");

    let key = scene.secrets.get(SERVICE, &client_key_account("cursor")).expect("readable").expect("stored");
    assert!(key.starts_with("torro_cursor_"));
    assert!(clients::has_key(&scene.cursor_config(), &key), "the client's config carries the key");
    let policy = scene.policy();
    assert_eq!(policy["clients"][0]["id"], "cursor");
    assert_eq!(policy["clients"][0]["token_sha256"], clients::sha256_hex(&key), "the allowlist carries its hash");
    assert_eq!(policy["clients"][0]["account_access"]["mode"], "all");
    assert!(!policy.to_string().contains(&key), "and never the key");
}

#[test]
fn reconnecting_renews_the_key_and_keeps_the_grants() {
    let scene = scene("renew");
    let pairing = scene.pairing();
    pairing.connect("cursor", "/usr/bin/torromail-mcp").expect("connects");
    let first = pairing.token("cursor").expect("readable").expect("stored");
    pairing
        .set_account_access("cursor", ClientAccountAccess::Selected(["work".to_owned()].into()))
        .expect("grants");

    pairing.connect("cursor", "/usr/bin/torromail-mcp").expect("reconnects");
    let second = pairing.token("cursor").expect("readable").expect("stored");
    assert_ne!(first, second);
    assert!(!clients::has_key(&scene.cursor_config(), &first), "the old key is gone from the config");
    let policy = scene.policy();
    assert_eq!(policy["clients"].as_array().expect("clients").len(), 1, "one entry, not two");
    assert_eq!(policy["clients"][0]["token_sha256"], clients::sha256_hex(&second));
    assert_eq!(policy["clients"][0]["account_access"]["account_ids"], serde_json::json!(["work"]));
}

#[test]
fn a_config_that_cannot_be_written_leaves_everything_as_it_was() {
    let scene = scene("rollback");
    std::fs::write(scene.home.join(".cursor/mcp.json"), "{ not json").expect("writable");

    let error = scene.pairing().connect("cursor", "/usr/bin/torromail-mcp").expect_err("must fail");
    assert!(error.to_string().contains("could not be read"), "got: {error}");
    assert_eq!(scene.secrets.get(SERVICE, &client_key_account("cursor")), Ok(None), "no key nobody holds");
    assert!(!scene.data.join(paths::POLICY_FILE).exists(), "and nobody new on the allowlist");
    assert_eq!(std::fs::read_to_string(scene.home.join(".cursor/mcp.json")).expect("still there"), "{ not json");
}

#[test]
fn a_manual_client_gets_a_key_without_any_config_being_touched() {
    let scene = scene("manual");
    scene.pairing().connect("hermes", "/usr/bin/torromail-mcp").expect("connects");
    assert!(scene.pairing().token("hermes").expect("readable").is_some());
    assert_eq!(scene.policy()["clients"][0]["name"], "Hermes");
    assert!(!scene.home.join(".hermes").exists());
}

#[test]
fn an_assistant_that_is_not_here_cannot_be_connected() {
    let scene = scene("absent");
    let error = scene.pairing().connect("windsurf", "/usr/bin/torromail-mcp").expect_err("not installed");
    assert!(error.to_string().contains("not installed"));
    assert!(scene.pairing().connect("nonsense", "/x").is_err());
}

#[test]
fn disconnecting_revokes_first_and_cleans_up_after() {
    let scene = scene("disconnect");
    let pairing = scene.pairing();
    pairing.connect("cursor", "/usr/bin/torromail-mcp").expect("connects");
    pairing.disconnect("cursor").expect("disconnects");

    assert_eq!(scene.policy()["clients"], serde_json::json!([]));
    assert_eq!(pairing.token("cursor"), Ok(None));
    assert!(!clients::is_configured(&scene.cursor_config()));
    assert!(pairing.set_account_access("cursor", ClientAccountAccess::All).is_err(), "nothing to grant to");
}
