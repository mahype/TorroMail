//! The IMAP protocol core against a scripted server: canned responses in,
//! sent commands recorded — no network anywhere.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use torromail_core::{
    AccountId, CoreResult, ImapClient, ImapMailProvider, ImapTransport, MailProvider, MarkChange,
};

#[derive(Debug)]
enum Incoming {
    Line(String),
    Bytes(Vec<u8>),
}

fn line(text: &str) -> Incoming {
    Incoming::Line(text.to_owned())
}

/// Shared view on everything the client sent, surviving the move of the
/// transport into the client.
#[derive(Clone, Default)]
struct SentLog(Rc<RefCell<Vec<String>>>);

impl SentLog {
    fn lines(&self) -> Vec<String> {
        self.0.borrow().clone()
    }
}

struct ScriptedTransport {
    incoming: VecDeque<Incoming>,
    log: SentLog,
}

impl ScriptedTransport {
    fn new(script: Vec<Incoming>, log: SentLog) -> Self {
        Self {
            incoming: script.into(),
            log,
        }
    }
}

impl ImapTransport for ScriptedTransport {
    fn send_line(&mut self, sent: &str) -> CoreResult<()> {
        self.log.0.borrow_mut().push(sent.to_owned());
        Ok(())
    }

    fn read_line(&mut self) -> CoreResult<String> {
        match self.incoming.pop_front() {
            Some(Incoming::Line(text)) => Ok(text),
            other => panic!("script expected a line, got {other:?}"),
        }
    }

    fn read_bytes(&mut self, count: usize) -> CoreResult<Vec<u8>> {
        match self.incoming.pop_front() {
            Some(Incoming::Bytes(bytes)) => {
                assert_eq!(bytes.len(), count, "literal length announced vs scripted");
                Ok(bytes)
            }
            other => panic!("script expected {count} literal bytes, got {other:?}"),
        }
    }
}

fn login_script() -> Vec<Incoming> {
    vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 OK LOGIN completed"),
    ]
}

fn fetch_script(tag: &str, uid: u32, flags: &str, headers: &str, body: &str) -> Vec<Incoming> {
    vec![
        line(&format!(
            "* 1 FETCH (UID {uid} FLAGS ({flags}) BODY[HEADER.FIELDS (SUBJECT FROM)] {{{}}}",
            headers.len()
        )),
        Incoming::Bytes(headers.as_bytes().to_vec()),
        line(&format!(" BODY[TEXT] {{{}}}", body.len())),
        Incoming::Bytes(body.as_bytes().to_vec()),
        line(")"),
        line(&format!("{tag} OK FETCH completed")),
    ]
}

const HEADERS: &str = "Subject: Quarterly invoice\r\nFrom: billing@example.com\r\n\r\n";
const BODY: &str = "Invoice body";

#[test]
fn login_lists_mailboxes_and_quotes_credentials() {
    let mut script = login_script();
    script.extend([
        line("* LIST (\\HasNoChildren) \"/\" \"INBOX\""),
        line("* LIST (\\HasNoChildren) \"/\" \"Archive\""),
        line("t2 OK LIST completed"),
    ]);
    let log = SentLog::default();
    let transport = ScriptedTransport::new(script, log.clone());

    let mut client =
        ImapClient::connect(transport, "work@example.com", "app-secret").expect("login succeeds");
    let mailboxes = client.list_mailboxes().expect("list succeeds");

    assert_eq!(mailboxes, ["INBOX", "Archive"]);
    assert_eq!(
        log.lines(),
        [
            "t1 LOGIN \"work@example.com\" \"app-secret\"",
            "t2 LIST \"\" \"*\""
        ]
    );
}

#[test]
fn a_refused_login_surfaces_the_server_answer() {
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO LOGIN failed"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "wrong",
    );

    let error = match result {
        Ok(_) => panic!("login must fail"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("LOGIN failed"));
}

#[test]
fn searching_selects_searches_and_fetches_hits() {
    let mut script = login_script();
    script.extend([
        line("* 1 EXISTS"),
        line("t2 OK SELECT completed"),
        line("* SEARCH 101"),
        line("t3 OK SEARCH completed"),
    ]);
    script.extend(fetch_script("t4", 101, "", HEADERS, BODY));
    let log = SentLog::default();
    let client = ImapClient::connect(
        ScriptedTransport::new(script, log.clone()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let account_id = AccountId::new("work");
    let provider = ImapMailProvider::new(account_id.clone(), client);

    let hits = provider
        .search(&account_id, "invoice", Some("INBOX"), 10)
        .expect("search succeeds");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id(), "INBOX/101");
    assert_eq!(hits[0].mailbox(), "INBOX");
    assert_eq!(hits[0].subject(), "Quarterly invoice");
    let sent = log.lines();
    assert_eq!(sent[1], "t2 SELECT \"INBOX\"");
    assert_eq!(sent[2], "t3 UID SEARCH TEXT \"invoice\"");
    assert!(sent[3].starts_with("t4 UID FETCH 101 "));
}

#[test]
fn fetched_messages_carry_flags_and_body() {
    let mut script = login_script();
    script.extend([line("* 1 EXISTS"), line("t2 OK SELECT completed")]);
    script.extend(fetch_script("t3", 101, "\\Seen", HEADERS, BODY));
    let client = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let account_id = AccountId::new("work");
    let provider = ImapMailProvider::new(account_id.clone(), client);

    let message = provider
        .get_message(&account_id, "INBOX/101")
        .expect("fetch succeeds");

    assert_eq!(message.mailbox(), "INBOX");
    assert_eq!(message.subject(), "Quarterly invoice");
    assert_eq!(message.sender(), "billing@example.com");
    assert_eq!(message.body(), "Invoice body");
    assert!(message.seen());
    assert!(!message.flagged());
}

#[test]
fn marking_sends_a_uid_store_and_reuses_the_selection() {
    let mut script = login_script();
    script.extend([
        line("* 1 EXISTS"),
        line("t2 OK SELECT completed"),
        line("t3 OK STORE completed"),
        // The mailbox is already selected — the second store goes straight
        // through without another SELECT.
        line("t4 OK STORE completed"),
    ]);
    let log = SentLog::default();
    let client = ImapClient::connect(
        ScriptedTransport::new(script, log.clone()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let account_id = AccountId::new("work");
    let mut provider = ImapMailProvider::new(account_id.clone(), client);

    provider
        .mark(&account_id, "INBOX/101", MarkChange::Seen)
        .expect("marking succeeds");
    provider
        .mark(&account_id, "INBOX/101", MarkChange::Flagged)
        .expect("marking succeeds");

    let sent = log.lines();
    assert_eq!(sent[2], "t3 UID STORE 101 +FLAGS (\\Seen)");
    assert_eq!(sent[3], "t4 UID STORE 101 +FLAGS (\\Flagged)");
}

#[test]
fn the_provider_answers_only_for_its_own_account() {
    let client = ImapClient::connect(
        ScriptedTransport::new(login_script(), SentLog::default()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let provider = ImapMailProvider::new(AccountId::new("work"), client);

    assert!(
        provider
            .search(&AccountId::new("other"), "invoice", None, 10)
            .is_err()
    );
}
