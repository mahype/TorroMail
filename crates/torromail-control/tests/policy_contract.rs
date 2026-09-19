//! The policy document both configuration surfaces must publish, as shared
//! cases: `contracts/policy-document.json` is run here and by the Swift
//! contract suite, so the Rust writer and `PolicyDocument` in TorroMailKit
//! cannot come to disagree without one suite going red.

use serde_json::Value;
use torromail_control::policy::{self, ClientAccountAccess, ClientPairing, PolicyContext};
use torromail_control::MailAccount;

fn contract_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/policy-document.json")
}

fn pairing(value: &Value) -> ClientPairing {
    let access = &value["account_access"];
    ClientPairing {
        id: value["id"].as_str().expect("client id").to_owned(),
        name: value["name"].as_str().expect("client name").to_owned(),
        token_sha256: value["token_sha256"].as_str().expect("token hash").to_owned(),
        account_access: match access["mode"].as_str() {
            Some("selected") => ClientAccountAccess::Selected(
                access["account_ids"]
                    .as_array()
                    .expect("selected ids")
                    .iter()
                    .map(|id| id.as_str().expect("id").to_owned())
                    .collect(),
            ),
            _ => ClientAccountAccess::All,
        },
    }
}

#[test]
fn the_shared_policy_cases_publish_as_written() {
    let mut file: Value = serde_json::from_str(
        &std::fs::read_to_string(contract_path()).expect("the shared cases are in the repository"),
    )
    .expect("the shared cases are JSON");
    // The placeholders stand for themselves here; the Swift suite swaps in the
    // ids its build carries.
    let context = PolicyContext {
        secret_ref_prefix: file["secret_ref_prefix"].as_str().expect("prefix").to_owned(),
        google_client_id: "$GOOGLE_CLIENT_ID".to_owned(),
        microsoft_client_id: "$MICROSOFT_CLIENT_ID".to_owned(),
    };
    let bless = std::env::var_os("TORROMAIL_BLESS").is_some();

    let cases = file["cases"].as_array_mut().expect("cases is a list");
    assert!(!cases.is_empty());
    for case in cases.iter_mut() {
        let name = case["name"].as_str().expect("every case is named").to_owned();
        let accounts: Vec<MailAccount> = case["accounts"]
            .as_array()
            .expect("accounts is a list")
            .iter()
            .map(|account| MailAccount::from_json(account).unwrap_or_else(|error| panic!("{name}: {error}")))
            .collect();
        let clients: Vec<ClientPairing> =
            case["clients"].as_array().expect("clients is a list").iter().map(pairing).collect();
        let published = policy::document(&accounts, &clients, &context);
        if bless {
            case["expected"] = published;
        } else {
            assert_eq!(published, case["expected"], "{name}");
        }
    }
    if bless {
        let text = serde_json::to_string_pretty(&file).expect("the cases serialize");
        std::fs::write(contract_path(), text + "\n").expect("the contract file is writable");
    }
}
