use std::cell::Cell;
use std::rc::Rc;

use torromail_core::{
    AccountId, CoreError, CoreResult, FixtureMailProvider, MailProvider, MarkChange, SearchHit,
    SearchWindow, StoredMessage,
};
use torromail_mcp::{AccessLevel, LineMcpServer, ToolCatalog, ToolName, TransportMode};

/// A fixture mailbox that fails its first `list_mailboxes` with a connection
/// error, then behaves — a stand-in for a pooled session the server dropped
/// while idle.
struct FailingOnce {
    inner: FixtureMailProvider,
    healthy: Cell<bool>,
}

impl FailingOnce {
    fn new(inner: FixtureMailProvider) -> Self {
        Self {
            inner,
            healthy: Cell::new(false),
        }
    }
}

impl MailProvider for FailingOnce {
    fn search(
        &self,
        account_id: &AccountId,
        query: &str,
        mailbox: Option<&str>,
        limit: usize,
        window: &SearchWindow,
    ) -> CoreResult<Vec<SearchHit>> {
        self.inner.search(account_id, query, mailbox, limit, window)
    }

    fn get_message(&self, account_id: &AccountId, message_id: &str) -> CoreResult<StoredMessage> {
        self.inner.get_message(account_id, message_id)
    }

    fn get_thread(
        &self,
        account_id: &AccountId,
        thread_id: &str,
    ) -> CoreResult<Vec<StoredMessage>> {
        self.inner.get_thread(account_id, thread_id)
    }

    fn mark(
        &mut self,
        account_id: &AccountId,
        message_id: &str,
        change: MarkChange,
    ) -> CoreResult<()> {
        self.inner.mark(account_id, message_id, change)
    }

    fn list_mailboxes(&self, account_id: &AccountId) -> CoreResult<Vec<String>> {
        if !self.healthy.get() {
            self.healthy.set(true);
            return Err(CoreError::ProviderFailure("IMAP connection closed".to_owned()));
        }
        self.inner.list_mailboxes(account_id)
    }

    fn append_draft(
        &mut self,
        account_id: &AccountId,
        mailbox: &str,
        message: &str,
    ) -> CoreResult<()> {
        self.inner.append_draft(account_id, mailbox, message)
    }

    fn move_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
        target: &str,
    ) -> CoreResult<()> {
        self.inner.move_messages(account_id, message_ids, target)
    }

    fn expunge_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
    ) -> CoreResult<()> {
        self.inner.expunge_messages(account_id, message_ids)
    }
}

/// One INBOX message for `work`, so `list_mailboxes` has an accessible folder
/// to report.
fn one_message_mailbox() -> FixtureMailProvider {
    FixtureMailProvider::new([StoredMessage::new(
        AccountId::new("work"),
        "INBOX",
        "m1",
        "thread-1",
        "Subject",
        "sender@example.com",
        "snippet",
        "body",
    )])
}

/// A policy document naming `work` with full read — no `imap` block, so with
/// fixtures enabled the account runs in fixture mode.
fn fixture_account_document(path: &std::path::Path) {
    std::fs::write(
        path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");
}

const LIST_MAILBOXES: &str = r#"{"jsonrpc":"2.0","id":40,"method":"tools/call","params":{"name":"mail_list_mailboxes","arguments":{"account_id":"work"}}}"#;

#[test]
fn a_second_call_reuses_the_pooled_connection() {
    let path = temp_policy_path("pool-reuse");
    fixture_account_document(&path);

    let opens = Rc::new(Cell::new(0usize));
    let counter = opens.clone();
    let server = LineMcpServer::with_connect_override(path.clone(), true, move |_account_id| {
        counter.set(counter.get() + 1);
        Ok(Box::new(one_message_mailbox()) as Box<dyn MailProvider>)
    });

    let first = server.handle_line(LIST_MAILBOXES).expect("a response");
    let second = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(first.contains("INBOX"));
    assert!(second.contains("INBOX"));
    // Two calls, one login: the second reused the pooled connection.
    assert_eq!(opens.get(), 1);
}

#[test]
fn a_stale_connection_is_rebuilt_within_the_same_call() {
    let path = temp_policy_path("pool-reconnect");
    fixture_account_document(&path);

    let opens = Rc::new(Cell::new(0usize));
    let counter = opens.clone();
    let server = LineMcpServer::with_connect_override(path.clone(), true, move |_account_id| {
        let opened = counter.get();
        counter.set(opened + 1);
        // The first session is stale and fails once; the rebuilt one works.
        if opened == 0 {
            Ok(Box::new(FailingOnce::new(one_message_mailbox())) as Box<dyn MailProvider>)
        } else {
            Ok(Box::new(one_message_mailbox()) as Box<dyn MailProvider>)
        }
    });

    let response = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    // The stale session's failure was absorbed: rebuilt once, answered.
    assert!(response.contains("INBOX"), "got: {response}");
    assert!(!response.contains("error"), "got: {response}");
    assert_eq!(opens.get(), 2);
}

#[test]
fn a_stale_connection_that_stays_broken_gives_up_after_one_rebuild() {
    let path = temp_policy_path("pool-persistent-failure");
    fixture_account_document(&path);

    let opens = Rc::new(Cell::new(0usize));
    let counter = opens.clone();
    // Every session fails its first list — the rebuild cannot save this call.
    let server = LineMcpServer::with_connect_override(path.clone(), true, move |_account_id| {
        counter.set(counter.get() + 1);
        Ok(Box::new(FailingOnce::new(one_message_mailbox())) as Box<dyn MailProvider>)
    });

    let response = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#""code":-32000"#), "got: {response}");
    // Opened once, rebuilt once, then stopped — never an endless loop.
    assert_eq!(opens.get(), 2);
}

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
fn every_catalog_tool_is_implemented() {
    let server = LineMcpServer::fixture();
    for name in ToolCatalog::default().names_as_str() {
        let response = server
            .handle_line(&format!(
                r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"{name}","arguments":{{}}}}}}"#
            ))
            .expect("a request gets a response");
        // Every catalog tool is dispatched now: a bad call fails on its own
        // terms, never on the "not implemented yet" fallback.
        assert!(
            !response.contains("not implemented yet"),
            "{name} still falls through to the unimplemented stub: {response}"
        );
    }
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
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"mail_mark","arguments":{"account_id":"work","message_ids":["m1"],"mark":"seen"}}}"#,
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
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"mail_mark","arguments":{"account_id":"work","message_ids":["m1"],"mark":"starred"}}}"#,
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

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"mail_mark","arguments":{"account_id":"work","message_ids":["m1"],"mark":"seen"}}}"#,
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
fn an_unconfigured_account_reports_it_rather_than_serving_fixtures() {
    let path = temp_policy_path("unconfigured");
    // A real account, fully permissioned, whose setup was never finished —
    // exactly what the app publishes between "Add Account" and a working
    // login.
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":"invoice"}}}"#,
        )
        .expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    // The assistant must hear "not set up", never the fixture mailbox: demo
    // messages presented as this account's mail is a lie it cannot detect.
    assert!(response.contains(r#""code":-32000"#));
    assert!(response.contains("no connection configured"));
    assert!(!response.contains("result-set-1"));
    assert!(!response.contains("Quarterly invoice"));
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

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());
    let response = server.handle_line(
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"mail_list_mailboxes","arguments":{"account_id":"work"}}}"#,
    ).expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains("INBOX"));
    assert!(!response.contains("Archive"));
}

#[test]
fn mail_list_accounts_names_the_accounts_without_being_told_one() {
    let path = temp_policy_path("list-accounts");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","name":"Work","email":"me@example.com","read":"full_message","write":{"drafts":true},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    // No account_id: this tool is where one is learned, so demanding one
    // would leave a fresh client with no way in.
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"mail_list_accounts","arguments":{}}}"#,
        )
        .expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#"\"account_id\":\"work\""#));
    assert!(response.contains("Work"));
    assert!(response.contains("me@example.com"));
}

#[test]
fn admin_reads_never_touch_a_mailbox() {
    // Connection facts that would fail hard if anything tried to use them:
    // the admin tools must answer from the document alone.
    let path = temp_policy_path("admin-offline");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","name":"Work","email":"me@example.com","read":"full_message","write":{"drafts":true},"send":false,"per_folder":false,"folder_rules":{},"imap":{"host":"unreachable.invalid","port":993,"username":"me@example.com","secret_ref":"keychain://TorroMail/nonexistent"}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let accounts = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"mail_list_accounts","arguments":{}}}"#,
        )
        .expect("a request gets a response");
    let cache = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"mail_get_cache_status","arguments":{}}}"#,
        )
        .expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    assert!(!accounts.contains("error"));
    assert!(!cache.contains("error"));
    assert!(accounts.contains(r#"\"connected\":true"#));
}

#[test]
fn mail_get_policy_reports_the_switches_and_what_they_permit() {
    let path = temp_policy_path("get-policy");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","name":"Work","email":"me@example.com","read":"full_message","write":{"drafts":true,"mark":false},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"mail_get_policy","arguments":{"account_id":"work"}}}"#,
        )
        .expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    // The switches as the user set them...
    assert!(response.contains(r#"\"read\":\"full_message\""#));
    assert!(response.contains(r#"\"mark\":false"#));
    // ...and the whole verdict they add up to. Pinned as a set, so the
    // absent rights — mark, send, move, delete — are part of the assertion.
    assert!(response.contains(
        r#"\"capabilities\":[\"search\",\"read_headers\",\"read_body\",\"draft\"]"#
    ));
}

#[test]
fn mail_get_policy_refuses_accounts_the_document_does_not_name() {
    let path = temp_policy_path("get-policy-unknown");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"personal","read":"headers","write":{},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":16,"method":"tools/call","params":{"name":"mail_get_policy","arguments":{"account_id":"work"}}}"#,
        )
        .expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#""code":-32000"#));
    assert!(response.contains("account not found"));
}

#[test]
fn mail_get_cache_status_reports_the_published_facts() {
    let path = temp_policy_path("cache-status");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true},"send":false,"per_folder":false,"folder_rules":{},"cache":{"local_cache_enabled":true,"mode":"headers","index_bodies":false,"index_attachments":false,"storage":"12 MB"}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path(path.clone());
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"mail_get_cache_status","arguments":{"account_id":"work"}}}"#,
        )
        .expect("a request gets a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#"\"mode\":\"headers\""#));
    assert!(response.contains("12 MB"));
}

#[test]
fn an_empty_query_asks_for_the_latest_mail_instead_of_failing() {
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":18,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work"}}}"#,
        )
        .expect("a request gets a response");

    // "What came in lately?" has no query to give, so no query must mean no
    // filter — everything the fixture holds, not an error.
    assert!(response.contains("Quarterly invoice"));
    assert!(response.contains("Team notes"));
}

#[test]
fn a_malformed_search_date_is_refused_before_touching_the_mailbox() {
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","since":"last tuesday"}}}"#,
        )
        .expect("a request gets a response");

    // A bad date is the client's mistake, not a server failure: invalid
    // params, and no search runs.
    assert!(response.contains(r#""code":-32602"#));
    assert!(response.contains("ISO date"));
}

#[test]
fn a_well_formed_search_date_is_accepted() {
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","since":"2026-07-01"}}}"#,
        )
        .expect("a request gets a response");

    // The fixture ignores the window, but the date parsed and the search ran.
    assert!(response.contains("result-set-1"));
    assert!(!response.contains("error"));
}

#[test]
fn a_search_can_be_refined_by_its_id() {
    let server = LineMcpServer::fixture();

    // The fixture holds two messages; an empty query returns both.
    let search = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":50,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":""}}}"#,
        )
        .expect("a response");
    assert!(search.contains("result-set-1"));
    assert!(search.contains("Quarterly invoice"));
    assert!(search.contains("Team notes"));

    // Refining the stored set by id keeps only the matching hit — no rescan,
    // and no account or connection needed.
    let refined = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":51,"method":"tools/call","params":{"name":"mail_refine_search","arguments":{"result_set_id":"result-set-1","refinement":"team"}}}"#,
        )
        .expect("a response");
    assert!(refined.contains("Team notes"), "got: {refined}");
    assert!(!refined.contains("Quarterly invoice"), "got: {refined}");
}

#[test]
fn refining_an_unknown_result_set_is_refused() {
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":52,"method":"tools/call","params":{"name":"mail_refine_search","arguments":{"result_set_id":"result-set-999","refinement":"x"}}}"#,
        )
        .expect("a response");

    assert!(response.contains(r#""code":-32000"#));
    assert!(response.contains("result set not found"));
}

#[test]
fn a_thread_returns_every_message_that_shares_its_id() {
    let path = temp_policy_path("thread");
    fixture_account_document(&path);

    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account_id| {
        Ok(Box::new(FixtureMailProvider::new([
            StoredMessage::new(
                AccountId::new("work"),
                "INBOX",
                "m1",
                "thread-1",
                "Original question",
                "a@example.com",
                "s",
                "body one",
            ),
            StoredMessage::new(
                AccountId::new("work"),
                "INBOX",
                "m2",
                "thread-1",
                "Re: Original question",
                "b@example.com",
                "s",
                "body two",
            ),
            StoredMessage::new(
                AccountId::new("work"),
                "INBOX",
                "m3",
                "thread-2",
                "Something unrelated",
                "c@example.com",
                "s",
                "body three",
            ),
        ])) as Box<dyn MailProvider>)
    });

    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":60,"method":"tools/call","params":{"name":"mail_get_thread","arguments":{"account_id":"work","thread_id":"thread-1","include_bodies":true}}}"#,
        )
        .expect("a response");
    std::fs::remove_file(&path).ok();

    // Both messages of thread-1, and not the one from thread-2.
    assert!(response.contains("Original question"), "got: {response}");
    assert!(response.contains("Re: Original question"), "got: {response}");
    assert!(!response.contains("Something unrelated"), "got: {response}");
    // Bodies were requested, so they travel too.
    assert!(response.contains("body one") && response.contains("body two"));
}

#[test]
fn a_draft_is_composed_and_appended() {
    // The fixture account runs the read + drafts default, so drafting is
    // allowed. No Drafts folder exists, so the append falls back to "Drafts".
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":70,"method":"tools/call","params":{"name":"mail_create_draft","arguments":{"account_id":"work","to":["someone@example.com"],"subject":"Hallo","body":"Kurzer Text."}}}"#,
        )
        .expect("a response");

    assert!(response.contains("draft_created"), "got: {response}");
    assert!(response.contains("Drafts"), "got: {response}");
}

#[test]
fn a_draft_needs_at_least_one_recipient() {
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"mail_create_draft","arguments":{"account_id":"work","to":[],"subject":"Hallo","body":"Text"}}}"#,
        )
        .expect("a response");

    assert!(response.contains(r#""code":-32602"#), "got: {response}");
}

#[test]
fn drafting_is_refused_without_the_draft_right() {
    let path = temp_policy_path("no-draft");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":false},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":72,"method":"tools/call","params":{"name":"mail_create_draft","arguments":{"account_id":"work","to":["someone@example.com"],"subject":"Hallo","body":"Text"}}}"#,
        )
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#""code":-32000"#), "got: {response}");
    assert!(response.contains("Draft"), "got: {response}");
}

/// Pull a string field out of a tool response whose payload is JSON escaped
/// inside the JSON-RPC text block (so quotes appear as `\"`).
fn payload_field(response: &str, key: &str) -> String {
    let needle = format!("\\\"{key}\\\":\\\"");
    let start = response.find(&needle).expect("field present") + needle.len();
    let rest = &response[start..];
    let end = rest.find("\\\"").expect("field end");
    rest[..end].to_owned()
}

fn tidy_up_document(path: &std::path::Path) {
    std::fs::write(
        path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true,"mark":true,"move":true,"trash":true},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");
}

#[test]
fn a_prepared_move_executes_only_after_confirmation() {
    let path = temp_policy_path("prepare-move");
    tidy_up_document(&path);

    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account_id| {
        Ok(Box::new(FixtureMailProvider::new([StoredMessage::new(
            AccountId::new("work"),
            "INBOX",
            "m1",
            "thread-1",
            "Subject",
            "s@example.com",
            "snippet",
            "body",
        )])) as Box<dyn MailProvider>)
    });

    let prepare = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":80,"method":"tools/call","params":{"name":"mail_prepare_move","arguments":{"account_id":"work","message_ids":["m1"],"target_mailbox":"Archive"}}}"#,
        )
        .expect("a response");
    assert!(prepare.contains("pending_action_id"), "got: {prepare}");
    assert!(prepare.contains("Move 1 message(s) to Archive"), "got: {prepare}");

    let pending_id = payload_field(&prepare, "pending_action_id");
    let code = payload_field(&prepare, "confirmation_code");

    let confirm = server
        .handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":81,"method":"tools/call","params":{{"name":"mail_confirm_action","arguments":{{"pending_action_id":"{pending_id}","confirmation_code":"{code}"}}}}}}"#
        ))
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(confirm.contains("moved"), "got: {confirm}");
}

#[test]
fn a_wrong_confirmation_code_is_refused() {
    let path = temp_policy_path("wrong-code");
    tidy_up_document(&path);
    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());

    let prepare = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":82,"method":"tools/call","params":{"name":"mail_prepare_delete","arguments":{"account_id":"work","message_ids":["m1"]}}}"#,
        )
        .expect("a response");
    let pending_id = payload_field(&prepare, "pending_action_id");

    let confirm = server
        .handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":83,"method":"tools/call","params":{{"name":"mail_confirm_action","arguments":{{"pending_action_id":"{pending_id}","confirmation_code":"not-the-code"}}}}}}"#
        ))
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(confirm.contains(r#""code":-32602"#), "got: {confirm}");
}

#[test]
fn preparing_a_move_needs_the_move_right() {
    let path = temp_policy_path("no-move");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true,"move":false},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");
    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());

    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":84,"method":"tools/call","params":{"name":"mail_prepare_move","arguments":{"account_id":"work","message_ids":["m1"],"target_mailbox":"Archive"}}}"#,
        )
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#""code":-32000"#), "got: {response}");
    assert!(response.contains("Move"), "got: {response}");
}

#[test]
fn preparing_a_permanent_delete_needs_the_permanent_right() {
    // trash is granted, permanent delete is not — the escalation the presets
    // never include.
    let path = temp_policy_path("no-permanent");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"trash":true,"permanent_delete":false},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");
    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());

    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":85,"method":"tools/call","params":{"name":"mail_prepare_delete","arguments":{"account_id":"work","message_ids":["m1"],"permanent":true}}}"#,
        )
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#""code":-32000"#), "got: {response}");
    assert!(response.contains("DeletePermanent"), "got: {response}");
}

#[test]
fn confirming_an_unknown_action_is_refused() {
    let server = LineMcpServer::fixture();
    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":86,"method":"tools/call","params":{"name":"mail_confirm_action","arguments":{"pending_action_id":"pending-999","confirmation_code":"abc"}}}"#,
        )
        .expect("a response");

    assert!(response.contains(r#""code":-32000"#), "got: {response}");
    assert!(response.contains("pending action not found"), "got: {response}");
}

fn send_capable_document(path: &std::path::Path) {
    std::fs::write(
        path,
        r#"{"version":1,"accounts":[{"id":"work","email":"me@example.com","read":"full_message","write":{"drafts":true},"send":true,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");
}

const CREATE_DRAFT: &str = r#"{"jsonrpc":"2.0","id":90,"method":"tools/call","params":{"name":"mail_create_draft","arguments":{"account_id":"work","to":["someone@example.com"],"subject":"Hallo","body":"Text"}}}"#;

#[test]
fn a_draft_can_be_prepared_for_sending() {
    let path = temp_policy_path("prepare-send");
    send_capable_document(&path);
    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account_id| {
        Ok(Box::new(FixtureMailProvider::new([])) as Box<dyn MailProvider>)
    });

    let draft = server.handle_line(CREATE_DRAFT).expect("a response");
    assert!(draft.contains("draft_id"), "got: {draft}");
    let draft_id = payload_field(&draft, "draft_id");

    let prepare = server
        .handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":91,"method":"tools/call","params":{{"name":"mail_prepare_send","arguments":{{"account_id":"work","draft_id":"{draft_id}"}}}}}}"#
        ))
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(prepare.contains("pending_action_id"), "got: {prepare}");
    assert!(prepare.contains("Send to someone@example.com"), "got: {prepare}");
}

#[test]
fn confirming_a_send_needs_smtp_to_be_configured() {
    // Send is granted and a draft exists, but the document carries no smtp
    // block, so the confirmed send has nowhere to go.
    let path = temp_policy_path("send-no-smtp");
    send_capable_document(&path);
    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account_id| {
        Ok(Box::new(FixtureMailProvider::new([])) as Box<dyn MailProvider>)
    });

    let draft = server.handle_line(CREATE_DRAFT).expect("a response");
    let draft_id = payload_field(&draft, "draft_id");
    let prepare = server
        .handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":92,"method":"tools/call","params":{{"name":"mail_prepare_send","arguments":{{"account_id":"work","draft_id":"{draft_id}"}}}}}}"#
        ))
        .expect("a response");
    let pending_id = payload_field(&prepare, "pending_action_id");
    let code = payload_field(&prepare, "confirmation_code");

    let confirm = server
        .handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":93,"method":"tools/call","params":{{"name":"mail_confirm_action","arguments":{{"pending_action_id":"{pending_id}","confirmation_code":"{code}"}}}}}}"#
        ))
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(confirm.contains(r#""code":-32000"#), "got: {confirm}");
    assert!(confirm.contains("no SMTP"), "got: {confirm}");
}

#[test]
fn preparing_a_send_needs_the_send_right() {
    let path = temp_policy_path("no-send");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true},"send":false,"per_folder":false,"folder_rules":{}}]}"#,
    )
    .expect("policy document written");
    let server = LineMcpServer::with_connect_override(path.clone(), true, |_account_id| {
        Ok(Box::new(FixtureMailProvider::new([])) as Box<dyn MailProvider>)
    });

    let draft = server.handle_line(CREATE_DRAFT).expect("a response");
    let draft_id = payload_field(&draft, "draft_id");
    let prepare = server
        .handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":94,"method":"tools/call","params":{{"name":"mail_prepare_send","arguments":{{"account_id":"work","draft_id":"{draft_id}"}}}}}}"#
        ))
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(prepare.contains(r#""code":-32000"#), "got: {prepare}");
    assert!(prepare.contains("Send"), "got: {prepare}");
}

#[test]
fn preparing_a_send_from_an_unknown_draft_is_refused() {
    let path = temp_policy_path("send-unknown-draft");
    send_capable_document(&path);
    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());

    let response = server
        .handle_line(
            r#"{"jsonrpc":"2.0","id":95,"method":"tools/call","params":{"name":"mail_prepare_send","arguments":{"account_id":"work","draft_id":"draft-404"}}}"#,
        )
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(response.contains(r#""code":-32000"#), "got: {response}");
    assert!(response.contains("draft not found"), "got: {response}");
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

/// The schema must promise only what the tools actually do. Both of these
/// were advertised and then ignored — a client that trusts them is misled.
#[test]
fn the_schema_does_not_advertise_arguments_the_tools_ignore() {
    let manifest = ToolCatalog::default().to_mcp_tools_json();

    // Attachments are never fetched, so mail_get_message must not offer to
    // include them.
    assert!(!manifest.contains("include_attachments"));
    // A message id already carries its mailbox; mail_mark reads the mailbox
    // from there, never from a separate argument.
    assert!(!manifest.contains(r#"required":["account_id","mailbox""#));
}

#[test]
fn an_xoauth2_account_is_accepted_and_reaches_the_keychain() {
    // Byte-for-byte what the Swift app publishes for a Gmail account. There
    // is no token in this test's keychain, so the furthest this can get is
    // the lookup — which is the point: getting that far proves the document
    // parsed, the mechanism was understood, and the renewal facts were taken.
    let path = temp_policy_path("xoauth2");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"gmail","name":"Sven","email":"sven@gmail.com","read":"full_message","write":{},"send":false,"per_folder":false,"folder_rules":{},"imap":{"host":"imap.gmail.com","port":993,"username":"sven@gmail.com","secret_ref":"keychain://TorroMail/torromail-absent-test-account","auth":"xoauth2","token_endpoint":"https://oauth2.googleapis.com/token","client_id":"abc.apps.googleusercontent.com"}}]}"#,
    )
    .expect("policy document written");

    let error = torromail_mcp::check_account("gmail", Some(path.clone()), None)
        .expect_err("no token is stored for this account");
    std::fs::remove_file(&path).ok();

    assert!(
        error.contains("keychain"),
        "should fail at the secret, not before it — got: {error}"
    );
}

#[test]
fn an_xoauth2_account_without_renewal_facts_is_refused() {
    // Half a contract is worse than none: this would connect once and then
    // fail an hour later with nothing to point at.
    let path = temp_policy_path("xoauth2-incomplete");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"gmail","read":"headers","write":{},"send":false,"per_folder":false,"folder_rules":{},"imap":{"host":"imap.gmail.com","username":"s@gmail.com","secret_ref":"keychain://TorroMail/gmail","auth":"xoauth2"}}]}"#,
    )
    .expect("policy document written");

    let error = torromail_mcp::check_account("gmail", Some(path.clone()), None)
        .expect_err("an xoauth2 block without a token endpoint must not parse");
    std::fs::remove_file(&path).ok();

    assert!(error.contains("token_endpoint"), "got: {error}");
}

#[test]
fn an_unknown_auth_mechanism_is_refused_rather_than_guessed_at() {
    // Guessing "password" here would send the stored secret as a cleartext
    // LOGIN to a server that asked for something else.
    let path = temp_policy_path("unknown-auth");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"headers","write":{},"send":false,"per_folder":false,"folder_rules":{},"imap":{"host":"imap.example.com","username":"w","secret_ref":"keychain://TorroMail/work","auth":"ntlm"}}]}"#,
    )
    .expect("policy document written");

    let error = torromail_mcp::check_account("work", Some(path.clone()), None)
        .expect_err("an unknown mechanism must not fall back to a password");
    std::fs::remove_file(&path).ok();

    assert!(error.contains("ntlm"), "got: {error}");
}

// --- Client pairing -------------------------------------------------------
//
// The app hands every connected assistant an access key and publishes the
// key's SHA-256 in the document's `clients` allowlist. These tests pin the
// gate's contract: who gets in, what a refusal says, and when the list is
// not enforced at all.

/// The access key the paired-client tests present, and its SHA-256 exactly
/// as the app would publish it.
const TEST_KEY: &str = "torro_test-client_4fa1c2d8e6b7a9503f0e1d2c3b4a5968";
const TEST_KEY_SHA256: &str = "81727766d13995a1eca96addd0da70cb703ee6f73161f8fb6689def24b9ee4cc";

/// A document naming `work` in fixture mode — like `fixture_account_document`
/// — whose allowlist admits exactly `TEST_KEY`.
fn paired_fixture_document(path: &std::path::Path) {
    std::fs::write(
        path,
        format!(
            r#"{{"version":1,"accounts":[{{"id":"work","read":"full_message","write":{{"drafts":true}},"send":false,"per_folder":false,"folder_rules":{{}}}}],"clients":[{{"id":"test-client","name":"Test Client","token_sha256":"{TEST_KEY_SHA256}"}}]}}"#
        ),
    )
    .expect("policy document written");
}

const LIST_ACCOUNTS: &str = r#"{"jsonrpc":"2.0","id":70,"method":"tools/call","params":{"name":"mail_list_accounts","arguments":{}}}"#;

#[test]
fn an_unpaired_client_is_refused_every_tool_call() {
    let path = temp_policy_path("gate-unpaired");
    paired_fixture_document(&path);

    // No token presented at all — a process that just spawned the binary.
    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());

    let admin = server.handle_line(LIST_ACCOUNTS).expect("a response");
    let mailboxes = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    // The admin tools too: account names and permissions are none of an
    // unpaired process's business.
    for refusal in [&admin, &mailboxes] {
        assert!(refusal.contains("-32001"), "got: {refusal}");
        assert!(refusal.contains("not paired with TorroMail"), "got: {refusal}");
        assert!(refusal.contains("MCP Clients"), "the refusal must say where to fix it");
    }
}

#[test]
fn initialize_and_tools_list_stay_open_to_an_unpaired_client() {
    // The client has to be able to connect and see the catalog — the first
    // tool call is where the refusal explains itself. Failing the handshake
    // instead would bury the message in a client log.
    let path = temp_policy_path("gate-handshake");
    paired_fixture_document(&path);

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());

    let initialize = server
        .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .expect("a response");
    let tools = server
        .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
        .expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(initialize.contains("TorroMail"), "got: {initialize}");
    assert!(tools.contains("mail_search"), "got: {tools}");
    assert!(!initialize.contains("-32001"));
    assert!(!tools.contains("-32001"));
}

#[test]
fn a_wrong_key_reads_as_revoked_not_as_unpaired() {
    let path = temp_policy_path("gate-wrong-key");
    paired_fixture_document(&path);

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone())
        .with_presented_token(Some("torro_intruder_ffffffffffffffffffffffffffffffff"));

    let refusal = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(refusal.contains("-32001"), "got: {refusal}");
    assert!(
        refusal.contains("revoked or is not valid"),
        "a stale key should read as revoked, so the human reconnects rather than re-installs — got: {refusal}"
    );
}

#[test]
fn a_paired_client_passes_the_gate() {
    let path = temp_policy_path("gate-paired");
    paired_fixture_document(&path);

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone())
        .with_presented_token(Some(TEST_KEY));

    let accounts = server.handle_line(LIST_ACCOUNTS).expect("a response");
    let mailboxes = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(accounts.contains("work"), "got: {accounts}");
    assert!(mailboxes.contains("INBOX"), "got: {mailboxes}");
}

#[test]
fn revoking_a_key_locks_the_client_out_mid_session() {
    let path = temp_policy_path("gate-revoked-live");
    paired_fixture_document(&path);

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone())
        .with_presented_token(Some(TEST_KEY));

    let before = server.handle_line(LIST_MAILBOXES).expect("a response");

    // The user disconnects the client in the app: the allowlist is
    // republished without it. The server process is still running.
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true},"send":false,"per_folder":false,"folder_rules":{}}],"clients":[]}"#,
    )
    .expect("policy document written");

    let after = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(before.contains("INBOX"), "got: {before}");
    assert!(
        after.contains("-32001") && after.contains("revoked"),
        "revocation must bite the running session, not the next one — got: {after}"
    );
}

#[test]
fn a_document_without_a_clients_key_enforces_nothing() {
    // Pre-pairing documents (and hand-managed ones) keep working; the app
    // closes the gap the moment it republishes with an allowlist.
    let path = temp_policy_path("gate-legacy");
    fixture_account_document(&path);

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone());

    let mailboxes = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(mailboxes.contains("INBOX"), "got: {mailboxes}");
}

#[test]
fn a_malformed_clients_entry_fails_closed() {
    // A present-but-broken allowlist must not be read as "no allowlist" —
    // that would turn a truncated write into an open door.
    let path = temp_policy_path("gate-malformed");
    std::fs::write(
        &path,
        r#"{"version":1,"accounts":[{"id":"work","read":"full_message","write":{"drafts":true},"send":false,"per_folder":false,"folder_rules":{}}],"clients":[{"id":"test-client"}]}"#,
    )
    .expect("policy document written");

    let server = LineMcpServer::with_policy_path_and_fixtures(path.clone())
        .with_presented_token(Some(TEST_KEY));

    let refusal = server.handle_line(LIST_MAILBOXES).expect("a response");
    std::fs::remove_file(&path).ok();

    assert!(refusal.contains("-32000"), "got: {refusal}");
    assert!(refusal.contains("token_sha256"), "got: {refusal}");
}

#[test]
fn check_account_sits_behind_the_pairing_gate() {
    let path = temp_policy_path("gate-check-account");
    std::fs::write(
        &path,
        format!(
            r#"{{"version":1,"accounts":[{{"id":"work","read":"headers","write":{{}},"send":false,"per_folder":false,"folder_rules":{{}},"imap":{{"host":"imap.example.com","port":993,"username":"w","secret_ref":"keychain://TorroMail/torromail-absent-test-account"}}}}],"clients":[{{"id":"test-client","name":"Test Client","token_sha256":"{TEST_KEY_SHA256}"}}]}}"#
        ),
    )
    .expect("policy document written");

    let unpaired = torromail_mcp::check_account("work", Some(path.clone()), None)
        .expect_err("no key, no login check");
    let wrong = torromail_mcp::check_account(
        "work",
        Some(path.clone()),
        Some("torro_intruder_ffffffffffffffffffffffffffffffff"),
    )
    .expect_err("a wrong key must not reach the mailbox");
    // The right key passes the gate and fails at the (absent) keychain
    // secret — proof the gate, not the account, was what refused above.
    let through = torromail_mcp::check_account("work", Some(path.clone()), Some(TEST_KEY))
        .expect_err("no secret is stored for this account");
    std::fs::remove_file(&path).ok();

    assert!(unpaired.contains("not paired"), "got: {unpaired}");
    assert!(wrong.contains("revoked or is not valid"), "got: {wrong}");
    assert!(through.contains("keychain"), "got: {through}");
}

// --- Audit log ------------------------------------------------------------
//
// Every tool call appends one JSONL line beside the policy document, so the
// app's activity view and log have something real to show. These pin the
// shape and the client attribution.

/// A policy path inside a fresh directory, so the `audit.jsonl` the server
/// writes as a sibling is isolated from every other test's calls.
fn temp_audit_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("torromail-audit-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("audit test directory");
    dir
}

#[test]
fn a_tool_call_appends_an_audit_line_attributing_the_client() {
    let dir = temp_audit_dir("attribution");
    let policy = dir.join("policy.json");
    paired_fixture_document(&policy);

    let server = LineMcpServer::with_policy_path_and_fixtures(policy)
        .with_presented_token(Some(TEST_KEY));
    server.handle_line(LIST_ACCOUNTS).expect("a response");

    let log = std::fs::read_to_string(dir.join("audit.jsonl")).expect("an audit line was written");
    std::fs::remove_dir_all(&dir).ok();

    let entry: serde_json::Value =
        serde_json::from_str(log.lines().next().expect("one line")).expect("valid json line");
    assert_eq!(entry["tool"], "mail_list_accounts");
    assert_eq!(entry["result"], "ok");
    assert_eq!(entry["client_id"], "test-client");
    assert_eq!(entry["client"], "Test Client");
    assert!(entry["ts"].is_u64(), "the timestamp is a machine number: {entry}");
}

#[test]
fn a_refused_tool_call_is_logged_as_an_error() {
    let dir = temp_audit_dir("error-result");
    let policy = dir.join("policy.json");
    paired_fixture_document(&policy);

    // A paired client asking for an account the document does not name: the
    // gate passes, the account lookup refuses, and that refusal is the line.
    let server = LineMcpServer::with_policy_path_and_fixtures(policy)
        .with_presented_token(Some(TEST_KEY));
    let missing = r#"{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"mail_list_mailboxes","arguments":{"account_id":"nope"}}}"#;
    server.handle_line(missing).expect("a response");

    let log = std::fs::read_to_string(dir.join("audit.jsonl")).expect("an audit line was written");
    std::fs::remove_dir_all(&dir).ok();

    let entry: serde_json::Value =
        serde_json::from_str(log.lines().next().expect("one line")).expect("valid json line");
    assert_eq!(entry["tool"], "mail_list_mailboxes");
    assert_eq!(entry["account"], "nope");
    assert_eq!(entry["result"], "error");
}

#[test]
fn an_unpaired_refusal_is_still_logged_for_the_owner_to_see() {
    let dir = temp_audit_dir("unpaired");
    let policy = dir.join("policy.json");
    paired_fixture_document(&policy);

    // No token: the gate refuses before dispatch, but the owner still wants
    // to know something knocked — logged as an unknown client.
    let server = LineMcpServer::with_policy_path_and_fixtures(policy);
    server.handle_line(LIST_ACCOUNTS).expect("a response");

    let log = std::fs::read_to_string(dir.join("audit.jsonl")).expect("an audit line was written");
    std::fs::remove_dir_all(&dir).ok();

    let entry: serde_json::Value =
        serde_json::from_str(log.lines().next().expect("one line")).expect("valid json line");
    assert_eq!(entry["tool"], "mail_list_accounts");
    assert_eq!(entry["result"], "error");
    assert_eq!(entry["client_id"], "unknown");
}

/// Every line of the isolated audit log, parsed.
fn audit_lines(dir: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("audit.jsonl"))
        .expect("an audit log was written")
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid json line"))
        .collect()
}

#[test]
fn audit_detail_names_what_a_search_and_draft_touched() {
    let dir = temp_audit_dir("detail-read");
    let policy = dir.join("policy.json");
    paired_fixture_document(&policy);

    let server = LineMcpServer::with_policy_path_and_fixtures(policy)
        .with_presented_token(Some(TEST_KEY));
    server
        .handle_line(r#"{"jsonrpc":"2.0","id":90,"method":"tools/call","params":{"name":"mail_search","arguments":{"account_id":"work","query":"Rechnung","mailbox":"INBOX"}}}"#)
        .expect("a response");
    server
        .handle_line(r#"{"jsonrpc":"2.0","id":91,"method":"tools/call","params":{"name":"mail_create_draft","arguments":{"account_id":"work","to":["kunde@example.com"],"subject":"Angebot"}}}"#)
        .expect("a response");

    let lines = audit_lines(&dir);
    std::fs::remove_dir_all(&dir).ok();

    let search = &lines[0];
    assert_eq!(search["tool"], "mail_search");
    // The query and the mailbox it scanned — what was retrieved, not just that
    // a search happened.
    assert_eq!(search["detail"], "Rechnung · INBOX");

    let draft = &lines[1];
    assert_eq!(draft["tool"], "mail_create_draft");
    assert_eq!(draft["detail"], "kunde@example.com — Angebot");
}

#[test]
fn audit_detail_follows_a_move_from_intent_to_execution() {
    let dir = temp_audit_dir("detail-move");
    let policy = dir.join("policy.json");
    // Move granted, and an allowlist so the line is attributed to a real client.
    std::fs::write(
        &policy,
        format!(
            r#"{{"version":1,"accounts":[{{"id":"work","read":"full_message","write":{{"move":true}},"send":false,"per_folder":false,"folder_rules":{{}}}}],"clients":[{{"id":"test-client","name":"Test Client","token_sha256":"{TEST_KEY_SHA256}"}}]}}"#
        ),
    )
    .expect("policy document written");

    let server = LineMcpServer::with_connect_override(policy, true, |_account_id| {
        Ok(Box::new(FixtureMailProvider::new([StoredMessage::new(
            AccountId::new("work"),
            "INBOX",
            "m1",
            "thread-1",
            "Subject",
            "s@example.com",
            "snippet",
            "body",
        )])) as Box<dyn MailProvider>)
    })
    .with_presented_token(Some(TEST_KEY));

    let prepare = server
        .handle_line(r#"{"jsonrpc":"2.0","id":92,"method":"tools/call","params":{"name":"mail_prepare_move","arguments":{"account_id":"work","message_ids":["m1"],"target_mailbox":"Archive"}}}"#)
        .expect("a response");
    let pending_id = payload_field(&prepare, "pending_action_id");
    let code = payload_field(&prepare, "confirmation_code");
    server
        .handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":93,"method":"tools/call","params":{{"name":"mail_confirm_action","arguments":{{"pending_action_id":"{pending_id}","confirmation_code":"{code}"}}}}}}"#
        ))
        .expect("a response");

    let lines = audit_lines(&dir);
    std::fs::remove_dir_all(&dir).ok();

    // The intent: what would move, and where.
    assert_eq!(lines[0]["tool"], "mail_prepare_move");
    assert_eq!(lines[0]["detail"], "1 → Archive");
    // The execution: read from the result, so the confirm line stands on its
    // own rather than only referencing a pending id.
    assert_eq!(lines[1]["tool"], "mail_confirm_action");
    assert_eq!(lines[1]["detail"], "moved 1 → Archive");
    assert_eq!(lines[1]["result"], "ok");
    assert_eq!(lines[1]["client"], "Test Client");
}
