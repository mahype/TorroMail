//! Opt-in integration test against an actual, disposable Dovecot server.
//! scripts/test-drafts-dovecot.sh supplies a loopback-only test instance.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use torromail_core::smtp::{SmtpAuth, SmtpClient};
use torromail_core::{
    AccountId, CoreError, CoreResult, ImapClient, ImapMailProvider, ImapTransport, MailProvider,
    StreamImapTransport,
};
use torromail_mcp::LineMcpServer;

struct DropAppendReply {
    inner: StreamImapTransport<TcpStream>,
    drop_once: Arc<AtomicBool>,
    append_tag: Option<String>,
    refusal: Option<String>,
    deny_create: bool,
    deny_append: bool,
}
impl ImapTransport for DropAppendReply {
    fn send_line(&mut self, line: &str) -> CoreResult<()> {
        let command = line.split_whitespace().nth(1);
        if (self.deny_create && command == Some("CREATE"))
            || (self.deny_append && command == Some("APPEND"))
        {
            self.refusal = Some(format!(
                "{} NO [NOPERM] controlled test refusal",
                line.split_whitespace().next().unwrap()
            ));
            return Ok(());
        }
        if command == Some("APPEND") {
            self.append_tag = line.split_whitespace().next().map(str::to_owned);
        }
        self.inner.send_line(line)
    }
    fn read_line(&mut self) -> CoreResult<String> {
        if let Some(refusal) = self.refusal.take() {
            return Ok(refusal);
        }
        let line = self.inner.read_line()?;
        if self
            .append_tag
            .as_ref()
            .is_some_and(|tag| line.starts_with(&format!("{tag} OK")))
        {
            self.append_tag = None;
            if self.drop_once.swap(false, Ordering::SeqCst) {
                return Err(CoreError::ProviderFailure(
                    "test: lost APPEND acknowledgement".into(),
                ));
            }
        }
        Ok(line)
    }
    fn read_bytes(&mut self, count: usize) -> CoreResult<Vec<u8>> {
        self.inner.read_bytes(count)
    }
}

fn client(user: &str, drop_once: Arc<AtomicBool>) -> ImapClient<DropAppendReply> {
    client_faulty(user, drop_once, false, false)
}
fn client_faulty(
    user: &str,
    drop_once: Arc<AtomicBool>,
    deny_create: bool,
    deny_append: bool,
) -> ImapClient<DropAppendReply> {
    let port: u16 = std::env::var("TORROMAIL_TEST_IMAP_PORT")
        .unwrap_or_else(|_| "31143".into())
        .parse()
        .unwrap();
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    ImapClient::connect(
        DropAppendReply {
            inner: StreamImapTransport::new(stream),
            drop_once,
            append_tag: None,
            refusal: None,
            deny_create,
            deny_append,
        },
        user,
        "supersecret",
    )
    .unwrap()
}

fn server(path: &Path, user: &str, fault: Arc<AtomicBool>) -> LineMcpServer {
    let user = user.to_owned();
    LineMcpServer::with_connect_override(path, false, move |id| {
        Ok(Box::new(ImapMailProvider::new(
            id.clone(),
            client(&user, fault.clone()),
        )) as Box<dyn MailProvider>)
    })
}
fn call(server: &LineMcpServer, name: &str, arguments: Value) -> Value {
    serde_json::from_str(&server.handle_line(&json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":name,"arguments":arguments}}).to_string()).unwrap()).unwrap()
}
fn payload(answer: &Value) -> Value {
    assert!(answer.get("error").is_none(), "{answer}");
    serde_json::from_str(answer["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn policy(path: &Path, create: bool, send: bool) {
    std::fs::write(
        path,
        json!({"version":1,"accounts":[{
            "id":"test", "email":"sender@example.invalid", "read":"full_message",
            "write":{"drafts":true}, "send":send, "allow_create_drafts_mailbox":create,
            "imap":{"host":"test.invalid", "username":"test", "secret_ref":"test-only"},
            "smtp":{"host":"test.invalid", "username":"test", "secret_ref":"test-only"}
        }]})
        .to_string(),
    )
    .unwrap();
}

#[path = "support/smtp.rs"]
mod smtp;
use smtp::smtp_sink;

#[test]
#[ignore = "requires disposable Dovecot; run scripts/test-drafts-dovecot.sh"]
fn real_dovecot_creation_ambiguous_retry_concurrency_attachment_and_confirmed_send() {
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let user = format!("draft-{suffix}@example.invalid");
    let directory = std::env::temp_dir().join(format!("torromail-drafts-{suffix}"));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("policy.json");
    let expected_mailbox =
        std::env::var("TORROMAIL_TEST_DRAFTS_MAILBOX").unwrap_or_else(|_| "Drafts".into());
    let sent_mailbox = format!("{}Sent", expected_mailbox.strip_suffix("Drafts").unwrap());
    let fault = Arc::new(AtomicBool::new(false));
    let first = server(&path, &user, fault.clone());
    let arguments = json!({"account_id":"test", "idempotency_key":"unique-draft", "to":["to@example.invalid"], "cc":["cc@example.invalid"], "bcc":["bcc@example.invalid"], "subject":"Draft verification", "body":"Exact body with umlaut: ä\nSecond line.", "attachments":[{"filename":"sample.bin", "content_base64":"AAECA/8=", "media_type":"application/octet-stream"}]});
    policy(&path, false, true);
    let denied = call(&first, "mail_create_draft", arguments.clone());
    assert_eq!(
        denied["error"]["data"]["reason"], "drafts_folder_missing",
        "{denied}"
    );
    assert_eq!(
        client(&user, fault.clone()).selectable_mailboxes().unwrap(),
        vec!["INBOX"]
    );

    policy(&path, true, true);
    fault.store(true, Ordering::SeqCst);
    let uncertain = call(&first, "mail_create_draft", arguments.clone());
    assert_eq!(
        uncertain["error"]["data"]["reason"],
        "draft_storage_uncertain"
    );
    assert!(
        uncertain.get("result").is_none(),
        "no draft id before verification"
    );
    drop(first);
    let second = server(&path, &user, fault.clone());
    let recovered = payload(&call(&second, "mail_create_draft", arguments.clone()));
    assert_eq!(recovered["attachment_count"], 1);
    let mailbox = recovered["mailbox"].as_str().unwrap();
    assert_eq!(mailbox, expected_mailbox);
    let draft_id = recovered["draft_id"].as_str().unwrap();
    let again = payload(&call(&second, "mail_create_draft", arguments.clone()));
    assert_eq!(again["draft_id"], draft_id);
    let mut check = client(&user, fault.clone());
    let hits = check.uid_search(mailbox, "", &Default::default()).unwrap();
    assert_eq!(
        hits.len(),
        1,
        "ambiguous APPEND and retry store only one draft"
    );
    let fetched = check.uid_fetch(mailbox, hits[0]).unwrap();
    assert!(fetched.body.contains("Exact body with umlaut: ä"));
    let provider = ImapMailProvider::new(AccountId::new("test"), check);
    let id = format!("{mailbox}/{}", hits[0]);
    let stored = provider.get_message(&AccountId::new("test"), &id).unwrap();
    let attachment_id = stored.attachments()[0].id();
    let attachment = provider
        .get_attachment(&AccountId::new("test"), &id, attachment_id)
        .unwrap();
    assert_eq!(attachment.content(), &[0, 1, 2, 3, 255]);

    // Independent MCP processes share the journal lock and identity.
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let (path, user, args, fault, barrier) = (
                path.clone(),
                user.clone(),
                arguments.clone(),
                fault.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                let server = server(&path, &user, fault);
                barrier.wait();
                payload(&call(&server, "mail_create_draft", args))
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap()["draft_id"], draft_id);
    }
    let mut changed = arguments.clone();
    changed["subject"] = json!("Changed contents");
    assert_eq!(
        call(&second, "mail_create_draft", changed)["error"]["code"],
        -32602
    );
    policy(&path, true, false);
    assert!(
        call(
            &second,
            "mail_prepare_send",
            json!({"account_id":"test","draft_id":draft_id})
        )
        .get("error")
        .is_some()
    );
    policy(&path, true, true);

    // Test-only setup for the unchanged Sent prerequisite.
    let port = std::env::var("TORROMAIL_TEST_IMAP_PORT").unwrap_or_else(|_| "31143".into());
    let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    let mut raw = BufReader::new(stream);
    let mut line = String::new();
    raw.read_line(&mut line).unwrap();
    for command in [
        format!("s1 LOGIN \"{user}\" supersecret\r\n"),
        format!("s2 CREATE \"{sent_mailbox}\"\r\n"),
    ] {
        raw.get_mut().write_all(command.as_bytes()).unwrap();
        let tag = command.split_whitespace().next().unwrap();
        loop {
            line.clear();
            raw.read_line(&mut line).unwrap();
            if line.starts_with(tag) {
                assert!(line.contains(" OK"), "{line}");
                break;
            }
        }
    }
    let (smtp_port, sink) = smtp_sink();
    let second = second.with_smtp_override(move |from, recipients, mime| {
        let socket = TcpStream::connect(("127.0.0.1", smtp_port)).unwrap();
        let mut smtp = SmtpClient::connect(
            StreamImapTransport::new(socket),
            "example.invalid",
            SmtpAuth::Login {
                username: "test".into(),
                secret: "test".into(),
            },
        )?;
        let result = smtp.send_message(from, recipients, mime);
        smtp.quit();
        result
    });
    let prepared = payload(&call(
        &second,
        "mail_prepare_send",
        json!({"account_id":"test","draft_id":draft_id}),
    ));
    assert_eq!(
        client(&user, fault.clone())
            .uid_search(&sent_mailbox, "", &Default::default())
            .unwrap()
            .len(),
        0,
        "prepare never sends"
    );
    let action = prepared["pending_action_id"].clone();
    assert!(
        call(
            &second,
            "mail_confirm_action",
            json!({"pending_action_id":action,"confirmation_code":"wrong"})
        )
        .get("error")
        .is_some()
    );
    let sent = payload(&call(
        &second,
        "mail_confirm_action",
        json!({"pending_action_id":action,"confirmation_code":prepared["confirmation_code"]}),
    ));
    assert_eq!(sent["status"], "sent");
    assert_eq!(sent["sent_copy_status"], "saved");
    let (recipients, mime) = sink.join().unwrap();
    assert_eq!(
        recipients,
        vec![
            "RCPT TO:<to@example.invalid>",
            "RCPT TO:<cc@example.invalid>",
            "RCPT TO:<bcc@example.invalid>"
        ]
    );
    assert!(mime.contains("To: to@example.invalid\r\nCc: cc@example.invalid\r\n"));
    assert!(!mime.contains("Bcc:"));
    assert!(mime.contains("AAECA/8="));
    let repeated = payload(&call(
        &second,
        "mail_confirm_action",
        json!({"pending_action_id":action,"confirmation_code":prepared["confirmation_code"]}),
    ));
    assert_eq!(
        repeated["submission_status"], "accepted",
        "confirmation reads the durable outcome without sending again"
    );

    let mut without_key = arguments.clone();
    without_key
        .as_object_mut()
        .unwrap()
        .remove("idempotency_key");
    let default_draft = payload(&call(&second, "mail_create_draft", without_key.clone()));
    assert_eq!(
        payload(&call(&second, "mail_create_draft", without_key.clone()))["draft_id"],
        default_draft["draft_id"]
    );
    without_key["idempotency_key"] = json!("intentional-second-identical-draft");
    assert_ne!(
        payload(&call(&second, "mail_create_draft", without_key))["draft_id"],
        default_draft["draft_id"]
    );
    assert_eq!(
        client(&user, fault.clone())
            .uid_search(mailbox, "", &Default::default())
            .unwrap()
            .len(),
        3
    );

    // Key renewal preserves the stable client id and the operation identity.
    let rotate_user = format!("rotate-{suffix}@example.invalid");
    let rotate_path = directory.join("rotate-policy.json");
    policy(&rotate_path, true, true);
    let mut rotate_policy: Value =
        serde_json::from_str(&std::fs::read_to_string(&rotate_path).unwrap()).unwrap();
    let hash = |key: &str| {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(key.as_bytes()))
    };
    rotate_policy["clients"] = json!([{"id":"stable-client", "token_sha256":hash("first-key")}]);
    std::fs::write(&rotate_path, rotate_policy.to_string()).unwrap();
    let rotate_server =
        server(&rotate_path, &rotate_user, fault.clone()).with_presented_token(Some("first-key"));
    let rotated_draft = payload(&call(
        &rotate_server,
        "mail_create_draft",
        arguments.clone(),
    ));
    drop(rotate_server);
    rotate_policy["clients"][0]["token_sha256"] = json!(hash("renewed-key"));
    std::fs::write(&rotate_path, rotate_policy.to_string()).unwrap();
    let rotate_server =
        server(&rotate_path, &rotate_user, fault.clone()).with_presented_token(Some("renewed-key"));
    assert_eq!(
        payload(&call(
            &rotate_server,
            "mail_create_draft",
            arguments.clone()
        ))["draft_id"],
        rotated_draft["draft_id"]
    );
    assert_eq!(
        client(&rotate_user, fault.clone())
            .uid_search(mailbox, "", &Default::default())
            .unwrap()
            .len(),
        1
    );

    // Real concurrent CREATE attempts, without the MCP-side lock.
    let race_user = format!("race-{suffix}@example.invalid");
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let user = race_user.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut client = client(&user, Arc::new(AtomicBool::new(false)));
                barrier.wait();
                client.ensure_drafts_mailbox(None, true).unwrap()
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), expected_mailbox);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

/// All copies use real Dovecot. Refusals and lost acknowledgement are injected
/// at the protocol boundary; SMTP is a real loopback socket with no delivery.
#[test]
#[ignore = "requires disposable Dovecot; run scripts/test-drafts-dovecot.sh"]
fn real_sent_copy_creation_refusal_append_failure_restart_and_reconciliation() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let expected = std::env::var("TORROMAIL_TEST_DRAFTS_MAILBOX")
        .unwrap_or_else(|_| "Drafts".into())
        .replace("Drafts", "Sent");
    for scenario in [
        "missing",
        "creation-denied",
        "append-refused",
        "append-ack-lost",
        "auto-create",
        "bad-mapping",
    ] {
        let user = format!("sent-{suffix}-{scenario}@example.invalid");
        let directory = std::env::temp_dir().join(format!("torromail-sent-{suffix}-{scenario}"));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("policy.json");
        policy(&path, false, true);
        let mut document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        document["accounts"][0]["allow_create_sent_mailbox"] = json!(matches!(
            scenario,
            "creation-denied" | "auto-create" | "bad-mapping"
        ));
        if scenario == "bad-mapping" {
            document["accounts"][0]["mailbox_overrides"] = json!({"sent":"broken-manual-choice"});
        }
        std::fs::write(&path, document.to_string()).unwrap();
        if matches!(
            scenario,
            "append-refused" | "append-ack-lost" | "bad-mapping"
        ) {
            client(&user, Arc::new(AtomicBool::new(false)))
                .ensure_special_mailbox(torromail_core::SpecialMailboxRole::Sent, None, true)
                .unwrap();
        }
        let fault = Arc::new(AtomicBool::new(scenario == "append-ack-lost"));
        let user_in = user.clone();
        let fault_in = fault.clone();
        let first = LineMcpServer::with_connect_override(&path, false, move |id| {
            Ok(Box::new(ImapMailProvider::new(
                id.clone(),
                client_faulty(
                    &user_in,
                    fault_in.clone(),
                    scenario == "creation-denied",
                    scenario == "append-refused",
                ),
            )))
        });
        let (port, sink) = smtp_sink();
        let first = first.with_submission_override(move |from, to, mime| {
            use torromail_core::smtp::SubmissionError;
            let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let mut smtp = SmtpClient::connect(
                StreamImapTransport::new(socket),
                "example.invalid",
                SmtpAuth::Login {
                    username: "test".into(),
                    secret: "test".into(),
                },
            )
            .map_err(SubmissionError::NotAccepted)?;
            let result = smtp.submit_message(from, to, mime);
            smtp.quit();
            result
        });
        let draft = payload(&call(
            &first,
            "mail_create_draft",
            json!({"account_id":"test","storage":"local","idempotency_key":"once","to":["to@example.invalid"],"subject":"Sent copy matrix","body":"MIME body ä\n.line","attachments":[{"filename":"test.bin","content_base64":"AAEC/w=="}]}),
        ));
        let action = payload(&call(
            &first,
            "mail_prepare_send",
            json!({"account_id":"test","draft_id":draft["draft_id"]}),
        ));
        let confirm = json!({"pending_action_id":action["pending_action_id"],"confirmation_code":action["confirmation_code"]});
        let submitted = payload(&call(&first, "mail_confirm_action", confirm.clone()));
        assert_eq!(
            submitted["submission_status"], "accepted",
            "{scenario}: {submitted}"
        );
        assert_eq!(
            submitted["sent_copy_status"],
            if scenario == "auto-create" {
                "saved"
            } else {
                "pending"
            },
            "{scenario}: {submitted}"
        );
        let (_, mime) = sink.join().unwrap();
        drop(first);
        // Supply folder creation consent now, or repair a broken manual mapping.
        if scenario == "bad-mapping" {
            document["accounts"][0]
                .as_object_mut()
                .unwrap()
                .remove("mailbox_overrides");
        }
        document["accounts"][0]["allow_create_sent_mailbox"] = json!(true);
        std::fs::write(&path, document.to_string()).unwrap();
        let second = server(&path, &user, Arc::new(AtomicBool::new(false)))
            .with_submission_override(|_, _, _| panic!("copy recovery must not call SMTP"));
        second.retry_sent_copies().unwrap();
        let recovered = payload(&call(&second, "mail_confirm_action", confirm));
        assert_eq!(recovered["submission_status"], "accepted");
        assert_eq!(
            recovered["sent_copy_status"], "saved",
            "{scenario}: {recovered}"
        );
        assert_eq!(recovered["sent_copy_mailbox"], expected);
        let mut check = client(&user, Arc::new(AtomicBool::new(false)));
        let hits = check
            .uid_search(&expected, "", &Default::default())
            .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "{scenario}: exactly one Sent copy after retries"
        );
        let marker = mime
            .lines()
            .find_map(|line| line.strip_prefix("Message-ID: "))
            .unwrap();
        check
            .append_sent_verified(&expected, &mime, marker, false)
            .unwrap();
        // The exact-MIME verification also verifies attachment bytes and Seen.
        assert_eq!(
            check
                .uid_search(&expected, "", &Default::default())
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
