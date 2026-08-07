//! The IMAP protocol core against a scripted server: canned responses in,
//! sent commands recorded — no network anywhere.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Cursor, Read, Write};
use std::rc::Rc;

use torromail_core::{
    AccountId, CoreError, CoreResult, ImapAuth, ImapClient, ImapMailProvider, ImapTransport,
    MailProvider, MarkChange, SearchHit, SearchWindow, StreamImapTransport,
};

#[derive(Debug)]
enum Incoming {
    Line(String),
    Bytes(Vec<u8>),
    /// The connection dying where the script says so, rather than the server
    /// answering — the one thing a canned list of lines cannot otherwise say.
    Failure(CoreError),
}

fn line(text: &str) -> Incoming {
    Incoming::Line(text.to_owned())
}

fn dropped_connection() -> Incoming {
    Incoming::Failure(CoreError::ProviderFailure("connection reset".to_owned()))
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
            Some(Incoming::Failure(error)) => Err(error),
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
fn append_sends_the_message_as_a_literal_after_the_continuation() {
    let mut script = login_script();
    // The server invites the literal with a `+`, then accepts it.
    script.extend([
        line("+ OK go ahead"),
        line("t2 OK [APPENDUID 1 5] APPEND completed"),
    ]);
    let log = SentLog::default();
    let mut client = ImapClient::connect(
        ScriptedTransport::new(script, log.clone()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");

    let message = "From: me@example.com\r\nSubject: Hi\r\n\r\nBody";
    client
        .append("Drafts", "\\Draft", message)
        .expect("append succeeds");

    let sent = log.lines();
    // The command announces the message's exact byte length...
    assert_eq!(
        sent[1],
        format!("t2 APPEND \"Drafts\" (\\Draft) {{{}}}", message.len())
    );
    // ...and the message follows only after the continuation.
    assert_eq!(sent[2], message);
}

#[test]
fn moving_and_expunging_send_the_right_uid_commands() {
    let mut script = login_script();
    script.extend([
        line("* 1 EXISTS"),
        line("t2 OK SELECT completed"),
        line("t3 OK MOVE completed"),
        line("t4 OK STORE completed"),
        line("t5 OK EXPUNGE completed"),
    ]);
    let log = SentLog::default();
    let mut client = ImapClient::connect(
        ScriptedTransport::new(script, log.clone()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");

    client.uid_move("INBOX", 7, "Archive").expect("move ok");
    client.uid_expunge("INBOX", 7).expect("expunge ok");

    let sent = log.lines();
    assert_eq!(sent[1], "t2 SELECT \"INBOX\"");
    assert_eq!(sent[2], "t3 UID MOVE 7 \"Archive\"");
    // Permanent delete is a flag then a targeted expunge, not a blanket one.
    assert_eq!(sent[3], "t4 UID STORE 7 +FLAGS (\\Deleted)");
    assert_eq!(sent[4], "t5 UID EXPUNGE 7");
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

#[test]
fn a_refused_password_is_a_credential_rejection_not_a_transport_failure() {
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO [AUTHENTICATIONFAILED] Invalid credentials"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "wrong",
    );

    match result {
        Ok(_) => panic!("login must fail"),
        Err(CoreError::CredentialRejected(message)) => {
            assert!(message.contains("Invalid credentials"), "got: {message}");
        }
        Err(other) => panic!("a refused login must be CredentialRejected, got: {other:?}"),
    }
}

#[test]
fn a_refused_token_is_a_credential_rejection_too() {
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("+ eyJzdGF0dXMiOiI0MDEifQ=="),
        line("t1 NO Invalid credentials (Failure)"),
    ];
    let result = ImapClient::connect_with(
        ScriptedTransport::new(script, SentLog::default()),
        "me@gmail.com",
        "expired",
        ImapAuth::XOAuth2,
    );

    assert!(
        matches!(result, Err(CoreError::CredentialRejected(_))),
        "an expired token is a credential problem, not a transport one"
    );
}

#[test]
fn a_broken_greeting_stays_a_provider_failure() {
    // Nothing was refused — we never got far enough to say a password. This
    // is the case the three-strikes grace period exists for.
    let script = vec![line("* BYE server too busy")];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    );

    assert!(
        matches!(result, Err(CoreError::ProviderFailure(_))),
        "a failure before authentication must not accuse the credentials"
    );
}

#[test]
fn a_refused_command_after_login_is_not_a_credential_problem() {
    let mut script = login_script();
    script.push(line("t2 NO LIST failed"));
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds")
    .list_mailboxes();

    assert!(
        matches!(result, Err(CoreError::ProviderFailure(_))),
        "only the login command can produce a credential rejection"
    );
}

#[test]
fn a_login_that_never_gets_an_answer_is_a_transport_failure() {
    // The seam that matters: LOGIN went out, then the connection died before
    // the server said anything. Nothing was refused, so nothing may be
    // blamed on the password.
    let script = vec![line("* OK IMAP4rev1 server ready"), dropped_connection()];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    );

    match result {
        Ok(_) => panic!("login must fail"),
        Err(CoreError::ProviderFailure(message)) => {
            assert!(message.contains("connection reset"), "got: {message}");
            assert!(
                !message.contains("rejected the login"),
                "a silence is not a refusal: {message}"
            );
        }
        Err(other) => panic!("a dead connection is a transport failure, got: {other:?}"),
    }
}

#[test]
fn a_temporary_backend_failure_is_not_a_wrong_password() {
    // RFC 5530 has a code for exactly this, and it means "try later" — the
    // one thing a credential rejection is not allowed to mean.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO [UNAVAILABLE] Temporary authentication failure"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    );

    assert!(
        matches!(result, Err(CoreError::ProviderFailure(_))),
        "a server having a bad day must not be reported as a wrong password"
    );
}

#[test]
fn a_refused_plaintext_login_blames_the_connection_not_the_password() {
    // Dovecot's answer on a connection that never got its TLS. The password
    // may well be perfect; the security setting is what is wrong.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line(
            "t1 NO [PRIVACYREQUIRED] Plaintext authentication disallowed on non-secure connections",
        ),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    );

    assert!(
        matches!(result, Err(CoreError::ProviderFailure(_))),
        "a missing TLS upgrade is a connection problem, not a credential one"
    );
}

#[test]
fn a_tagged_bad_never_accuses_the_credentials() {
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 BAD Error in IMAP command LOGIN: Unexpected argument"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    );

    assert!(
        matches!(result, Err(CoreError::ProviderFailure(_))),
        "BAD is us speaking nonsense, and says nothing about the password"
    );
}

#[test]
fn a_refusal_with_no_response_code_still_means_a_wrong_password() {
    // Plenty of servers send no response code at all. A plain NO to LOGIN is
    // the oldest way there is of saying the credentials are wrong.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO Login failed"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "wrong",
    );

    assert!(
        matches!(result, Err(CoreError::CredentialRejected(_))),
        "a bare NO to LOGIN is a refusal of the credentials"
    );
}

#[test]
fn the_actionable_alert_line_travels_with_the_refusal() {
    // Gmail puts the sentence the user has to act on in an untagged ALERT
    // ahead of the tagged NO, which on its own says nothing useful.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line(
            "* NO [ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833",
        ),
        line("t1 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "me@gmail.com",
        "wrong",
    );

    match result {
        Ok(_) => panic!("login must fail"),
        Err(CoreError::CredentialRejected(message)) => {
            assert!(
                message.contains("Application-specific password required"),
                "the fix the user needs must survive: {message}"
            );
        }
        Err(other) => panic!("a refused login must be CredentialRejected, got: {other:?}"),
    }
}

#[test]
fn a_server_that_echoes_the_login_does_not_get_the_password_into_the_error() {
    // This text ends up in a log file on disk, and LOGIN is the one command
    // whose arguments are the secret itself.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 BAD Error in IMAP command LOGIN \"work@example.com\" \"hunter2\""),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "hunter2",
    );

    let message = match result {
        Ok(_) => panic!("login must fail"),
        Err(error) => error.to_string(),
    };
    assert!(!message.contains("hunter2"), "secret leaked: {message}");
    assert!(message.contains("***"), "got: {message}");
}

#[test]
fn gmail_asking_for_an_app_specific_password_is_a_credential_problem() {
    // 2FA without an app password. Permanent, and only the user can fix it —
    // the ALERT marker says "show this to a human", not "the server is fine".
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line(
            "t1 NO [ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)",
        ),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "me@gmail.com",
        "plain-password",
    );

    assert!(
        matches!(result, Err(CoreError::CredentialRejected(_))),
        "an account that needs an app password will never fix itself"
    );
}

#[test]
fn gmail_demanding_a_web_sign_in_is_a_credential_problem() {
    // The code carries a URL inside the brackets, so only the code word may
    // be matched.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO [WEBALERT https://accounts.google.com/signin/continue] Web login required"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "me@gmail.com",
        "plain-password",
    );

    assert!(
        matches!(result, Err(CoreError::CredentialRejected(_))),
        "a demand for a web sign-in is the user's to act on, not the network's"
    );
}

#[test]
fn an_unknown_diagnosis_code_still_lands_on_the_transport_side() {
    // Stripping the alert markers must not turn into "anything unrecognised
    // is a wrong password". A code that does diagnose, but not an
    // authentication problem, keeps its three strikes.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO [INUSE] Mailbox is locked by another session"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    );

    assert!(
        matches!(result, Err(CoreError::ProviderFailure(_))),
        "an unrecognised diagnosis must not be read as a wrong password"
    );
}

#[test]
fn a_failure_that_spares_the_credentials_does_not_say_they_were_rejected() {
    // The app shows this sentence to the user verbatim. Telling someone their
    // login was rejected is the one reading a temporary backend failure must
    // not invite.
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO [UNAVAILABLE] Temporary authentication failure"),
    ];
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    );

    let message = match result {
        Ok(_) => panic!("login must fail"),
        Err(error) => error.to_string(),
    };
    assert!(
        !message.contains("rejected"),
        "nothing was rejected here: {message}"
    );
    assert!(
        message.contains("Temporary authentication failure"),
        "got: {message}"
    );
}

#[test]
fn an_alert_in_a_subject_line_is_not_an_alert() {
    // "[ALERT]" is a real spam-subject convention. Untagged prose that merely
    // contains it must not be spliced into an unrelated error.
    let mut script = login_script();
    script.extend([
        line("* LIST (\\HasNoChildren) \"/\" \"[ALERT] Free money inside\""),
        line("t2 NO LIST failed"),
    ]);
    let result = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds")
    .list_mailboxes();

    let message = match result {
        Ok(_) => panic!("the LIST must fail"),
        Err(error) => error.to_string(),
    };
    assert!(
        !message.contains("Free money"),
        "unrelated prose leaked into the error: {message}"
    );
}

#[test]
fn an_echoed_xoauth2_blob_does_not_carry_the_token_into_the_error() {
    // The blob is base64 of the token, so echoing it back leaks the token to
    // anyone who reads the log. This is the exact blob for the user and token
    // below, as pinned by `xoauth2_sends_the_bearer_blob_and_logs_in`.
    let blob = "dXNlcj1tZUBnbWFpbC5jb20BYXV0aD1CZWFyZXIgeWEyOS50b2tlbgEB";
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line(&format!(
            "t1 BAD Error in IMAP command AUTHENTICATE: {blob}"
        )),
    ];
    let result = ImapClient::connect_with(
        ScriptedTransport::new(script, SentLog::default()),
        "me@gmail.com",
        "ya29.token",
        ImapAuth::XOAuth2,
    );

    let message = match result {
        Ok(_) => panic!("the token must not authenticate"),
        Err(error) => error.to_string(),
    };
    assert!(!message.contains(blob), "the blob leaked: {message}");
    assert!(
        !message.contains("ya29.token"),
        "the token leaked: {message}"
    );
    assert!(message.contains("***"), "got: {message}");
}

#[test]
fn an_echoed_raw_token_does_not_reach_the_error_either() {
    let script = vec![
        line("* OK IMAP4rev1 server ready"),
        line("t1 NO Invalid credentials for token ya29.token (Failure)"),
    ];
    let result = ImapClient::connect_with(
        ScriptedTransport::new(script, SentLog::default()),
        "me@gmail.com",
        "ya29.token",
        ImapAuth::XOAuth2,
    );

    let message = match result {
        Ok(_) => panic!("the token must not authenticate"),
        Err(error) => error.to_string(),
    };
    assert!(
        !message.contains("ya29.token"),
        "the token leaked: {message}"
    );
    assert!(message.contains("***"), "got: {message}");
}

/// A multipart/mixed with a body and a base64 PDF beside it — the shape the
/// attachment tools have to handle end to end.
const ATTACHMENT_HEADERS: &str = concat!(
    "Subject: Mit Anhang\r\n",
    "From: a@example.com\r\n",
    "Content-Type: multipart/mixed; boundary=\"outer\"\r\n",
    "\r\n",
);
const ATTACHMENT_BODY: &str = concat!(
    "--outer\r\n",
    "Content-Type: text/plain; charset=utf-8\r\n",
    "\r\n",
    "Bitte den Anhang pruefen.\r\n",
    "--outer\r\n",
    "Content-Type: application/pdf\r\n",
    "Content-Disposition: attachment; filename=angebot.pdf\r\n",
    "Content-Transfer-Encoding: base64\r\n",
    "\r\n",
    "JVBERi0xLjQ=\r\n",
    "--outer--\r\n",
);

#[test]
fn a_fetched_message_lists_and_serves_its_attachments() {
    let mut script = login_script();
    script.extend([line("* 1 EXISTS"), line("t2 OK SELECT completed")]);
    // Three fetches: the message read, then one per download attempt — the
    // provider re-fetches rather than holding message state between calls.
    script.extend(fetch_script("t3", 101, "", ATTACHMENT_HEADERS, ATTACHMENT_BODY));
    script.extend(fetch_script("t4", 101, "", ATTACHMENT_HEADERS, ATTACHMENT_BODY));
    script.extend(fetch_script("t5", 101, "", ATTACHMENT_HEADERS, ATTACHMENT_BODY));
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
    assert_eq!(message.body(), "Bitte den Anhang pruefen.");
    assert_eq!(message.attachments().len(), 1);
    let info = &message.attachments()[0];
    assert_eq!(info.id(), "2");
    assert_eq!(info.filename(), "angebot.pdf");
    assert_eq!(info.media_type(), "application/pdf");
    assert_eq!(info.size_bytes(), 8); // "JVBERi0xLjQ=" → "%PDF-1.4", 8 bytes

    let payload = provider
        .get_attachment(&account_id, "INBOX/101", "2")
        .expect("extraction works");
    assert_eq!(payload.mailbox(), "INBOX");
    assert_eq!(payload.filename(), "angebot.pdf");
    assert_eq!(payload.content(), b"%PDF-1.4");

    let missing = provider
        .get_attachment(&account_id, "INBOX/101", "5")
        .expect_err("no such part");
    assert!(matches!(missing, CoreError::AttachmentNotFound { .. }));
}

#[test]
fn select_captures_uidvalidity_for_the_cache() {
    let mut script = login_script();
    script.extend([
        line("* 1 EXISTS"),
        line("* OK [UIDVALIDITY 9] UIDs valid"),
        line("t2 OK SELECT completed"),
    ]);
    script.extend(fetch_script("t3", 101, "", HEADERS, BODY));
    let client = ImapClient::connect(
        ScriptedTransport::new(script, SentLog::default()),
        "work@example.com",
        "app-secret",
    )
    .expect("login succeeds");
    let account_id = AccountId::new("work");
    let provider = ImapMailProvider::new(account_id.clone(), client);

    let _ = provider
        .get_message(&account_id, "INBOX/101")
        .expect("fetch succeeds");
    assert_eq!(provider.mailbox_generation(&account_id, "INBOX"), Some(9));
}
