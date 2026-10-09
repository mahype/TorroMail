//! Durable submission contract with a loopback SMTP server. No user secrets.
#[path = "support/smtp.rs"]
mod smtp;
use serde_json::{Value, json};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use torromail_core::smtp::{SmtpAuth, SmtpClient, SubmissionError};
use torromail_core::{CoreError, FixtureMailProvider, StreamImapTransport};
use torromail_mcp::LineMcpServer;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "torromail-send-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn policy(&self, strategy: &str, send: bool) -> PathBuf {
        let path = self.0.join("policy.json");
        std::fs::write(&path,json!({"version":1,"accounts":[{"id":"test","email":"from@example.invalid","read":"full_message","write":{"drafts":true},"send":send,
            "sent_copy_strategy":strategy,"smtp":{"host":"test.invalid","username":"test","secret_ref":"unused"}}]}).to_string()).unwrap();
        path
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn call(server: &LineMcpServer, name: &str, arguments: Value) -> Value {
    serde_json::from_str(&server.handle_line(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}).to_string()).unwrap()).unwrap()
}
fn payload(answer: Value) -> Value {
    assert!(answer.get("error").is_none(), "{answer}");
    serde_json::from_str(answer["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn draft(server: &LineMcpServer) -> Value {
    payload(call(
        server,
        "mail_create_draft",
        json!({"account_id":"test","storage":"local","to":["to@example.invalid"],"cc":["cc@example.invalid"],"bcc":["bcc@example.invalid"],"subject":"Submission contract","body":"Exact MIME\n.line\n","attachments":[{"filename":"test.bin","content_base64":"AAEC/w=="}],"idempotency_key":"once"}),
    ))
}
fn prepare(server: &LineMcpServer, draft: &Value) -> Value {
    payload(call(
        server,
        "mail_prepare_send",
        json!({"account_id":"test","draft_id":draft["draft_id"]}),
    ))
}
fn confirm(server: &LineMcpServer, action: &Value) -> Value {
    call(
        server,
        "mail_confirm_action",
        json!({"pending_action_id":action["pending_action_id"],"confirmation_code":action["confirmation_code"]}),
    )
}
fn transport(
    port: u16,
    from: &str,
    recipients: &[String],
    mime: &str,
) -> Result<(), SubmissionError> {
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut smtp = SmtpClient::connect(
        StreamImapTransport::new(stream),
        "example.invalid",
        SmtpAuth::Login {
            username: "test".into(),
            secret: "test".into(),
        },
    )
    .map_err(SubmissionError::NotAccepted)?;
    let outcome = smtp.submit_message(from, recipients, mime);
    smtp.quit();
    outcome
}
fn unreachable(path: &Path) -> LineMcpServer {
    LineMcpServer::with_connect_override(path, false, |_| {
        Err(CoreError::ProviderFailure("controlled IMAP outage".into()))
    })
}
fn stored(path: &Path, id: &str) -> PathBuf {
    let root = path.parent().unwrap().join("send-state");
    let directory = std::fs::read_dir(root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    directory.join(format!("{id}.json"))
}
#[test]
fn smtp_accepted_imap_unreachable_restart_only_retries_copy() {
    let directory = Directory::new();
    let path = directory.policy("imap", true);
    let count = Arc::new(AtomicUsize::new(0));
    let count_in = count.clone();
    let (port, sink) = smtp::smtp_sink();
    let first = unreachable(&path).with_submission_override(move |from, to, mime| {
        count_in.fetch_add(1, Ordering::SeqCst);
        transport(port, from, to, mime)
    });
    let draft = draft(&first);
    let action = prepare(&first, &draft);
    let result = payload(confirm(&first, &action));
    assert_eq!(result["submission_status"], "accepted");
    assert_eq!(result["sent_copy_status"], "pending");
    let (_, mime) = sink.join().unwrap();
    let disk: Value = serde_json::from_slice(
        &std::fs::read(stored(&path, draft["draft_id"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(
        disk["record"]["raw"], mime,
        "SMTP and private spool contain exactly the same MIME"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = stored(&path, draft["draft_id"].as_str().unwrap());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    drop(first);
    let recovered = LineMcpServer::with_connect_override(&path, false, |_| {
        Ok(Box::new(FixtureMailProvider::new([])))
    })
    .with_submission_override(|_, _, _| panic!("SMTP must never run on recovery"));
    assert_eq!(recovered.retry_sent_copies().unwrap(), 1);
    let again = payload(confirm(&recovered, &action));
    assert_eq!(again["submission_status"], "accepted");
    assert_eq!(again["sent_copy_status"], "saved");
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[test]
fn unknown_final_smtp_reply_is_durable_and_never_resubmitted() {
    let directory = Directory::new();
    let path = directory.policy("imap", true);
    let (port, sink) = smtp::sink(smtp::Mode::DropFinal);
    let first = unreachable(&path)
        .with_submission_override(move |from, to, mime| transport(port, from, to, mime));
    let draft = draft(&first);
    let action = prepare(&first, &draft);
    assert_eq!(
        payload(confirm(&first, &action))["submission_status"],
        "unknown"
    );
    sink.join().unwrap();
    drop(first);
    let second = unreachable(&path)
        .with_submission_override(|_, _, _| panic!("unknown must not retry SMTP"));
    assert_eq!(second.retry_sent_copies().unwrap(), 0);
    assert_eq!(
        payload(confirm(&second, &action))["submission_status"],
        "unknown"
    );
    assert_eq!(
        prepare(&second, &draft)["pending_action_id"],
        action["pending_action_id"]
    );
}
#[test]
fn explicit_smtp_rejection_is_not_acceptance() {
    for mode in [smtp::Mode::RejectRecipient, smtp::Mode::RejectData] {
        let directory = Directory::new();
        let path = directory.policy("imap", true);
        let (port, sink) = smtp::sink(mode);
        let server = unreachable(&path)
            .with_submission_override(move |from, to, mime| transport(port, from, to, mime));
        let draft = draft(&server);
        let action = prepare(&server, &draft);
        assert_eq!(
            payload(confirm(&server, &action))["submission_status"],
            "not_accepted"
        );
        sink.join().unwrap();
        assert_eq!(server.retry_sent_copies().unwrap(), 0);
        assert_eq!(
            payload(confirm(&server, &action))["submission_status"],
            "not_accepted"
        );
    }
}
#[test]
fn provider_and_no_copy_never_connect_imap() {
    for (strategy, status) in [("provider", "provider"), ("none", "skipped")] {
        let directory = Directory::new();
        let path = directory.policy(strategy, true);
        let (port, sink) = smtp::smtp_sink();
        let server = LineMcpServer::with_connect_override(&path, false, |_| {
            panic!("copy strategy must not open IMAP")
        })
        .with_submission_override(move |from, to, mime| transport(port, from, to, mime));
        let draft = draft(&server);
        let action = prepare(&server, &draft);
        let result = payload(confirm(&server, &action));
        assert_eq!(result["submission_status"], "accepted");
        assert_eq!(result["sent_copy_status"], status);
        sink.join().unwrap();
        assert_eq!(server.retry_sent_copies().unwrap(), 0);
    }
}
#[test]
fn two_process_instances_confirming_one_operation_submit_once() {
    let directory = Directory::new();
    let path = directory.policy("provider", true);
    let server = unreachable(&path);
    let draft = draft(&server);
    let action = prepare(&server, &draft);
    drop(server);
    let (port, sink) = smtp::smtp_sink();
    let attempts = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let action = action.clone();
            let barrier = barrier.clone();
            let attempts = attempts.clone();
            std::thread::spawn(move || {
                let server = unreachable(&path).with_submission_override(move |from, to, mime| {
                    assert_eq!(attempts.fetch_add(1, Ordering::SeqCst), 0);
                    transport(port, from, to, mime)
                });
                barrier.wait();
                payload(confirm(&server, &action))
            })
        })
        .collect();
    for worker in workers {
        assert_eq!(worker.join().unwrap()["submission_status"], "accepted");
    }
    sink.join().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}
#[test]
fn changed_approved_message_and_revoked_send_are_blocked() {
    let directory = Directory::new();
    let path = directory.policy("imap", true);
    let server = unreachable(&path).with_submission_override(|_, _, _| panic!("blocked send"));
    let draft = draft(&server);
    let action = prepare(&server, &draft);
    let record_path = stored(&path, draft["draft_id"].as_str().unwrap());
    let bytes = std::fs::read(&record_path).unwrap();
    let mut record: Value = serde_json::from_slice(&bytes).unwrap();
    record["record"]["raw"] = json!("changed MIME");
    std::fs::write(&record_path, record.to_string()).unwrap();
    assert!(confirm(&server, &action).get("error").is_some());
    std::fs::write(&record_path, bytes).unwrap();
    directory.policy("imap", false);
    assert!(confirm(&server, &action).get("error").is_some());
}
#[test]
fn crashed_process_after_acceptance_leaves_unknown_and_never_sends_again() {
    let directory = Directory::new();
    let path = directory.policy("provider", true);
    let first = unreachable(&path);
    let draft = draft(&first);
    let action = prepare(&first, &draft);
    let (port, sink) = smtp::smtp_sink();
    let exit = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("TORROMAIL_CRASH_TEST_POLICY", &path)
        .env("TORROMAIL_CRASH_TEST_ACTION", action.to_string())
        .env("TORROMAIL_CRASH_TEST_SMTP_PORT", port.to_string())
        .status()
        .unwrap();
    assert_eq!(exit.code(), Some(23));
    sink.join().unwrap();
    let second =
        unreachable(&path).with_submission_override(|_, _, _| panic!("crash must not resend"));
    assert_eq!(second.retry_sent_copies().unwrap(), 0);
    assert_eq!(
        payload(confirm(&second, &action))["submission_status"],
        "unknown"
    );
}
#[test]
fn crash_worker() {
    let Ok(path) = std::env::var("TORROMAIL_CRASH_TEST_POLICY") else {
        return;
    };
    let action: Value =
        serde_json::from_str(&std::env::var("TORROMAIL_CRASH_TEST_ACTION").unwrap()).unwrap();
    let port = std::env::var("TORROMAIL_CRASH_TEST_SMTP_PORT")
        .unwrap()
        .parse()
        .unwrap();
    let committed = std::env::var_os("TORROMAIL_CRASH_TEST_DURING_COPY").is_some();
    let server = if committed {
        LineMcpServer::with_connect_override(&path, false, |_| std::process::exit(24))
    } else {
        unreachable(Path::new(&path))
    };
    let server = server.with_submission_override(move |from, to, mime| {
        transport(port, from, to, mime)?;
        if committed {
            Ok(())
        } else {
            std::process::exit(23)
        }
    });
    let _ = confirm(&server, &action);
    panic!("worker should exit after SMTP acknowledgement");
}

#[test]
fn gui_bridge_shares_submission_and_rechecks_originating_client_grants() {
    use sha2::{Digest, Sha256};
    let digest = |key: &str| format!("{:x}", Sha256::digest(key.as_bytes()));
    let directory = Directory::new();
    let path = directory.policy("provider", true);
    let mut document: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    document["clients"] = json!([
        {"id":"origin","token_sha256":digest("origin-key"),"account_access":{"mode":"all"}},
        {"id":"torromail-app","token_sha256":digest("app-key"),"account_access":{"mode":"all"}}
    ]);
    std::fs::write(&path, document.to_string()).unwrap();
    let origin = unreachable(&path).with_presented_token(Some("origin-key"));
    let draft = draft(&origin);
    let action = prepare(&origin, &draft);
    let app = unreachable(&path)
        .with_presented_token(Some("app-key"))
        .with_submission_override(|_, _, _| {
            panic!("origin's revoked grant must block app submission")
        });
    let listed = app.send_action_request(&json!({"action":"list"})).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], action["pending_action_id"]);
    assert!(!listed.to_string().contains("AAEC/w=="));
    let request = json!({"action":"confirm","id":action["pending_action_id"],"code":action["confirmation_code"]});
    document["clients"][0]["account_access"] = json!({"mode":"selected","account_ids":[]});
    std::fs::write(&path, document.to_string()).unwrap();
    assert!(app.send_action_request(&request).is_err());
    document["clients"][0]["account_access"] = json!({"mode":"all"});
    std::fs::write(&path, document.to_string()).unwrap();
    let (port, sink) = smtp::smtp_sink();
    let app = unreachable(&path)
        .with_presented_token(Some("app-key"))
        .with_submission_override(move |from, to, mime| transport(port, from, to, mime));
    assert_eq!(
        app.send_action_request(&request).unwrap()["submission_status"],
        "accepted"
    );
    sink.join().unwrap();
    assert_eq!(
        payload(confirm(&origin, &action))["submission_status"],
        "accepted"
    );
}

#[test]
fn process_crash_after_committed_acceptance_recovers_copy_without_smtp() {
    let directory = Directory::new();
    let path = directory.policy("imap", true);
    let first = unreachable(&path);
    let draft = draft(&first);
    let action = prepare(&first, &draft);
    let (port, sink) = smtp::smtp_sink();
    let exit = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("TORROMAIL_CRASH_TEST_POLICY", &path)
        .env("TORROMAIL_CRASH_TEST_ACTION", action.to_string())
        .env("TORROMAIL_CRASH_TEST_SMTP_PORT", port.to_string())
        .env("TORROMAIL_CRASH_TEST_DURING_COPY", "1")
        .status()
        .unwrap();
    assert_eq!(exit.code(), Some(24));
    sink.join().unwrap();
    let recovered = LineMcpServer::with_connect_override(&path, false, |_| {
        Ok(Box::new(FixtureMailProvider::new([])))
    })
    .with_submission_override(|_, _, _| panic!("committed acceptance must never repeat SMTP"));
    assert_eq!(recovered.retry_sent_copies().unwrap(), 1);
    assert_eq!(
        payload(confirm(&recovered, &action))["submission_status"],
        "accepted"
    );
}
