//! The writer and the reader of the policy document live in different crates.
//! This is the one place they meet: every document the shared cases describe
//! is published by `torromail-control` and then served from by the real
//! server, so a field one side renames cannot slip past the other.

use torromail_control::policy::{self, ClientAccountAccess, ClientPairing, PolicyContext};
use torromail_control::MailAccount;
use torromail_mcp::LineMcpServer;

const TEST_KEY: &str = "torro_test-client_4fa1c2d8e6b7a9503f0e1d2c3b4a5968";
const TEST_KEY_SHA256: &str = "81727766d13995a1eca96addd0da70cb703ee6f73161f8fb6689def24b9ee4cc";
const LIST_ACCOUNTS: &str =
    r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mail_list_accounts","arguments":{}}}"#;
const GET_POLICY: &str =
    r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mail_get_policy","arguments":{"account_id":"ACCOUNT"}}}"#;

#[test]
fn the_server_serves_from_every_document_the_writer_publishes() {
    let cases_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/policy-document.json");
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(cases_path).expect("the shared cases exist")).expect("JSON");
    let context = PolicyContext {
        secret_ref_prefix: "keychain://TorroMail/".to_owned(),
        google_client_id: "google-client".to_owned(),
        microsoft_client_id: "microsoft-client".to_owned(),
    };
    let client = ClientPairing {
        id: "test-client".to_owned(),
        name: "Test Client".to_owned(),
        token_sha256: TEST_KEY_SHA256.to_owned(),
        account_access: ClientAccountAccess::All,
    };

    for (index, case) in file["cases"].as_array().expect("cases").iter().enumerate() {
        let name = case["name"].as_str().expect("named");
        let accounts: Vec<MailAccount> = case["accounts"]
            .as_array()
            .expect("accounts")
            .iter()
            .map(|account| MailAccount::from_json(account).expect("a contract account reads"))
            .collect();
        let path = std::env::temp_dir().join(format!(
            "torromail-published-{}-{index}/policy.json",
            std::process::id()
        ));
        policy::publish(&path, &accounts, std::slice::from_ref(&client), &context).expect("the document publishes");

        let server = LineMcpServer::with_policy_path(path.clone()).with_presented_token(Some(TEST_KEY));
        let listed = server.handle_line(LIST_ACCOUNTS).expect("a request gets a response");
        for account in &accounts {
            assert!(
                listed.contains(&format!(r#"\"account_id\":\"{}\""#, account.id)),
                "{name}: {} is not listed — got: {listed}",
                account.id
            );
            let policy = server
                .handle_line(&GET_POLICY.replace("ACCOUNT", &account.id))
                .expect("a request gets a response");
            // The rights arrive as published, not merely "something parsed".
            assert!(
                policy.contains(&format!(r#"\"read\":\"{}\""#, account.permissions.read.policy_name())),
                "{name}: {} does not carry its read level — got: {policy}",
                account.id
            );
            assert!(
                policy.contains(&format!(r#"\"send\":{}"#, account.permissions.send)),
                "{name}: got: {policy}"
            );
        }
        std::fs::remove_dir_all(path.parent().expect("a parent")).ok();
    }
}

// MARK: the command another surface calls instead of writing the document itself

fn request_file(name: &str, request: &serde_json::Value) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("torromail-policy-request-{}-{name}.json", std::process::id()));
    std::fs::write(&path, request.to_string()).expect("the request is writable");
    path
}

fn policy_document(request_path: &std::path::Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_torromail-mcp"))
        .arg("--policy-document")
        .arg(request_path)
        // No token, no policy path: this mode must need neither.
        .env_remove("TORROMAIL_TOKEN")
        .env_remove("TORROMAIL_POLICY_PATH")
        .output()
        .expect("the server binary runs")
}

#[test]
fn the_policy_document_command_answers_every_shared_case_as_written() {
    let cases_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/policy-document.json");
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(cases_path).expect("the shared cases exist")).expect("JSON");

    for (index, case) in file["cases"].as_array().expect("cases").iter().enumerate() {
        let name = case["name"].as_str().expect("named");
        let request = serde_json::json!({
            "accounts": case["accounts"],
            "clients": case["clients"],
            "context": {
                "secret_ref_prefix": file["secret_ref_prefix"],
                "google_client_id": "$GOOGLE_CLIENT_ID",
                "microsoft_client_id": "$MICROSOFT_CLIENT_ID",
            },
        });
        let path = request_file(&index.to_string(), &request);
        let output = policy_document(&path);
        std::fs::remove_file(&path).ok();

        assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is the document");
        assert_eq!(document, case["expected"], "{name}");
    }
}

#[test]
fn a_grant_that_cannot_be_understood_fails_the_publication() {
    // Falling back to "all accounts" here would widen a client's access
    // because of a typo. No document is better than a broader one.
    for (label, access) in [
        ("unknown mode", serde_json::json!({ "mode": "everything" })),
        ("all with ids", serde_json::json!({ "mode": "all", "account_ids": ["work"] })),
        ("selected without ids", serde_json::json!({ "mode": "selected" })),
        ("empty id", serde_json::json!({ "mode": "selected", "account_ids": [""] })),
    ] {
        let request = serde_json::json!({
            "accounts": [],
            "clients": [{ "id": "c", "name": "C", "token_sha256": "aa", "account_access": access }],
            "context": { "secret_ref_prefix": "keychain://TorroMail/", "google_client_id": "g", "microsoft_client_id": "m" },
        });
        let path = request_file(&label.replace(' ', "-"), &request);
        let output = policy_document(&path);
        std::fs::remove_file(&path).ok();

        assert_eq!(output.status.code(), Some(1), "{label}");
        assert!(output.stdout.is_empty(), "{label}: nothing is published");
        assert!(String::from_utf8_lossy(&output.stderr).contains("account_access"), "{label}");
    }
}

#[test]
fn a_missing_request_is_an_error_not_an_empty_document() {
    let output = policy_document(std::path::Path::new("/nonexistent/torromail-request.json"));
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

// MARK: client setup from the command line

#[test]
fn client_setup_writes_the_entry_and_client_status_reports_its_hash() {
    let home = std::env::temp_dir().join(format!("torromail-client-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".cursor")).expect("cursor is installed");
    let run = |arguments: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_torromail-mcp"))
            .args(arguments)
            .env("HOME", &home)
            .env("PATH", "")
            .env_remove("TORROMAIL_TOKEN")
            .output()
            .expect("the server binary runs")
    };

    let request = request_file(
        "client-add",
        &serde_json::json!({ "action": "add", "client_id": "cursor", "command_path": "/opt/torromail-mcp", "token": TEST_KEY }),
    );
    let added = run(&["--client-setup", request.to_str().expect("utf-8 path")]);
    std::fs::remove_file(&request).ok();
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));

    let status = run(&["--client-status"]);
    assert!(status.status.success());
    let text = String::from_utf8_lossy(&status.stdout);
    assert!(!text.contains(TEST_KEY), "the key itself never leaves");
    let document: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let cursor = document["clients"]
        .as_array()
        .expect("clients")
        .iter()
        .find(|client| client["id"] == "cursor")
        .expect("cursor is listed");
    assert_eq!(cursor["configured"], true);
    assert_eq!(cursor["token_sha256"], TEST_KEY_SHA256);

    // Not installed: an error, and nothing is created for it.
    let request = request_file(
        "client-missing",
        &serde_json::json!({ "action": "add", "client_id": "windsurf", "command_path": "/opt/t", "token": TEST_KEY }),
    );
    let refused = run(&["--client-setup", request.to_str().expect("utf-8 path")]);
    std::fs::remove_file(&request).ok();
    assert_eq!(refused.status.code(), Some(1));
    assert!(!home.join(".codeium").exists());
    std::fs::remove_dir_all(&home).ok();
}
