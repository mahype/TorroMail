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
