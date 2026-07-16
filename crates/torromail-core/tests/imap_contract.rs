//! The IMAP protocol core against a scripted server: canned responses in,
//! sent commands recorded — no network anywhere.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Cursor, Read, Write};
use std::rc::Rc;

use torromail_core::{
    AccountId, CoreResult, ImapAuth, ImapClient, ImapMailProvider, ImapTransport, MailProvider,
    MarkChange, SearchHit, SearchWindow, StreamImapTransport,
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
        .search(&account_id, "invoice", Some("INBOX"), 10, &SearchWindow::default())
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
fn search_fetches_headers_only_never_the_body() {
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

    provider
        .search(&account_id, "invoice", Some("INBOX"), 10, &SearchWindow::default())
        .expect("search succeeds");

    // The hit's FETCH asks for headers and nothing else — no body pulled for
    // a snippet the search never returns.
    let fetch = log
        .lines()
        .into_iter()
        .find(|command| command.contains("UID FETCH"))
        .expect("a hit is fetched");
    assert!(fetch.contains("HEADER.FIELDS"), "got: {fetch}");
    assert!(!fetch.contains("BODY[TEXT]"), "got: {fetch}");
    assert!(!fetch.contains("BODY.PEEK[TEXT]"), "got: {fetch}");
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

/// The real shape of promotional mail, in miniature: a multipart/alternative
/// with a quoted-printable text part and an HTML part that should lose. The
/// whole fetch→decode path has to hand back readable words, not MIME.
const MULTIPART_HEADERS: &str = concat!(
    "Subject: Angebot\r\n",
    "From: info@mail.clark.de\r\n",
    "Content-Type: multipart/alternative; boundary=\"Xes\"\r\n",
    "\r\n",
);
const MULTIPART_BODY: &str = concat!(
    "--Xes\r\n",
    "Content-Type: text/plain; charset=utf-8\r\n",
    "Content-Transfer-Encoding: quoted-printable\r\n",
    "\r\n",
    "Altersvorsorge f=C3=BCr dich =E2=80=93 jetzt=\r\n",
    " clever vorsorgen\r\n",
    "--Xes\r\n",
    "Content-Type: text/html; charset=utf-8\r\n",
    "\r\n",
    "<html><body><p>Diese HTML-Fassung soll verlieren</p></body></html>\r\n",
    "--Xes--\r\n",
);

#[test]
fn a_multipart_message_is_delivered_as_readable_text() {
    let mut script = login_script();
    script.extend([line("* 1 EXISTS"), line("t2 OK SELECT completed")]);
    script.extend(fetch_script("t3", 101, "", MULTIPART_HEADERS, MULTIPART_BODY));
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

    // The plain part wins, its quoted-printable and soft line break undone;
    // the HTML fallback never surfaces.
    assert_eq!(
        message.body(),
        "Altersvorsorge für dich – jetzt clever vorsorgen"
    );
    assert!(!message.body().contains("HTML-Fassung"));
}

#[test]
fn a_thread_is_reconstructed_from_the_reference_headers() {
    let mut script = login_script();
    // get_thread(INBOX/10): select, fetch the reference headers of 10.
    script.extend([line("* 1 EXISTS"), line("t2 OK SELECT completed")]);
    let refs = "Message-ID: <root@example.com>\r\n\r\n";
    script.extend([
        line(&format!(
            "* 1 FETCH (BODY[HEADER.FIELDS (MESSAGE-ID REFERENCES IN-REPLY-TO)] {{{}}}",
            refs.len()
        )),
        Incoming::Bytes(refs.as_bytes().to_vec()),
        line(")"),
        line("t3 OK FETCH completed"),
    ]);
    // The one id — <root@example.com> — searched two ways: the root itself,
    // then whatever references it.
    script.extend([
        line("* SEARCH 10"),
        line("t4 OK SEARCH completed"),
        line("* SEARCH 11"),
        line("t5 OK SEARCH completed"),
    ]);
    // Then each message is fetched in full, oldest uid first.
    let root_headers = "Subject: Root\r\nFrom: a@example.com\r\n\r\n";
    let reply_headers = "Subject: Re: Root\r\nFrom: b@example.com\r\n\r\n";
    script.extend(fetch_script("t6", 10, "", root_headers, "root body"));
    script.extend(fetch_script("t7", 11, "", reply_headers, "reply body"));

    let log = SentLog::default();
    let client = ImapClient::connect(
        ScriptedTransport::new(script, log.clone()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let account_id = AccountId::new("work");
    let provider = ImapMailProvider::new(account_id.clone(), client);

    let thread = provider
        .get_thread(&account_id, "INBOX/10")
        .expect("thread assembles");

    assert_eq!(thread.len(), 2);
    assert_eq!(thread[0].subject(), "Root");
    assert_eq!(thread[1].subject(), "Re: Root");

    let sent = log.lines();
    assert_eq!(sent[3], "t4 UID SEARCH HEADER \"Message-ID\" \"root@example.com\"");
    assert_eq!(sent[4], "t5 UID SEARCH HEADER \"References\" \"root@example.com\"");
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

/// In-memory duplex stream: canned input, shared view on the output.
struct DuplexStream {
    input: Cursor<Vec<u8>>,
    output: Rc<RefCell<Vec<u8>>>,
}

impl Read for DuplexStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(buffer)
    }
}

impl Write for DuplexStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.output.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_stream_transport_strips_crlf_frames_literals_and_appends_crlf() {
    let written = Rc::new(RefCell::new(Vec::new()));
    let stream = DuplexStream {
        input: Cursor::new(b"* OK ready\r\nabcde rest\r\n".to_vec()),
        output: written.clone(),
    };
    let mut transport = StreamImapTransport::new(stream);

    assert_eq!(transport.read_line().expect("line arrives"), "* OK ready");
    assert_eq!(transport.read_bytes(5).expect("literal arrives"), b"abcde");
    assert_eq!(transport.read_line().expect("rest arrives"), " rest");
    assert!(
        transport.read_line().is_err(),
        "a closed stream is an error"
    );

    transport.send_line("t1 NOOP").expect("send succeeds");
    assert_eq!(written.borrow().as_slice(), b"t1 NOOP\r\n");
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
            .search(&AccountId::new("other"), "invoice", None, 10, &SearchWindow::default())
            .is_err()
    );
}

/// The real shape of a subject with non-ASCII in it: RFC 2047 encoded-words,
/// folded across lines because the encoding made it long. Taken from live
/// mail — decoding one word but dropping the continuation would look like it
/// worked while quietly truncating.
/// The `?Q?=F0…` word matters: a payload opening with an escape is where
/// searching for the `?=` terminator finds a false one inside the word.
const ENCODED_HEADERS: &str = concat!(
    "Subject: =?UTF-8?B?4pyI77iPIEZsw7xnZQ==?=\r\n",
    " =?UTF-8?Q?=F0=9F=99=8C_nach_Hanoi_jetzt_ab_1=2E081?=\r\n",
    "From: =?ISO-8859-1?Q?Fl=FCge?= <deals@example.com>\r\n",
    "Date: Wed, 15 Jul 2026 20:52:22 +0000\r\n",
    "\r\n"
);

#[test]
fn encoded_and_folded_headers_arrive_as_readable_text() {
    let mut script = login_script();
    script.extend([line("* 1 EXISTS"), line("t2 OK SELECT completed")]);
    script.extend(fetch_script("t3", 101, "", ENCODED_HEADERS, BODY));
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

    // Both words, joined without the fold's whitespace between them.
    assert_eq!(message.subject(), "✈️ Flüge🙌 nach Hanoi jetzt ab 1.081");
    // Latin-1 and a mix of encoded and plain text in one line.
    assert_eq!(message.sender(), "Flüge <deals@example.com>");
    assert_eq!(message.date(), "Wed, 15 Jul 2026 20:52:22 +0000");
}

#[test]
fn plain_headers_are_left_exactly_as_they_are() {
    let mut script = login_script();
    script.extend([line("* 1 EXISTS"), line("t2 OK SELECT completed")]);
    // A subject that merely looks like an encoded-word must survive intact.
    let headers = "Subject: Re: =?what?= is this\r\nFrom: a@example.com\r\n\r\n";
    script.extend(fetch_script("t3", 101, "", headers, BODY));
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

    assert_eq!(message.subject(), "Re: =?what?= is this");
}

#[test]
fn an_empty_query_asks_the_server_for_everything_newest_first() {
    let mut script = login_script();
    script.extend([
        line("* 3 EXISTS"),
        line("t2 OK SELECT completed"),
        line("* SEARCH 101 102"),
        line("t3 OK SEARCH completed"),
    ]);
    // Newest first, so the higher UID is fetched before the lower one.
    script.extend(fetch_script("t4", 102, "", HEADERS, BODY));
    script.extend(fetch_script("t5", 101, "", HEADERS, BODY));
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
        .search(&account_id, "", None, 10, &SearchWindow::default())
        .expect("search succeeds");

    // No mailbox and no query: the inbox, unfiltered — never a LIST that
    // would rank messages from different mailboxes against each other.
    let sent = log.lines();
    assert_eq!(sent[1], "t2 SELECT \"INBOX\"");
    assert_eq!(sent[2], "t3 UID SEARCH ALL");
    assert_eq!(
        hits.iter().map(SearchHit::message_id).collect::<Vec<_>>(),
        ["INBOX/102", "INBOX/101"]
    );
}

#[test]
fn a_limit_cuts_off_the_oldest_mail_not_the_newest() {
    let mut script = login_script();
    script.extend([
        line("* 3 EXISTS"),
        line("t2 OK SELECT completed"),
        line("* SEARCH 101 102 103"),
        line("t3 OK SEARCH completed"),
    ]);
    script.extend(fetch_script("t4", 103, "", HEADERS, BODY));
    let client = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let account_id = AccountId::new("work");
    let provider = ImapMailProvider::new(account_id.clone(), client);

    let hits = provider
        .search(&account_id, "", Some("INBOX"), 1, &SearchWindow::default())
        .expect("search succeeds");

    // One hit asked for, out of three: it must be the newest, and the other
    // two must never be fetched at all.
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id(), "INBOX/103");
}

#[test]
fn xoauth2_sends_the_bearer_blob_and_logs_in() {
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 OK me@gmail.com authenticated (Success)"),
    ];
    let log = SentLog::default();

    let client = ImapClient::connect_with(
        ScriptedTransport::new(script, log.clone()),
        "me@gmail.com",
        "ya29.token",
        ImapAuth::XOAuth2,
    );
    assert!(client.is_ok(), "xoauth2 login succeeds");

    // base64("user=me@gmail.com\x01auth=Bearer ya29.token\x01\x01") — the
    // exact bytes Gmail expects, control characters and all.
    assert_eq!(
        log.lines(),
        ["t1 AUTHENTICATE XOAUTH2 dXNlcj1tZUBnbWFpbC5jb20BYXV0aD1CZWFyZXIgeWEyOS50b2tlbgEB"]
    );
}

#[test]
fn a_rejected_token_gets_the_empty_reply_the_server_waits_for() {
    // Gmail answers a bad token with a `+` continuation carrying a base64
    // error, and will not send the tagged NO until the client replies. A
    // client that skips the empty line hangs here forever.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("+ eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiQmVhcmVyIn0="),
        line("t1 NO Invalid credentials (Failure)"),
    ];
    let log = SentLog::default();

    let result = ImapClient::connect_with(
        ScriptedTransport::new(script, log.clone()),
        "me@gmail.com",
        "expired",
        ImapAuth::XOAuth2,
    );

    let error = match result {
        Ok(_) => panic!("an expired token must not authenticate"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("Invalid credentials"), "got: {error}");
    assert_eq!(log.lines().len(), 2, "the empty continuation reply is owed");
    assert_eq!(log.lines()[1], "", "and it must be an empty line");
}

#[test]
fn a_date_window_narrows_the_search_to_since_and_before() {
    let mut script = login_script();
    script.extend([
        line("* 1 EXISTS"),
        line("t2 OK SELECT completed"),
        line("* SEARCH 42"),
        line("t3 OK SEARCH completed"),
    ]);
    script.extend(fetch_script("t4", 42, "", HEADERS, BODY));
    let log = SentLog::default();
    let client = ImapClient::connect(
        ScriptedTransport::new(script, log.clone()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let account_id = AccountId::new("work");
    let provider = ImapMailProvider::new(account_id.clone(), client);

    let window = SearchWindow {
        since: Some("01-Jul-2026".to_owned()),
        before: Some("08-Jul-2026".to_owned()),
    };
    provider
        .search(&account_id, "invoice", Some("INBOX"), 10, &window)
        .expect("search succeeds");

    // The text match and both date bounds arrive as one ANDed criterion.
    let sent = log.lines();
    assert_eq!(
        sent[2],
        "t3 UID SEARCH TEXT \"invoice\" SINCE 01-Jul-2026 BEFORE 08-Jul-2026"
    );
}

#[test]
fn a_date_only_search_needs_no_text_criterion() {
    let mut script = login_script();
    script.extend([
        line("* 1 EXISTS"),
        line("t2 OK SELECT completed"),
        line("* SEARCH"),
        line("t3 OK SEARCH completed"),
    ]);
    let log = SentLog::default();
    let client = ImapClient::connect(
        ScriptedTransport::new(script, log.clone()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let account_id = AccountId::new("work");
    let provider = ImapMailProvider::new(account_id.clone(), client);

    let window = SearchWindow {
        since: Some("01-Jul-2026".to_owned()),
        before: None,
    };
    // No query and no mailbox, but a date bound: this is a real filter, so it
    // must stay a SINCE search — never collapse to `ALL`.
    provider
        .search(&account_id, "", Some("INBOX"), 10, &window)
        .expect("search succeeds");

    assert_eq!(log.lines()[2], "t3 UID SEARCH SINCE 01-Jul-2026");
}
