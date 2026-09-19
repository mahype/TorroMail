use std::ffi::OsString;
use std::path::PathBuf;

use torromail_control::clients::Platform;
use torromail_control::{logs, paths};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("torromail-logs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

#[test]
fn the_newest_entries_are_kept_and_a_broken_line_is_skipped() {
    let path = scratch("audit").join("audit.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"ts":100,"client":"Codex","account":"work","tool":"mail_search","detail":"invoice","result":"ok"}"#, "\n",
            r#"{"ts":101.5,"client":"Codex","tool":"mail_list_accounts","result":"ok"}"#, "\n",
            "{ half a li\n",
            r#"{"ts":102,"client":"Claude Code","account":"work","tool":"mail_get_message","detail":"INBOX/7","result":"error"}"#, "\n",
        ),
    )
    .expect("writable");

    let entries = logs::load_audit(&path, 2);
    assert_eq!(entries.len(), 2);
    assert_eq!((entries[0].timestamp, entries[0].account.as_str(), entries[0].detail.as_str()), (101.5, "", ""));
    assert_eq!((entries[1].tool.as_str(), entries[1].result.as_str()), ("mail_get_message", "error"));
    assert!(logs::load_audit(&scratch("none").join("audit.jsonl"), 10).is_empty());
}

#[test]
fn each_client_keeps_its_latest_handshake() {
    let path = scratch("connections").join("connections.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"ts":200,"client_id":"claude-code","client_name":"claude-code","client_version":"2.1.3"}"#, "\n",
            r#"{"ts":300,"client_id":"claude-code","client_name":"claude-code","client_version":"2.1.4"}"#, "\n",
            r#"{"ts":250,"client_id":"cursor"}"#, "\n",
            r#"{"ts":400,"client_id":""}"#, "\n",
        ),
    )
    .expect("writable");

    let latest = logs::latest_connections(&path);
    assert_eq!(latest.len(), 2);
    assert_eq!(latest["claude-code"].reported_version, "2.1.4");
    assert_eq!(latest["cursor"].last_connected, 250.0);
}

#[test]
fn the_data_directory_on_windows_is_the_local_profile() {
    let profile = PathBuf::from("C:\\Users\\sven");
    assert_eq!(
        paths::data_directory(Platform::Windows, Some(profile.clone().into_os_string()), None),
        Some(profile.join("AppData/Local/TorroMail")),
        "the local half: the mail cache lives here and must not roam"
    );
    assert_eq!(paths::data_directory(Platform::Windows, None, None), None, "no profile, no invented location");
    assert_eq!(paths::program_name("torromail-mcp"), format!("torromail-mcp{}", std::env::consts::EXE_SUFFIX));
}

// `/data/state` is an absolute path only where paths start with a slash.
#[cfg(unix)]
#[test]
fn the_data_directory_follows_the_platform() {
    let home = || Some(OsString::from("/home/sven"));
    assert_eq!(
        paths::data_directory(Platform::MacOs, home(), None),
        Some(PathBuf::from("/home/sven/Library/Application Support/TorroMail"))
    );
    assert_eq!(
        paths::data_directory(Platform::Linux, home(), None),
        Some(PathBuf::from("/home/sven/.local/state/torromail"))
    );
    assert_eq!(
        paths::data_directory(Platform::Linux, home(), Some(OsString::from("/data/state"))),
        Some(PathBuf::from("/data/state/torromail"))
    );
    assert_eq!(
        paths::data_directory(Platform::Linux, home(), Some(OsString::from("relative/state"))),
        Some(PathBuf::from("/home/sven/.local/state/torromail")),
        "a relative XDG value is invalid and ignored"
    );
    assert_eq!(paths::data_directory(Platform::Linux, None, None), None, "no home, no invented location");
}


#[test]
fn an_export_cannot_be_made_to_run_as_a_formula() {
    let entry = |detail: &str| logs::AuditEntry {
        timestamp: 100.5,
        client: "Codex".to_owned(),
        account: "work".to_owned(),
        tool: "mail_search".to_owned(),
        detail: detail.to_owned(),
        result: "ok".to_owned(),
    };
    let csv = logs::audit_csv(&[entry(r#"=HYPERLINK("http://evil","x")"#), entry("say \"hi\", twice"), entry("-2+3")]);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "timestamp,client,account,tool,detail,result");
    assert!(lines[1].contains(r#""'=HYPERLINK(""http://evil"",""x"")""#), "got: {}", lines[1]);
    assert!(lines[2].contains(r#""say ""hi"", twice""#), "got: {}", lines[2]);
    assert!(lines[3].contains(r#""'-2+3""#), "got: {}", lines[3]);
}
