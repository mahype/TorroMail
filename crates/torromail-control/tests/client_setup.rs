//! Wiring TorroMail into MCP clients. The merge rules and the snippets are
//! shared cases (`contracts/client-config.json`) that the Swift contract suite
//! runs too; detection and the CLI-owned clients are exercised here against a
//! scratch home directory.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use serde_json::Value;
use torromail_control::clients::{
    self, ClientSetup, Environment, Platform, SetupError, SnippetFormat,
};

fn contract_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/client-config.json")
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("torromail-clients-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

fn no_tool(_: &Path, _: &[String]) -> Result<(), SetupError> {
    panic!("a JSON client never launches a tool")
}

#[test]
fn the_shared_config_cases_merge_as_written() {
    let mut file: Value =
        serde_json::from_str(&std::fs::read_to_string(contract_path()).expect("the shared cases exist")).expect("JSON");
    let command_path = file["command_path"].as_str().expect("command path").to_owned();
    let token = file["token"].as_str().expect("token").to_owned();
    let bless = std::env::var_os("TORROMAIL_BLESS").is_some();

    for (index, case) in file["merges"].as_array().expect("merges").iter().enumerate() {
        let name = case["name"].as_str().expect("named");
        let config = scratch(&format!("merge-{index}")).join("nested/config.json");
        if let Some(existing) = case["existing"].as_str() {
            std::fs::create_dir_all(config.parent().expect("a parent")).expect("directory");
            std::fs::write(&config, existing).expect("the existing config is writable");
        }
        let setup = match case["root_key"].as_str() {
            Some("servers") => ClientSetup::ServersJson { config: config.clone() },
            Some("mcp") => ClientSetup::OpenCodeJson { config: config.clone() },
            _ => ClientSetup::McpServersJson { config: config.clone() },
        };
        let outcome = match case["action"].as_str() {
            Some("remove") => clients::remove(&setup, &no_tool),
            _ => clients::add(&setup, &command_path, &token, &no_tool),
        };

        if case["error"].as_bool() == Some(true) {
            assert_eq!(outcome, Err(SetupError::UnreadableConfig), "{name}");
            assert_eq!(
                std::fs::read_to_string(&config).expect("still there"),
                case["existing"].as_str().expect("an error case has a file"),
                "{name}: the file is untouched"
            );
            continue;
        }
        assert_eq!(outcome, Ok(()), "{name}");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&config).expect("the config exists")).expect("JSON");
        assert_eq!(written, case["expected"], "{name}");
        if case["action"].as_str() != Some("remove") {
            assert!(clients::is_configured(&setup), "{name}");
            assert!(clients::has_key(&setup, &token), "{name}");
            assert!(!clients::has_key(&setup, "torro_cursor_other"), "{name}");
        } else {
            assert!(!clients::is_configured(&setup), "{name}");
        }
    }

    // Snippets are text the user pastes, so they are pinned as text.
    let formats = [
        SnippetFormat::McpServersJson,
        SnippetFormat::ServersJson,
        SnippetFormat::HermesYaml,
        SnippetFormat::OpenClawJson,
        SnippetFormat::OpenCodeJson,
    ];
    for format in formats {
        let snippet = clients::config_snippet(&command_path, format, &token);
        if bless {
            file["snippets"][format.as_str()] = Value::String(snippet);
        } else {
            assert_eq!(file["snippets"][format.as_str()], snippet, "{}", format.as_str());
        }
    }
    if bless {
        let text = serde_json::to_string_pretty(&file).expect("serializes");
        std::fs::write(contract_path(), text + "\n").expect("writable");
    }
}

#[test]
fn the_json_snippets_are_themselves_valid_configs() {
    for format in [SnippetFormat::McpServersJson, SnippetFormat::ServersJson, SnippetFormat::OpenClawJson, SnippetFormat::OpenCodeJson] {
        let snippet = clients::config_snippet("/opt/torromail-mcp", format, "torro_x_1");
        let parsed: Value = serde_json::from_str(&snippet).expect("a JSON snippet parses");
        assert!(parsed.to_string().contains("torro_x_1"), "{}", format.as_str());
    }
}

#[cfg(unix)]
fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("directory");
    std::fs::write(path, "#!/bin/sh\n").expect("writable");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[cfg(unix)]
#[test]
fn only_what_is_installed_is_listed_in_catalog_order() {
    let home = scratch("detect-linux");
    std::fs::create_dir_all(home.join(".cursor")).expect("cursor");
    std::fs::create_dir_all(home.join(".config/Code/User")).expect("vscode");
    executable(&home.join("bin/claude"));
    // Present but not executable: not a client.
    std::fs::write(home.join("bin/codex"), "").expect("writable");

    let environment = Environment {
        platform: Platform::Linux,
        home: home.clone(),
        executable_directories: vec![home.join("bin")],
    };
    let found = clients::installed(&environment);
    assert_eq!(found.iter().map(|client| client.id).collect::<Vec<_>>(), ["claude-code", "cursor", "vscode"]);
    assert_eq!(
        found[2].setup,
        ClientSetup::ServersJson { config: home.join(".config/Code/User/mcp.json") }
    );
    assert_eq!(
        found[0].setup,
        ClientSetup::ClaudeCodeCli { executable: home.join("bin/claude"), config: home.join(".claude.json") }
    );
}

#[test]
fn macos_looks_where_macos_apps_keep_their_config() {
    let home = scratch("detect-macos");
    std::fs::create_dir_all(home.join("Library/Application Support/Claude")).expect("claude");
    std::fs::create_dir_all(home.join(".config/Code/User")).expect("a linux-style vscode dir");

    let environment = Environment { platform: Platform::MacOs, home: home.clone(), executable_directories: vec![] };
    let found = clients::installed(&environment);
    assert_eq!(found.iter().map(|client| client.id).collect::<Vec<_>>(), ["claude-desktop"]);
    assert_eq!(
        found[0].setup.config(),
        home.join("Library/Application Support/Claude/claude_desktop_config.json")
    );
}

#[test]
fn a_cli_owned_client_is_replaced_through_its_own_tool() {
    let calls: RefCell<Vec<Vec<String>>> = RefCell::new(Vec::new());
    let record = |_: &Path, arguments: &[String]| {
        calls.borrow_mut().push(arguments.to_vec());
        // The remove of a missing entry fails, and must not stop the add.
        if arguments[1] == "remove" { Err(SetupError::ToolFailed) } else { Ok(()) }
    };
    let claude = ClientSetup::ClaudeCodeCli { executable: "/x/claude".into(), config: "/x/.claude.json".into() };
    assert_eq!(clients::add(&claude, "/opt/torromail-mcp", "torro_claude-code_1", &record), Ok(()));
    let codex = ClientSetup::CodexCli { executable: "/x/codex".into(), config: "/x/config.toml".into() };
    assert_eq!(clients::add(&codex, "/opt/torromail-mcp", "torro_chatgpt_1", &record), Ok(()));

    let calls = calls.into_inner();
    assert_eq!(calls[0], ["mcp", "remove", "torromail", "-s", "user"]);
    assert_eq!(
        calls[1],
        ["mcp", "add", "torromail", "-s", "user", "-e", "TORROMAIL_TOKEN=torro_claude-code_1", "--", "/opt/torromail-mcp"]
    );
    assert_eq!(calls[2], ["mcp", "remove", "torromail"]);
    assert_eq!(
        calls[3],
        ["mcp", "add", "torromail", "--env", "TORROMAIL_TOKEN=torro_chatgpt_1", "--", "/opt/torromail-mcp"]
    );
}

#[test]
fn a_failing_tool_is_reported_not_swallowed() {
    let failing = |_: &Path, _: &[String]| Err(SetupError::ToolFailed);
    let claude = ClientSetup::ClaudeCodeCli { executable: "/x/claude".into(), config: "/x/.claude.json".into() };
    assert_eq!(clients::add(&claude, "/opt/t", "torro_c_1", &failing), Err(SetupError::ToolFailed));
    assert_eq!(clients::remove(&claude, &failing), Err(SetupError::ToolFailed));
}

#[test]
fn the_key_codex_holds_is_read_from_either_toml_shape() {
    let directory = scratch("codex");
    for (name, toml) in [
        ("table", "[mcp_servers.other]\ncommand = \"x\"\n\n[mcp_servers.torromail]\ncommand = \"/opt/t\"\n\n[mcp_servers.torromail.env]\nTORROMAIL_TOKEN = \"torro_chatgpt_abc\"\n"),
        ("inline", "[mcp_servers.torromail]\ncommand = \"/opt/t\"\nenv = { \"TORROMAIL_TOKEN\" = \"torro_chatgpt_abc\" }\n"),
    ] {
        let config = directory.join(format!("{name}.toml"));
        std::fs::write(&config, toml).expect("writable");
        let setup = ClientSetup::CodexCli { executable: "/x/codex".into(), config };
        assert!(clients::is_configured(&setup), "{name}");
        assert!(clients::has_key(&setup, "torro_chatgpt_abc"), "{name}");
        assert!(!clients::has_key(&setup, "torro_chatgpt_ab"), "{name}: a prefix of the key is not the key");
    }
    let absent = ClientSetup::CodexCli { executable: "/x/codex".into(), config: directory.join("none.toml") };
    assert!(!clients::is_configured(&absent));
}

#[cfg(unix)]
#[test]
fn a_symlinked_config_stays_a_symlink() {
    let directory = scratch("symlink");
    let real = directory.join("dotfiles/mcp.json");
    std::fs::create_dir_all(real.parent().expect("a parent")).expect("directory");
    std::fs::write(&real, "{}").expect("writable");
    let link = directory.join("mcp.json");
    std::os::unix::fs::symlink(&real, &link).expect("a symlink");

    let setup = ClientSetup::McpServersJson { config: link.clone() };
    clients::add(&setup, "/opt/t", "torro_cursor_1", &no_tool).expect("adds");

    assert!(std::fs::symlink_metadata(&link).expect("exists").file_type().is_symlink());
    assert!(std::fs::read_to_string(&real).expect("readable").contains("torro_cursor_1"));
}

#[test]
fn keys_are_long_random_and_never_shown_whole() {
    let first = clients::mint_token("cursor").expect("randomness");
    let second = clients::mint_token("cursor").expect("randomness");
    assert!(first.starts_with("torro_cursor_") && first.len() == "torro_cursor_".len() + 64);
    assert_ne!(first, second);
    assert_eq!(clients::masked_token("hermes"), "torro_hermes_••••••••••••");
    assert_eq!(
        clients::sha256_hex("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn the_status_document_carries_a_key_hash_never_a_key() {
    let home = scratch("status");
    std::fs::create_dir_all(home.join(".cursor")).expect("cursor");
    let environment = Environment { platform: Platform::Linux, home: home.clone(), executable_directories: vec![] };
    let cursor = clients::installed_client(&environment, "cursor").expect("cursor is installed");
    clients::add(&cursor.setup, "/opt/t", "torro_cursor_secret", &no_tool).expect("adds");

    let status = clients::status_json(&environment);
    assert!(!status.to_string().contains("torro_cursor_secret"));
    let entries = status["clients"].as_array().expect("clients");
    assert_eq!(entries.len(), clients::CATALOG.len(), "every catalog client is listed, installed or not");
    let entry = entries.iter().find(|entry| entry["id"] == "cursor").expect("cursor");
    assert_eq!(entry["installed"], true);
    assert_eq!(entry["configured"], true);
    assert_eq!(entry["token_sha256"], clients::sha256_hex("torro_cursor_secret"));
    let hermes = entries.iter().find(|entry| entry["id"] == "hermes").expect("hermes");
    assert_eq!((&hermes["kind"], &hermes["installed"]), (&serde_json::json!("manual"), &serde_json::json!(false)));
}

#[test]
fn the_catalog_carries_the_ids_the_macos_app_stores_keys_under() {
    // The Swift contract suite asserts the same list: access keys are stored
    // by client id, so an id renamed on one side would orphan a key.
    let ids: Vec<&str> = clients::CATALOG.iter().map(|descriptor| descriptor.id).collect();
    assert_eq!(
        ids,
        ["claude-desktop", "claude-code", "opencode", "pi", "chatgpt", "gemini-cli", "cursor", "lm-studio", "vscode", "windsurf",
         "clawbot", "hermes", "other"]
    );
}


#[test]
fn pi_gets_its_own_file_and_says_what_it_still_needs() {
    let home = scratch("pi");
    std::fs::create_dir_all(home.join(".pi/agent")).expect("pi is installed");
    let environment = Environment { platform: Platform::Linux, home: home.clone(), executable_directories: vec![] };
    let pi = clients::installed_client(&environment, "pi").expect("pi is found");
    assert_eq!(pi.setup, ClientSetup::McpServersJson { config: home.join(".pi/agent/mcp.json") });

    let requirement = clients::descriptor("pi").expect("in the catalog").requirement.expect("pi needs its adapter");
    assert_eq!(requirement.install_command, "pi install npm:pi-mcp-adapter");
    assert!(!requirement.is_met(&home), "a bare Pi speaks no MCP");
    let status = clients::status_json(&environment);
    let entry = status["clients"].as_array().expect("clients").iter().find(|entry| entry["id"] == "pi").expect("pi").clone();
    assert_eq!(entry["requirement"]["met"], false);

    // Either trace of the installed adapter counts.
    std::fs::write(home.join(".pi/agent/settings.json"), r#"{"packages":["npm:pi-mcp-adapter"]}"#).expect("writable");
    assert!(requirement.is_met(&home));
    std::fs::remove_file(home.join(".pi/agent/settings.json")).expect("removable");
    std::fs::create_dir_all(home.join(".pi/agent/npm/node_modules/pi-mcp-adapter")).expect("installed");
    assert!(requirement.is_met(&home));

    // The entry is the standard shape, in Pi's own file — never the shared one.
    clients::add(&pi.setup, "/opt/torromail-mcp", "torro_pi_1", &no_tool).expect("adds");
    assert!(clients::has_key(&pi.setup, "torro_pi_1"));
    assert!(!home.join(".config/mcp/mcp.json").exists());
}
