//! Opt-in integration test against an actual, disposable Dovecot server.
//! scripts/test-drafts-dovecot.sh supplies a loopback-only test instance.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
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
}
impl ImapTransport for DropAppendReply {
    fn send_line(&mut self, line: &str) -> CoreResult<()> {
        if line.split_whitespace().nth(1) == Some("APPEND") {
            self.append_tag = line.split_whitespace().next().map(str::to_owned);
        }
        self.inner.send_line(line)
    }
    fn read_line(&mut self) -> CoreResult<String> {
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

// Local SMTP sink: real SMTP protocol, no delivery or upstream route. Retain
// only in test memory to compare the envelope and MIME after confirmation.
fn smtp_sink() -> (u16, std::thread::JoinHandle<(Vec<String>, String)>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        socket.write_all(b"220 local test sink\r\n").unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut recipients = Vec::new();
        let mut mime = String::new();
        let mut auth_step = 0;
        let mut data = false;
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if data && line != ".\r\n" {
                mime.push_str(line.strip_prefix('.').unwrap_or(&line));
                continue;
            }
            let reply = if data {
                data = false;
                "250 stored\r\n"
            } else if line.starts_with("EHLO") {
                "250-local\r\n250 AUTH LOGIN\r\n"
            } else if line.starts_with("AUTH LOGIN") {
                auth_step = 1;
                "334 VXNlcm5hbWU6\r\n"
            } else if auth_step == 1 {
                auth_step = 2;
                "334 UGFzc3dvcmQ6\r\n"
            } else if auth_step == 2 {
                auth_step = 0;
                "235 authenticated\r\n"
            } else if line.starts_with("RCPT TO:") {
                recipients.push(line.trim().to_owned());
                "250 recipient\r\n"
            } else if line.starts_with("DATA") {
                data = true;
                "354 continue\r\n"
            } else if line.starts_with("QUIT") {
                socket.write_all(b"221 bye\r\n").unwrap();
                break;
            } else {
                "250 ok\r\n"
            };
            socket.write_all(reply.as_bytes()).unwrap();
        }
        (recipients, mime)
    });
    (port, handle)
}

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
    assert!(
        call(
            &second,
            "mail_confirm_action",
            json!({"pending_action_id":action,"confirmation_code":prepared["confirmation_code"]})
        )
        .get("error")
        .is_some(),
        "confirmation executes once"
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
