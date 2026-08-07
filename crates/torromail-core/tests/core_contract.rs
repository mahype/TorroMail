use torromail_core::{
    AccountDraft, AccountId, AccountRegistry, ActionKind, AttachmentInfo, CachePolicy, Capability,
    Channel,
    FixtureMailProvider, FolderRule, MailAccessService, MailProvider, MarkChange,
    OutgoingAttachment, PendingActionRequest, PendingActionStore, PermissionPreset, PermissionSet,
    Policy, PolicyEngine, ReadAccess, SearchHit, SearchSessionStore, SearchWindow, StoredMessage,
    WriteAccess, compose_message, compose_message_with_attachments,
};

#[test]
fn compose_message_builds_headers_and_encodes_a_non_ascii_subject() {
    let message = compose_message(
        "me@example.com",
        &["a@example.com".to_owned()],
        &["c@example.com".to_owned()],
        "Grüße",
        "Zeile eins\nZeile zwei",
    );

    assert!(message.contains("From: me@example.com\r\n"));
    assert!(message.contains("To: a@example.com\r\n"));
    assert!(message.contains("Cc: c@example.com\r\n"));
    // A non-ASCII subject travels as an RFC 2047 encoded-word.
    assert!(message.contains("Subject: =?UTF-8?B?"), "{message}");
    // The body's bare newline became CRLF.
    assert!(message.contains("\r\nZeile eins\r\nZeile zwei\r\n"), "{message}");
}

#[test]
fn compose_message_builds_a_multipart_attachment() {
    let attachment = OutgoingAttachment::new(
        "Prüfung #2.pdf",
        "application/pdf",
        vec![0, 1, 2, 255],
    );
    let message = compose_message_with_attachments(
        "me@example.com",
        &["a@example.com".to_owned()],
        &[],
        "Dokument",
        "Anbei.",
        &[attachment],
    );

    assert!(message.contains("Content-Type: multipart/mixed; boundary=\"=_TorroMail_mixed_1\""));
    assert!(message.contains("Content-Type: text/plain; charset=utf-8\r\n"));
    assert!(message.contains("Content-Type: application/pdf; name*=UTF-8''Pr%C3%BCfung%20#2.pdf\r\n"));
    assert!(message.contains(
        "Content-Disposition: attachment; filename*=UTF-8''Pr%C3%BCfung%20#2.pdf\r\n"
    ));
    assert!(message.contains("Content-Transfer-Encoding: base64\r\n\r\nAAEC/w==\r\n"));
    assert!(message.ends_with("--=_TorroMail_mixed_1--\r\n"));
}

#[test]
fn attachment_base64_is_wrapped_and_the_boundary_cannot_collide_with_the_body() {
    let attachment = OutgoingAttachment::new("bytes.bin", "application/octet-stream", vec![7; 60]);
    let message = compose_message_with_attachments(
        "me@example.com",
        &["a@example.com".to_owned()],
        &[],
        "Bytes",
        "A deliberate --=_TorroMail_mixed_1 collision.",
        &[attachment],
    );

    assert!(message.contains("boundary=\"=_TorroMail_mixed_2\""));
    let encoded = message
        .split("Content-Transfer-Encoding: base64\r\n\r\n")
        .nth(1)
        .expect("attachment body")
        .split("--=_TorroMail_mixed_2--")
        .next()
        .expect("closing boundary");
    let lines = encoded.trim().split("\r\n").collect::<Vec<_>>();
    assert_eq!(lines.iter().map(|line| line.len()).collect::<Vec<_>>(), [76, 4]);
}

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
fn new_accounts_start_at_read_and_drafts() {
    let permissions = PermissionSet::default();

    assert_eq!(
        permissions.matching_preset(),
        Some(PermissionPreset::ReadAndDrafts)
    );
    assert!(!permissions.send);
}

#[test]
fn presets_are_derived_and_drift_to_custom() {
    let mut permissions = PermissionSet::default();
    permissions.apply(PermissionPreset::FullAccess);
    assert_eq!(
        permissions.matching_preset(),
        Some(PermissionPreset::FullAccess)
    );

    permissions.write.mark = false;
    assert_eq!(permissions.matching_preset(), None);
}

#[test]
fn no_preset_enables_permanent_deletion() {
    for preset in PermissionPreset::ALL {
        assert!(!preset.write().permanent_delete, "{preset:?}");
    }
}

#[test]
fn permanent_delete_falls_with_the_trash_right() {
    let write = WriteAccess {
        trash: false,
        permanent_delete: true,
        ..WriteAccess::NOTHING
    };

    assert!(!write.sanitized().permanent_delete);
}

#[test]
fn folder_exceptions_scope_the_groups_but_never_exceed_them() {
    let mut permissions = PermissionSet::default();
    permissions.per_folder = true;
    permissions.folder_rules.insert(
        "Private".to_owned(),
        FolderRule {
            read: false,
            write: false,
        },
    );
    permissions.folder_rules.insert(
        "Archive".to_owned(),
        FolderRule {
            read: true,
            write: false,
        },
    );

    assert!(!permissions.can_access("Private"));
    assert_eq!(permissions.read_access_in("INBOX"), ReadAccess::FullMessage);
    assert!(permissions.write_access_in("INBOX").drafts);
    assert_eq!(
        permissions.read_access_in("Archive"),
        ReadAccess::FullMessage
    );
    assert!(permissions.write_access_in("Archive").is_empty());

    // Switching per-folder off keeps the exceptions stored, just inert.
    permissions.per_folder = false;
    assert!(permissions.can_access("Private"));
    assert!(permissions.folder_rules.contains_key("Private"));
}

#[test]
fn policy_engine_authorizes_from_the_permission_groups() {
    let account_id = AccountId::new("personal");
    let mut permissions = PermissionSet::default();
    permissions.per_folder = true;
    permissions.folder_rules.insert(
        "Private".to_owned(),
        FolderRule {
            read: false,
            write: false,
        },
    );
    let engine = PolicyEngine::new([Policy::new(account_id.clone(), permissions)]);

    assert!(engine.authorize(&account_id, Capability::Search).is_ok());
    assert!(engine.authorize(&account_id, Capability::ReadBody).is_ok());
    assert!(
        engine
            .authorize(&account_id, Capability::DownloadAttachments)
            .is_err()
    );
    assert!(engine.authorize(&account_id, Capability::Draft).is_ok());
    assert!(engine.authorize(&account_id, Capability::Send).is_err());
    assert!(
        engine
            .authorize(&account_id, Capability::DeletePermanent)
            .is_err()
    );
    assert!(
        engine
            .authorize_in(&account_id, "INBOX", Capability::ReadHeaders)
            .is_ok()
    );
    assert!(
        engine
            .authorize_in(&account_id, "Private", Capability::ReadHeaders)
            .is_err()
    );
}

#[test]
fn moving_needs_write_on_both_ends() {
    let account_id = AccountId::new("work");
    let mut permissions = PermissionSet::default();
    permissions.apply(PermissionPreset::TidyUp);
    permissions.per_folder = true;
    permissions.folder_rules.insert(
        "Archive".to_owned(),
        FolderRule {
            read: true,
            write: false,
        },
    );
    let engine = PolicyEngine::new([Policy::new(account_id.clone(), permissions)]);

    assert!(
        engine
            .authorize_move(&account_id, "INBOX", "Processed")
            .is_ok()
    );
    assert!(
        engine
            .authorize_move(&account_id, "INBOX", "Archive")
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
fn imap_provider_config_keeps_secrets_out_of_debug_output() {
    let config = torromail_core::ImapProviderConfig::new(
        AccountId::new("work"),
        "imap.example.com",
        993,
        "work@example.com",
        torromail_core::SecretRef::new("keychain://torromail/work"),
    );

    let debug = format!("{config:?}");
    assert!(debug.contains("imap.example.com"));
    assert!(!debug.contains("keychain://torromail/work"));
}

#[test]
fn secret_refs_name_their_keychain_location() {
    let secret = torromail_core::SecretRef::new("keychain://TorroMail/work");
    assert_eq!(secret.keychain_location(), Some(("TorroMail", "work")));

    let opaque = torromail_core::SecretRef::new("vault:12345");
    assert_eq!(opaque.keychain_location(), None);
}

#[test]
fn fixture_provider_searches_and_reads_messages_without_ui_state() {
    let account_id = AccountId::new("work");
    let provider = FixtureMailProvider::new([
        StoredMessage::new(
            account_id.clone(),
            "INBOX",
            "m1",
            "thread-1",
            "Quarterly invoice",
            "billing@example.com",
            "The quarterly invoice is attached.",
            "Invoice body",
        ),
        StoredMessage::new(
            account_id.clone(),
            "Archive",
            "m2",
            "thread-2",
            "Team notes",
            "lead@example.com",
            "Planning notes",
            "Planning body",
        ),
    ]);

    let hits = provider
        .search(&account_id, "invoice", Some("INBOX"), 10, &SearchWindow::default())
        .expect("fixture search succeeds");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id(), "m1");
    assert_eq!(hits[0].mailbox(), "INBOX");

    let message = provider
        .get_message(&account_id, "m1")
        .expect("message exists");
    assert_eq!(message.subject(), "Quarterly invoice");
    assert_eq!(message.body(), "Invoice body");
}

#[test]
fn mail_access_service_enforces_search_and_body_policy() {
    let account_id = AccountId::new("work");
    let provider = FixtureMailProvider::new([StoredMessage::new(
        account_id.clone(),
        "INBOX",
        "m1",
        "thread-1",
        "Quarterly invoice",
        "billing@example.com",
        "The quarterly invoice is attached.",
        "Invoice body",
    )]);
    // Subject and sender only: search and headers pass, the body stays shut.
    let mut permissions = PermissionSet::default();
    permissions.read = ReadAccess::Headers;
    permissions.write = WriteAccess::NOTHING;
    let engine = PolicyEngine::new([Policy::new(account_id.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    let result_set = service
        .search(&account_id, "invoice", Some("INBOX"), 10, &SearchWindow::default(), 100)
        .expect("search is allowed at header level");
    assert_eq!(result_set.hits().len(), 1);

    let header_only = service
        .get_message(&account_id, "m1", false)
        .expect("headers are allowed");
    assert_eq!(header_only.body(), "");

    assert!(service.get_message(&account_id, "m1", true).is_err());
}

#[test]
fn mail_access_service_keeps_blocked_folders_invisible() {
    let account_id = AccountId::new("work");
    let provider = FixtureMailProvider::new([
        StoredMessage::new(
            account_id.clone(),
            "INBOX",
            "m1",
            "thread-1",
            "Invoice April",
            "billing@example.com",
            "April invoice attached",
            "April body",
        ),
        StoredMessage::new(
            account_id.clone(),
            "Private",
            "m2",
            "thread-2",
            "Invoice personal",
            "friend@example.net",
            "Personal invoice attached",
            "Personal body",
        ),
    ]);
    let mut permissions = PermissionSet::default();
    permissions.per_folder = true;
    permissions.folder_rules.insert(
        "Private".to_owned(),
        FolderRule {
            read: false,
            write: false,
        },
    );
    let engine = PolicyEngine::new([Policy::new(account_id.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    // A search across every folder must not leak the blocked one.
    let result_set = service
        .search(&account_id, "invoice", None, 10, &SearchWindow::default(), 100)
        .expect("account-wide search is allowed");
    assert_eq!(result_set.hits().len(), 1);
    assert_eq!(result_set.hits()[0].message_id(), "m1");

    // Not even headers escape a blocked folder.
    assert!(service.get_message(&account_id, "m2", false).is_err());

    // The folder listing keeps the blocked folder to itself entirely.
    let mailboxes = service
        .list_mailboxes(&account_id)
        .expect("listing is allowed");
    assert_eq!(mailboxes, vec!["INBOX".to_owned()]);
}

#[test]
fn marking_respects_the_mark_permission_and_folder_rules() {
    let account_id = AccountId::new("work");
    let provider = FixtureMailProvider::new([
        StoredMessage::new(
            account_id.clone(),
            "INBOX",
            "m1",
            "thread-1",
            "Quarterly invoice",
            "billing@example.com",
            "The quarterly invoice is attached.",
            "Invoice body",
        ),
        StoredMessage::new(
            account_id.clone(),
            "Private",
            "m2",
            "thread-2",
            "Personal note",
            "friend@example.net",
            "A personal note",
            "Personal body",
        ),
    ]);
    let mut permissions = PermissionSet::default();
    permissions.apply(PermissionPreset::TidyUp);
    permissions.per_folder = true;
    permissions.folder_rules.insert(
        "Private".to_owned(),
        FolderRule {
            read: false,
            write: false,
        },
    );
    let engine = PolicyEngine::new([Policy::new(account_id.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    let marked = service
        .mark(&account_id, "m1", MarkChange::Seen)
        .expect("marking is allowed in INBOX");
    assert!(marked.seen());
    assert!(!marked.flagged());

    // The blocked folder refuses the mutation no matter what the tool
    // arguments claim.
    assert!(service.mark(&account_id, "m2", MarkChange::Seen).is_err());
}

#[test]
fn default_permissions_keep_marking_shut() {
    let account_id = AccountId::new("work");
    let provider = FixtureMailProvider::new([StoredMessage::new(
        account_id.clone(),
        "INBOX",
        "m1",
        "thread-1",
        "Quarterly invoice",
        "billing@example.com",
        "The quarterly invoice is attached.",
        "Invoice body",
    )]);
    let engine = PolicyEngine::new([Policy::new(account_id.clone(), PermissionSet::default())]);
    let mut sessions = SearchSessionStore::default();
    let mut service = MailAccessService::new(provider, engine, &mut sessions);

    assert!(service.mark(&account_id, "m1", MarkChange::Seen).is_err());
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
                "INBOX",
                "Invoice April",
                "billing@example.com",
                "April invoice attached",
            ),
            SearchHit::new(
                "m2",
                "INBOX",
                "Team notes",
                "lead@example.com",
                "Budget planning notes",
            ),
            SearchHit::new(
                "m3",
                "INBOX",
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

#[test]
fn attachment_download_needs_the_attachment_read_level() {
    let account = AccountId::new("work");
    let message = StoredMessage::new(
        account.clone(),
        "INBOX",
        "m1",
        "t1",
        "Mit Anhang",
        "a@example.com",
        "s",
        "b",
    )
    .with_attachment_infos(vec![AttachmentInfo::new(
        "2",
        "angebot.pdf",
        "application/pdf",
        4,
        false,
    )]);
    let mut provider = FixtureMailProvider::new([message]).with_attachment(
        "m1",
        "2",
        "angebot.pdf",
        "application/pdf",
        b"%PDF".to_vec(),
    );

    // full_message may list but not download.
    let mut permissions = PermissionSet::default();
    permissions.read = ReadAccess::FullMessage;
    let engine = PolicyEngine::new([Policy::new(account.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(&mut provider, engine, &mut sessions);

    let listed = service.get_message(&account, "m1", false).expect("readable");
    assert_eq!(listed.attachments().len(), 1);
    assert_eq!(listed.attachments()[0].filename(), "angebot.pdf");
    assert!(!service.download_allowed(&account, "INBOX"));
    assert!(service.get_attachment(&account, "m1", "2").is_err());

    // with_attachments downloads.
    let mut permissions = PermissionSet::default();
    permissions.read = ReadAccess::WithAttachments;
    let engine = PolicyEngine::new([Policy::new(account.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(&mut provider, engine, &mut sessions);

    assert!(service.download_allowed(&account, "INBOX"));
    let payload = service.get_attachment(&account, "m1", "2").expect("allowed");
    assert_eq!(payload.mailbox(), "INBOX");
    assert_eq!(payload.filename(), "angebot.pdf");
    assert_eq!(payload.content(), b"%PDF");
}

#[test]
fn a_blocked_folder_blocks_the_download_too() {
    let account = AccountId::new("work");
    let message = StoredMessage::new(
        account.clone(),
        "Geheim",
        "m1",
        "t1",
        "S",
        "a@example.com",
        "s",
        "b",
    );
    let mut provider = FixtureMailProvider::new([message]).with_attachment(
        "m1",
        "1",
        "x.bin",
        "application/octet-stream",
        vec![0],
    );

    let mut permissions = PermissionSet::default();
    permissions.read = ReadAccess::WithAttachments;
    permissions.per_folder = true;
    permissions.folder_rules.insert(
        "Geheim".to_owned(),
        FolderRule {
            read: false,
            write: true,
        },
    );
    let engine = PolicyEngine::new([Policy::new(account.clone(), permissions)]);
    let mut sessions = SearchSessionStore::default();
    let service = MailAccessService::new(&mut provider, engine, &mut sessions);

    assert!(service.get_attachment(&account, "m1", "1").is_err());
}

#[test]
fn an_unknown_attachment_id_is_a_named_error() {
    let account = AccountId::new("work");
    let message = StoredMessage::new(
        account.clone(),
        "INBOX",
        "m1",
        "t1",
        "S",
        "a@example.com",
        "s",
        "b",
    );
    let provider = FixtureMailProvider::new([message]);

    let error = provider
        .get_attachment(&account, "m1", "7")
        .expect_err("nothing seeded");
    assert!(error.to_string().contains('7'));
    assert!(error.to_string().contains("m1"));
}
