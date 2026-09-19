//! Provider inference as shared cases: `contracts/provider-discovery.json` is
//! run here and by the Swift contract suite, so an address leads to the same
//! servers and the same login path on every surface.

use serde_json::{Value, json};
use torromail_control::{mailbox_names, providers};

fn contract_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/provider-discovery.json")
}

fn auth_json(auth: Option<providers::AuthPath>) -> Value {
    match auth {
        None => Value::Null,
        Some(providers::AuthPath::AppPassword { setup_url }) => json!({ "kind": "app_password", "setup_url": setup_url }),
        Some(providers::AuthPath::Password) => json!({ "kind": "password" }),
        Some(providers::AuthPath::OAuth(issuer)) => json!({ "kind": "oauth", "issuer": issuer.as_str() }),
    }
}

#[test]
fn the_shared_discovery_cases_resolve_as_written() {
    let mut file: Value =
        serde_json::from_str(&std::fs::read_to_string(contract_path()).expect("the shared cases exist")).expect("JSON");
    let bless = std::env::var_os("TORROMAIL_BLESS").is_some();
    let config = |found: Option<providers::DiscoveredConfig>| found.map_or(Value::Null, |found| found.to_json());

    type Resolve<'a> = &'a dyn Fn(&Value) -> Value;
    let sections: [(&str, Resolve<'_>); 7] = [
        ("domains", &|case| config(providers::lookup(case["input"].as_str().expect("input")))),
        ("mx", &|case| config(providers::from_mx_host(case["input"].as_str().expect("input")))),
        ("spf", &|case| config(providers::from_spf(case["input"].as_str().expect("input")))),
        ("app_password_fallback", &|case| {
            auth_json(providers::lookup(case["input"].as_str().expect("input")).and_then(|found| providers::app_password_fallback(&found)))
        }),
        ("email_domains", &|case| json!(providers::domain_of(case["input"].as_str().expect("input")))),
        ("autoconfig", &|case| {
            config(providers::parse_autoconfig(case["xml"].as_str().expect("xml"), case["source"].as_str().expect("source")))
        }),
        ("mailbox_names", &|case| json!(mailbox_names::display_name(case["input"].as_str().expect("input")))),
    ];
    for (section, resolve) in sections {
        let cases = file[section].as_array_mut().unwrap_or_else(|| panic!("{section} is a list"));
        assert!(!cases.is_empty(), "{section}");
        for case in cases.iter_mut() {
            let resolved = resolve(case);
            if bless {
                case["expected"] = resolved;
            } else {
                assert_eq!(resolved, case["expected"], "{section}: {}", case.get("name").unwrap_or(&case["input"]));
            }
        }
    }
    if bless {
        let text = serde_json::to_string_pretty(&file).expect("serializes");
        std::fs::write(contract_path(), text + "\n").expect("writable");
    }
}

#[test]
fn autoconfig_is_asked_at_the_domain_first_then_at_its_well_known_path() {
    assert_eq!(
        providers::autoconfig_urls("torro.dev", "sven+mail@torro.dev"),
        [
            "https://autoconfig.torro.dev/mail/config-v1.1.xml?emailaddress=sven+mail@torro.dev",
            "https://torro.dev/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=sven+mail@torro.dev",
        ]
    );
    assert!(providers::autoconfig_urls("x.de", "a b@x.de")[0].ends_with("emailaddress=a%20b@x.de"));
}

#[test]
fn an_mx_host_is_reduced_to_the_tail_the_table_is_keyed_by() {
    assert_eq!(providers::base_domain("ASPMX.L.Google.com"), "google.com");
    assert_eq!(providers::base_domain("localhost"), "localhost");
}
