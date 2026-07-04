use donnymail_core::{
    AccountDraft, AccountId, AccountRegistry, ActionKind, CachePolicy, Capability, Channel,
    PendingActionRequest, PendingActionStore, Policy, PolicyEngine, SearchHit, SearchSessionStore,
};

#[test]
fn account_mutations_are_gui_only() {
    let mut registry = AccountRegistry::default();
    let draft = AccountDraft::imap_password(
        AccountId::new("work"),
        "Work",
        "work@example.com",
        "imap.example.com",
        "smtp.example.com",
    );

    let mcp_result = registry.add_account(Channel::McpTool, draft.clone());
    assert!(mcp_result.is_err());
    assert!(registry.list_accounts().is_empty());

    let gui_result = registry.add_account(Channel::Gui, draft);
    assert!(gui_result.is_ok());
    assert_eq!(registry.list_accounts().len(), 1);
}

#[test]
fn default_cache_policy_is_metadata_only() {
    let policy = CachePolicy::default();

    assert!(policy.persist_metadata);
    assert!(policy.persist_headers);
    assert!(!policy.persist_bodies);
    assert!(!policy.index_bodies);
    assert!(!policy.index_attachments);
}

#[test]
fn policy_engine_denies_missing_capabilities() {
    let account_id = AccountId::new("personal");
    let policy = Policy::new(
        account_id.clone(),
        [Capability::ReadHeaders, Capability::Search],
    );
    let engine = PolicyEngine::new([policy]);

    assert!(engine.authorize(&account_id, Capability::Search).is_ok());
    assert!(engine.authorize(&account_id, Capability::Send).is_err());
    assert!(
        engine
            .authorize(&account_id, Capability::DeletePermanent)
            .is_err()
    );
}

#[test]
fn risky_actions_require_explicit_confirmation_and_are_single_use() {
    let mut store = PendingActionStore::default();
    let request = PendingActionRequest::new(
        AccountId::new("work"),
        ActionKind::Send,
        "Send message to customer@example.com",
        10,
        70,
    );

    let pending = store.prepare(request, 100);
    assert_eq!(pending.expires_at(), 170);
    assert!(!pending.is_confirmed());

    let confirmed = store.confirm(pending.id(), 120);
    assert!(confirmed.is_ok());

    let second_confirm = store.confirm(pending.id(), 121);
    assert!(second_confirm.is_err());
}

#[test]
fn expired_pending_actions_cannot_be_confirmed() {
    let mut store = PendingActionStore::default();
    let request = PendingActionRequest::new(
        AccountId::new("work"),
        ActionKind::DeleteSoft,
        "Move 2 messages to Trash",
        2,
        30,
    );

    let pending = store.prepare(request, 100);
    let result = store.confirm(pending.id(), 131);

    assert!(result.is_err());
}

#[test]
fn search_result_sets_can_be_refined_in_memory() {
    let mut sessions = SearchSessionStore::default();
    let result_set = sessions.create(
        AccountId::new("work"),
        "invoice",
        vec![
            SearchHit::new(
                "m1",
                "Invoice April",
                "billing@example.com",
                "April invoice attached",
            ),
            SearchHit::new(
                "m2",
                "Team notes",
                "lead@example.com",
                "Budget planning notes",
            ),
            SearchHit::new(
                "m3",
                "Invoice May",
                "billing@example.com",
                "May invoice attached",
            ),
        ],
        100,
        7200,
    );

    let refined = match sessions.refine(result_set.id(), "may billing", 120) {
        Ok(result_set) => result_set,
        Err(error) => panic!("result set should exist: {error}"),
    };

    assert_eq!(refined.query(), "invoice may billing");
    assert_eq!(refined.hits().len(), 1);
    assert_eq!(refined.hits()[0].message_id(), "m3");
}
