use donnymail_mcp::{AccessLevel, ToolCatalog, ToolName, TransportMode};

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
fn gui_only_mutations_are_not_exposed_as_mcp_tools() {
    let catalog = ToolCatalog::default();
    let names = catalog.names_as_str();

    assert!(!names.contains(&"mail_add_account"));
    assert!(!names.contains(&"mail_update_secret"));
    assert!(!names.contains(&"mail_set_permissions"));
    assert!(!names.contains(&"mail_enable_oauth"));
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
