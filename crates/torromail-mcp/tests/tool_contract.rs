use torromail_mcp::{AccessLevel, LineMcpServer, ToolCatalog, ToolName, TransportMode};

#[test]
fn stdio_is_the_default_transport() {
    assert_eq!(TransportMode::default(), TransportMode::Stdio);
}

#[test]
fn planned_mail_tools_are_exposed() {
    let catalog = ToolCatalog::default();
    let names = catalog.tool_names();

    assert!(names.contains(&ToolName::MailSearch));
    assert!(names.contains(&ToolName::MailRefineSearch));
    assert!(names.contains(&ToolName::MailGetMessage));
    assert!(names.contains(&ToolName::MailGetThread));
    assert!(names.contains(&ToolName::MailListMailboxes));
    assert!(names.contains(&ToolName::MailMark));
    assert!(names.contains(&ToolName::MailCreateDraft));
    assert!(names.contains(&ToolName::MailPrepareSend));
    assert!(names.contains(&ToolName::MailPrepareMove));
    assert!(names.contains(&ToolName::MailPrepareDelete));
    assert!(names.contains(&ToolName::MailConfirmAction));
}

#[test]
fn admin_tools_are_read_only() {
    let catalog = ToolCatalog::default();

    for name in [
        ToolName::MailListAccounts,
        ToolName::MailGetPolicy,
        ToolName::MailGetCacheStatus,
    ] {
        let tool = match catalog.find(name) {
            Some(tool) => tool,
            None => panic!("admin tool exists: {name:?}"),
        };
        assert_eq!(tool.access_level(), AccessLevel::ReadOnlyAdmin);
    }
}

#[test]
fn mail_mark_is_a_direct_write_without_confirmation() {
    let catalog = ToolCatalog::default();
    let tool = match catalog.find(ToolName::MailMark) {
        Some(tool) => tool,
        None => panic!("mail_mark exists"),
    };

    assert_eq!(tool.access_level(), AccessLevel::DirectWrite);
    assert!(!tool.requires_gui_confirmation());
}

#[test]
fn gui_only_mutations_are_not_exposed_as_mcp_tools() {
    let catalog = ToolCatalog::default();
    let names = catalog.names_as_str();

    assert!(!names.contains(&"mail_add_account"));
    assert!(!names.contains(&"mail_update_secret"));
    assert!(!names.contains(&"mail_set_permissions"));
    assert!(!names.contains(&"mail_enable_oauth"));
}

/// The handshake Claude Desktop actually performs. Answering the
/// notification — or answering anything with a null id — makes the client
/// reject the whole session.
#[test]
fn notifications_are_never_answered() {
    let server = LineMcpServer::fixture();

    assert!(
        server
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none(),
        "a notification carries no id and gets no response"
    );
    assert!(
        server.handle_line("not json at all").is_none(),
        "an unparsable line has no id to answer to"
    );
    assert!(
        server
            .handle_line(r#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#)
            .is_none(),
        "a null id is not an id"
    );
}

#[test]
fn initialize_reports_tool_capability() {
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#)
        .expect("a request gets a response");

    assert!(response.contains(r#""id":0"#));
    assert!(response.contains(r#""protocolVersion""#));
    assert!(response.contains(r#""tools""#));
    assert!(!response.contains("error"));
}

#[test]
fn unknown_methods_answer_with_the_requests_own_id() {
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(r#"{"jsonrpc":"2.0","id":"abc","method":"resources/list"}"#)
        .expect("a request gets a response");

    assert!(response.contains(r#""id":"abc""#));
    assert!(response.contains(r#""code":-32601"#));
}

#[test]
fn mcp_server_executes_fixture_backed_mail_search() {
    let server = LineMcpServer::fixture();
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":"invoice","mailbox":"INBOX","limit":10}}}"#,
    ).expect("a request gets a response");

    assert!(response.contains(r#""id":1"#));
    assert!(response.contains("result-set-1"));
    assert!(response.contains("Quarterly invoice"));
}

#[test]
fn mcp_server_rejects_unknown_tool_calls() {
    let server = LineMcpServer::fixture();
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mail_add_account","arguments":{}}}"#,
    ).expect("a request gets a response");

    assert!(response.contains(r#""code":-32601"#));
    assert!(response.contains("tool not found"));
}

#[test]
fn known_but_unimplemented_tools_say_so_instead_of_vanishing() {
    let server = LineMcpServer::fixture();
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mail_get_thread","arguments":{}}}"#,
    ).expect("a request gets a response");

    assert!(response.contains(r#""code":-32000"#));
    assert!(response.contains("not implemented"));
}

#[test]
fn mcp_server_reads_single_messages_through_the_policy() {
    let server = LineMcpServer::fixture();
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mail_get_message","arguments":{"account_id":"work","message_id":"m1","include_body":true}}}"#,
    ).expect("a request gets a response");

    assert!(response.contains(r#""id":4"#));
    assert!(response.contains("Quarterly invoice"));
    assert!(response.contains("Invoice body"));
}

#[test]
fn mcp_server_enforces_the_mark_permission() {
    let server = LineMcpServer::fixture();
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"mail_mark","arguments":{"account_id":"work","mailbox":"INBOX","message_ids":["m1"],"mark":"seen"}}}"#,
    ).expect("a request gets a response");

    // The fixture account runs on the read + drafts default, so marking is
    // exactly what the policy must refuse.
    assert!(response.contains(r#""code":-32000"#));
    assert!(response.contains("Mark is not allowed"));
}

#[test]
fn mail_mark_rejects_unknown_flag_names() {
    let server = LineMcpServer::fixture();
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"mail_mark","arguments":{"account_id":"work","mailbox":"INBOX","message_ids":["m1"],"mark":"starred"}}}"#,
    ).expect("a request gets a response");

    assert!(response.contains(r#""code":-32602"#));
}

fn temp_policy_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "torromail-policy-{}-{name}.json",
        std::process::id()
    ))
}

#[test]
fn policy_document_permissions_reach_the_tools() {
    let path = temp_policy_path("grants");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":false,"mark":true,"move":false,"trash":false,"permanent_delete":false},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"mail_mark","arguments":{"account_id":"work","mailbox":"INBOX","message_ids":["m1"],"mark":"seen"}}}"#,
    ).expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    // The same call the default policy refuses succeeds once the document
    // grants the mark permission. The payload travels as escaped JSON text.
    assert!(response.contains(r#"\"seen\":true"#));
}

#[test]
fn accounts_missing_from_the_policy_document_are_refused() {
    let path = temp_policy_path("other-account");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"personal","read":"headers","write":{},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":"invoice"}}}"#,
    ).expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#""code":-32000"#));
    assert!(response.contains("account not found"));
}

#[test]
fn corrupt_policy_documents_fail_closed() {
    let path = temp_policy_path("corrupt");
    std::fs::write(&path, "not json at all").expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":"invoice"}}}"#,
    ).expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#""code":-32000"#));
    assert!(response.contains("policy document invalid"));
}

#[test]
fn a_missing_policy_document_falls_back_to_the_product_default() {
    let server = LineMcpServer::with_policy_path(temp_policy_path("never-written"));
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":"invoice"}}}"#,
    ).expect("a request gets a response");

    assert!(response.contains("result-set-1"));
}

#[test]
fn mail_list_mailboxes_hides_blocked_folders() {
    let path = temp_policy_path("list-mailboxes");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true},"send":false,"per_folder":true,"folder_rules":{"Archive":{"read":false,"write":false}}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"mail_list_mailboxes","arguments":{"account_id":"work"}}}"#,
    ).expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains("INBOX"));
    assert!(!response.contains("Archive"));
}

#[test]
fn tool_list_serializes_without_secrets_or_local_paths() {
    let manifest = ToolCatalog::default().to_mcp_tools_json();

    assert!(manifest.contains("\"mail_search\""));
    assert!(manifest.contains("\"result_set_id\""));
    assert!(!manifest.contains("password"));
    assert!(!manifest.contains("token"));
    assert!(!manifest.contains("/Users/"));
}
