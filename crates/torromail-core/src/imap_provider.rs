//! Configuration boundary for the real IMAP provider.
//!
//! Connection facts live here; the secret itself never does. `SecretRef`
//! points into the platform keychain and refuses to reveal itself in any
//! Debug output, so configs can travel through logs and diagnostics safely.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use crate::{
    AccountId, AttachmentPayload, CoreError, CoreResult, MailProvider, MarkChange, SearchHit,
    SearchWindow, StoredMessage,
};

/// A pointer to a credential in the platform keychain — never the
/// credential itself.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretRef(String);

impl SecretRef {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `keychain://{service}/{account}` — the one scheme resolvers know.
    pub fn keychain_location(&self) -> Option<(&str, &str)> {
        self.0.strip_prefix("keychain://")?.split_once('/')
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretRef(<redacted>)")
    }
}

/// How the session proves who it is. Only the mechanism lives here — where a
/// token comes from and how it is renewed is somebody else's problem, which
/// is what keeps this crate free of a network stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImapAuth {
    /// `LOGIN user secret` — a password or an app password.
    #[default]
    Password,
    /// `AUTHENTICATE XOAUTH2` — the secret is a bearer access token. What
    /// Gmail and Microsoft 365 require; both have retired plain passwords for
    /// everything but Google's personal app passwords.
    XOAuth2,
}

/// How the session gets its TLS. This is a fact about the server, not about
/// the port: 993 and 465 are conventions, not rules, and a provider is free
/// to offer implicit TLS somewhere else entirely. Deriving it from the port
/// number is what used to make such a server unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionSecurity {
    /// TLS from the first byte — the server speaks only after the handshake.
    #[default]
    Tls,
    /// Plaintext up to the `STARTTLS` upgrade, which happens before a single
    /// byte of credentials moves.
    StartTls,
}

impl ConnectionSecurity {
    /// What a connection block without an explicit `security` key meant when
    /// only the port could say. Kept so documents written by an older app
    /// build keep connecting exactly as they did.
    #[must_use]
    pub fn implied_by_imap_port(port: u16) -> Self {
        if port == 993 { Self::Tls } else { Self::StartTls }
    }

    #[must_use]
    pub fn implied_by_smtp_port(port: u16) -> Self {
        if port == 465 { Self::Tls } else { Self::StartTls }
    }
}

/// Everything the IMAP adapter needs to connect, minus the secret it
/// resolves at connect time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapProviderConfig {
    pub account_id: AccountId,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub secret_ref: SecretRef,
    pub auth: ImapAuth,
    pub security: ConnectionSecurity,
}

impl ImapProviderConfig {
    pub fn new(
        account_id: AccountId,
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        secret_ref: SecretRef,
    ) -> Self {
        Self {
            account_id,
            host: host.into(),
            port,
            username: username.into(),
            secret_ref,
            auth: ImapAuth::Password,
            security: ConnectionSecurity::Tls,
        }
    }

    #[must_use]
    pub fn with_auth(mut self, auth: ImapAuth) -> Self {
        self.auth = auth;
        self
    }

    #[must_use]
    pub fn with_security(mut self, security: ConnectionSecurity) -> Self {
        self.security = security;
        self
    }
}

/// The wire an IMAP session runs over. The scripted transport in the tests
/// and the TCP transport below share it, so the protocol core is testable
/// without a network.
pub trait ImapTransport {
    /// Sends one command line; the transport appends CRLF.
    fn send_line(&mut self, line: &str) -> CoreResult<()>;
    /// Reads one response line without its CRLF.
    fn read_line(&mut self) -> CoreResult<String>;
    /// Reads exactly `count` bytes of an IMAP literal.
    fn read_bytes(&mut self, count: usize) -> CoreResult<Vec<u8>>;
}

/// What one UID FETCH yields, before it becomes a `StoredMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedMessage {
    pub uid: u32,
    pub subject: String,
    pub sender: String,
    pub date: String,
    pub body: String,
    /// The body exactly as fetched, MIME fences and all — what the
    /// attachment walk reads; `body` above is the rendered text.
    pub raw_body: String,
    /// Top-level `Content-Type` / `Content-Transfer-Encoding`, kept because
    /// the raw body cannot be interpreted without them.
    pub content_type: String,
    pub transfer_encoding: String,
    pub seen: bool,
    pub flagged: bool,
}

/// A search hit's headers, without its body. Search fans out across mailboxes
/// and would otherwise pull every match in full for a subject line it shows
/// and a body it never returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedSummary {
    pub uid: u32,
    pub subject: String,
    pub sender: String,
    pub date: String,
}

/// The header fields that relate one message to the rest of its conversation.
/// Plain IMAP has no thread id, so this is how a thread is reconstructed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadHeaders {
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub in_reply_to: Vec<String>,
}

/// One untagged response line, with any literals it carried in order.
struct ResponseLine {
    text: String,
    literals: Vec<Vec<u8>>,
}

/// The word the server refused with. `NO` is the command failing on its own
/// terms, which is the only way a credential problem can be expressed; `BAD`
/// is the client having spoken nonsense and never says anything about the
/// credentials. Anything else a server invents is read as `Bad`, the side
/// that accuses nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefusalKind {
    No,
    Bad,
}

/// A tagged refusal the server actually spoke, parsed far enough that
/// deciding what it means reads a field instead of hunting through a
/// sentence.
#[derive(Debug)]
struct Refusal {
    kind: RefusalKind,
    /// The RFC 5530 response code, as `response_code` reads it — but only
    /// where it diagnoses something.
    ///
    /// `None` when the server refused without giving a *reason* — either it
    /// sent no response code at all, which is what plenty of servers do to a
    /// wrong password, or it sent only an alert marker, which says nothing
    /// about the cause. See `is_human_alert`.
    code: Option<String>,
    /// Everything the server said in refusing, including any untagged
    /// `[ALERT]` sentence that came ahead of the tagged line. This text
    /// reaches the user, so it keeps the server's own words.
    text: String,
}

impl Refusal {
    /// Reads the remainder of a tagged answer — everything after `t3 ` — plus
    /// the untagged alert line that may have preceded it.
    fn parse(rest: &str, alert: Option<&str>) -> Self {
        let word = rest.split_whitespace().next().unwrap_or_default();
        let kind = if word.eq_ignore_ascii_case("NO") {
            RefusalKind::No
        } else {
            RefusalKind::Bad
        };

        // An alert marker is not a diagnosis, so it is not kept as one: drop
        // it and the refusal reads as the bare `NO` it effectively is.
        let code = response_code(rest).filter(|code| !is_human_alert(code));

        let text = match alert {
            Some(alert) => format!("{rest} ({alert})"),
            None => rest.to_owned(),
        };

        Self { kind, code, text }
    }

    /// Whether this refusal accuses the credentials — the question the two
    /// authentication paths ask, answered in one place so they cannot drift.
    ///
    /// Only the codes that name an authentication problem count, plus a
    /// refusal that named no reason at all, which is how most servers say
    /// "wrong password". Any other *diagnosis* code is the server or the
    /// connection having a problem of its own: `[UNAVAILABLE]` is RFC 5530's
    /// *temporary* backend failure, `[PRIVACYREQUIRED]` means this connection
    /// wants TLS first. Reporting those as a wrong password is worse than
    /// saying nothing, because downstream a credential rejection turns the
    /// account red on the first occurrence and notifies the user, while a
    /// provider failure gets three strikes first. So an unrecognised
    /// diagnosis lands on the transport side on purpose.
    ///
    /// `[ALERT]` and `[WEBALERT]` are deliberately *not* treated as
    /// diagnoses — `Refusal::parse` has already stripped them. RFC 3501
    /// defines `ALERT` as "show this text to the human", which is a statement
    /// about presentation and carries no information about why the command
    /// failed. Reading it as one got Gmail's two permanent, user-must-act
    /// answers exactly backwards: `NO [ALERT] Application-specific password
    /// required` and `NO [WEBALERT <url>] Web login required` would have
    /// collected three transport strikes and then reported an unreachable
    /// server. The price of stripping them is that Gmail's `NO [ALERT] Too
    /// many simultaneous connections` throttle goes red on the first strike —
    /// a wrong answer that the next check corrects, traded against one that
    /// never corrects itself.
    fn accuses_the_credentials(&self) -> bool {
        if self.kind == RefusalKind::Bad {
            return false;
        }
        match self.code.as_deref() {
            None => true,
            Some(code) => matches!(
                code,
                "AUTHENTICATIONFAILED" | "AUTHORIZATIONFAILED" | "EXPIRED"
            ),
        }
    }

    /// The same refusal with every occurrence of `secret` blanked out. Both
    /// authentication commands carry their secret in an argument — `LOGIN` in
    /// the clear, `AUTHENTICATE XOAUTH2` base64-wrapped — and a server that
    /// echoes back the command it could not parse would otherwise put it in
    /// an error that reaches a plaintext log on disk. The XOAUTH2 path has to
    /// blank the blob as well as the raw token, because the blob decodes
    /// straight back to it.
    ///
    /// Called after `parse`, never before: the classification reads `kind`
    /// and `code`, so a secret that happens to look like a response code
    /// cannot change the verdict by being redacted out of the text.
    fn with_secret_redacted(mut self, secret: &str) -> Self {
        if !secret.is_empty() {
            self.text = self.text.replace(secret, "***");
        }
        self
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

/// Turns a refusal to an authentication command into the error the rest of
/// the app reasons about. Both authentication paths go through here so that
/// what counts as a wrong password is decided exactly once.
///
/// The wording follows the verdict, because this string is shown to the user
/// verbatim. "Rejected" is only said where the credentials really were
/// rejected — telling someone whose server is merely having a bad day that
/// their login was rejected invites exactly the conclusion the
/// classification above exists to prevent.
fn authentication_error(refusal: &Refusal, what: &str) -> CoreError {
    if refusal.accuses_the_credentials() {
        CoreError::CredentialRejected(format!("IMAP rejected the {what}: {refusal}"))
    } else {
        CoreError::ProviderFailure(format!("IMAP could not check the {what}: {refusal}"))
    }
}

/// The response code of a `NO`/`BAD`/`OK`/`BYE` answer, uppercased and
/// stripped of its brackets and arguments — `AUTHENTICATIONFAILED` out of
/// `[AUTHENTICATIONFAILED]`, `WEBALERT` out of `[WEBALERT <url>]`.
///
/// Positional on purpose: RFC 3501 puts the code straight after the
/// condition word, so a `[` anywhere later is prose — a message subject, a
/// mailbox name — and must not be read as a code.
fn response_code(rest: &str) -> Option<String> {
    rest.split_once(' ')
        .and_then(|(_, remainder)| remainder.trim_start().strip_prefix('['))
        .and_then(|bracketed| bracketed.split_once(']'))
        .and_then(|(inside, _)| inside.split_whitespace().next())
        .map(str::to_ascii_uppercase)
}

/// Whether `code` only means "show this text to the human". RFC 3501 defines
/// `ALERT` that way and Gmail's `WEBALERT` follows it; neither says anything
/// about *why* a command failed. One predicate for both places that care —
/// the classification, which must not mistake it for a diagnosis, and the
/// untagged capture below, which wants exactly these lines.
fn is_human_alert(code: &str) -> bool {
    matches!(code, "ALERT" | "WEBALERT")
}

/// The actionable sentence out of an untagged `* NO [ALERT] …`, which is
/// where Gmail and Yahoo put the thing the user has to go and do
/// ("Application-specific password required: <url>"). The tagged refusal
/// that follows it is usually far less useful.
///
/// The alert has to sit where a response code goes, after the untagged
/// condition word; the literal text "[ALERT]" inside a subject line is a
/// real spam convention and must not be spliced into an unrelated error.
/// Where a server sends several, the first is kept.
fn untagged_alert(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("* ")?;
    let word = rest.split_whitespace().next()?;
    if !matches!(
        word.to_ascii_uppercase().as_str(),
        "NO" | "BAD" | "OK" | "BYE"
    ) {
        return None;
    }
    response_code(rest)
        .is_some_and(|code| is_human_alert(&code))
        .then_some(rest)
}

/// One logged-in IMAP session speaking the smallest useful IMAP4rev1
/// subset: LIST, SELECT, UID SEARCH, UID FETCH, UID STORE. Bodies are
/// fetched with PEEK — reading never sets flags; changing flags is what the
/// mark permission is for.
pub struct ImapClient<T: ImapTransport> {
    transport: T,
    next_tag: u32,
    selected: Option<String>,
    /// UIDVALIDITY per mailbox, as the server stated it on SELECT. The cache
    /// keys its rows by this — a changed value means every cached UID names
    /// a different message now.
    uidvalidity: BTreeMap<String, u32>,
}

impl<T: ImapTransport> ImapClient<T> {
    /// Reads the greeting and logs in with a password.
    pub fn connect(transport: T, username: &str, secret: &str) -> CoreResult<Self> {
        Self::connect_with(transport, username, secret, ImapAuth::Password)
    }

    /// Reads the greeting and authenticates the way `auth` says. For
    /// `XOAuth2` the secret is an access token, not a password.
    pub fn connect_with(
        transport: T,
        username: &str,
        secret: &str,
        auth: ImapAuth,
    ) -> CoreResult<Self> {
        let mut client = Self {
            transport,
            next_tag: 0,
            selected: None,
            uidvalidity: BTreeMap::new(),
        };
        let greeting = client.transport.read_line()?;
        if !greeting.starts_with("* OK") {
            return Err(CoreError::ProviderFailure(format!(
                "unexpected IMAP greeting: {greeting}"
            )));
        }
        client.authenticate(username, secret, auth)?;
        Ok(client)
    }

    /// Authenticates on a connection whose greeting was already read — the
    /// state right after a STARTTLS upgrade, where the server sends no fresh
    /// greeting and reading for one would hang.
    pub fn connect_upgraded_with(
        transport: T,
        username: &str,
        secret: &str,
        auth: ImapAuth,
    ) -> CoreResult<Self> {
        let mut client = Self {
            transport,
            next_tag: 0,
            selected: None,
            uidvalidity: BTreeMap::new(),
        };
        client.authenticate(username, secret, auth)?;
        Ok(client)
    }

    fn authenticate(&mut self, username: &str, secret: &str, auth: ImapAuth) -> CoreResult<()> {
        match auth {
            ImapAuth::Password => {
                self.command_or_refusal(&format!(
                    "LOGIN {} {}",
                    imap_quoted(username),
                    imap_quoted(secret)
                ))?
                .map_err(|refusal| {
                    authentication_error(&refusal.with_secret_redacted(secret), "login")
                })?;
            }
            ImapAuth::XOAuth2 => self.authenticate_xoauth2(username, secret)?,
        }
        Ok(())
    }

    /// SASL XOAUTH2: one base64 blob carrying the user and a bearer token.
    ///
    /// The failure path is the awkward part. A rejected token does not get a
    /// tagged NO straight away — the server sends a `+` continuation holding
    /// a base64 error, and the client owes it an empty line before the real
    /// verdict arrives. Skipping that reply leaves the session wedged
    /// mid-handshake.
    fn authenticate_xoauth2(&mut self, username: &str, access_token: &str) -> CoreResult<()> {
        let initial = encode_base64(
            format!("user={username}\x01auth=Bearer {access_token}\x01\x01").as_bytes(),
        );
        self.next_tag += 1;
        let tag = format!("t{}", self.next_tag);
        self.transport
            .send_line(&format!("{tag} AUTHENTICATE XOAUTH2 {initial}"))?;

        let mut alert = None;
        loop {
            let line = self.transport.read_line()?;

            if let Some(rest) = line.strip_prefix(&format!("{tag} ")) {
                if rest.starts_with("OK") {
                    return Ok(());
                }
                // The same verdict the password path uses. A rejected token is
                // a credential problem, but `NO [UNAVAILABLE]` on this command
                // is no more the token's fault than it is a password's. The
                // blob is redacted alongside the raw token because a server
                // echoing the command it could not parse hands back the one
                // that decodes into the other.
                return Err(authentication_error(
                    &Refusal::parse(rest, alert.as_deref())
                        .with_secret_redacted(access_token)
                        .with_secret_redacted(&initial),
                    "access token",
                ));
            }

            if line.starts_with('+') {
                // The rejection detail, and a server waiting on us. `command`
                // cannot be used for this exchange precisely because it would
                // sit here waiting for a tagged line that only arrives after
                // this empty reply.
                self.transport.send_line("")?;
                continue;
            }
            if alert.is_none() {
                alert = untagged_alert(&line).map(str::to_owned);
            }
            // Untagged chatter (`* CAPABILITY …`) — not our business.
        }
    }

    pub fn list_mailboxes(&mut self) -> CoreResult<Vec<String>> {
        let lines = self.command("LIST \"\" \"*\"")?;
        Ok(lines
            .iter()
            .filter_map(|line| parse_list_mailbox(&line.text))
            .collect())
    }

    /// UIDs matching `query` within `window`, ascending — oldest first, as
    /// IMAP hands them over. Criteria are space-separated and ANDed: a text
    /// match, a `SINCE`, a `BEFORE`. With none of them the criterion is
    /// `ALL`, because `TEXT ""` tells the server to match every message
    /// against nothing, which servers answer inconsistently.
    pub fn uid_search(
        &mut self,
        mailbox: &str,
        query: &str,
        window: &SearchWindow,
    ) -> CoreResult<Vec<u32>> {
        self.select(mailbox)?;

        let mut criteria = Vec::new();
        if !query.trim().is_empty() {
            criteria.push(format!("TEXT {}", imap_quoted(query)));
        }
        if let Some(since) = &window.since {
            criteria.push(format!("SINCE {since}"));
        }
        if let Some(before) = &window.before {
            criteria.push(format!("BEFORE {before}"));
        }
        if criteria.is_empty() {
            criteria.push("ALL".to_owned());
        }

        let lines = self.command(&format!("UID SEARCH {}", criteria.join(" ")))?;
        Ok(collect_search_uids(&lines))
    }

    pub fn uid_fetch(&mut self, mailbox: &str, uid: u32) -> CoreResult<FetchedMessage> {
        self.select(mailbox)?;
        let lines = self.command(&format!(
            "UID FETCH {uid} (UID FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT FROM DATE CONTENT-TYPE CONTENT-TRANSFER-ENCODING)] BODY.PEEK[TEXT])"
        ))?;
        let fetch = lines
            .iter()
            .find(|line| line.text.contains("FETCH"))
            .ok_or_else(|| {
                CoreError::ProviderFailure(format!("no FETCH response for uid {uid}"))
            })?;

        // Literals arrive in the order the command asked: the header block
        // first, then the raw body text.
        let headers = fetch
            .literals
            .first()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        let raw_body = fetch
            .literals
            .get(1)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();

        // The body arrives MIME-encoded — multipart, transfer-encoded, often
        // HTML. Unwrap it to readable text once, here, so nothing downstream
        // has to know MIME; the raw form travels along for the attachment
        // walk, which needs the fences the renderer strips.
        let content_type = header_field(&headers, "content-type").unwrap_or_default();
        let transfer_encoding =
            header_field(&headers, "content-transfer-encoding").unwrap_or_default();
        let body = crate::mime::body_to_text(&raw_body, &content_type, &transfer_encoding);

        Ok(FetchedMessage {
            uid,
            subject: header_field(&headers, "subject").unwrap_or_default(),
            sender: header_field(&headers, "from").unwrap_or_default(),
            date: header_field(&headers, "date").unwrap_or_default(),
            body,
            raw_body,
            content_type,
            transfer_encoding,
            seen: fetch.text.contains("\\Seen"),
            flagged: fetch.text.contains("\\Flagged"),
        })
    }

    /// Just the headers a search hit shows — no `BODY[TEXT]`, so a search over
    /// many mailboxes stops pulling every match in full.
    pub fn uid_fetch_summary(&mut self, mailbox: &str, uid: u32) -> CoreResult<FetchedSummary> {
        let mut summaries = self.uid_fetch_summaries(mailbox, &[uid])?;
        summaries.pop().ok_or_else(|| {
            CoreError::ProviderFailure(format!("no FETCH response for uid {uid}"))
        })
    }

    /// The summaries of many UIDs in one round trip — `UID FETCH a,b,c`.
    /// Answers in the order asked, holes silently skipped (an expunged uid
    /// is not this caller's problem). One command instead of one per
    /// message is what makes search listings and cache rebuilds bearable
    /// over a real network.
    pub fn uid_fetch_summaries(
        &mut self,
        mailbox: &str,
        uids: &[u32],
    ) -> CoreResult<Vec<FetchedSummary>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        self.select(mailbox)?;
        let set = uids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let lines = self.command(&format!(
            "UID FETCH {set} (UID BODY.PEEK[HEADER.FIELDS (SUBJECT FROM DATE)])"
        ))?;

        // The server answers one untagged FETCH per message, in its own
        // order and with the UID inside the line — collect by UID, then
        // answer in the order asked.
        let mut by_uid = BTreeMap::new();
        for fetch in lines.iter().filter(|line| line.text.contains("FETCH")) {
            let Some(uid) = parse_fetch_uid(&fetch.text) else {
                continue;
            };
            let headers = fetch
                .literals
                .first()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
            by_uid.insert(
                uid,
                FetchedSummary {
                    uid,
                    subject: header_field(&headers, "subject").unwrap_or_default(),
                    sender: header_field(&headers, "from").unwrap_or_default(),
                    date: header_field(&headers, "date").unwrap_or_default(),
                },
            );
        }
        Ok(uids
            .iter()
            .filter_map(|uid| by_uid.remove(uid))
            .collect())
    }

    /// The headers that tie a message to its conversation — its own id, and
    /// the ids it answers or descends from.
    pub fn uid_fetch_reference_headers(
        &mut self,
        mailbox: &str,
        uid: u32,
    ) -> CoreResult<ThreadHeaders> {
        self.select(mailbox)?;
        let lines = self.command(&format!(
            "UID FETCH {uid} (BODY.PEEK[HEADER.FIELDS (MESSAGE-ID REFERENCES IN-REPLY-TO)])"
        ))?;
        let fetch = lines
            .iter()
            .find(|line| line.text.contains("FETCH"))
            .ok_or_else(|| {
                CoreError::ProviderFailure(format!("no FETCH response for uid {uid}"))
            })?;
        let headers = fetch
            .literals
            .first()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();

        Ok(ThreadHeaders {
            message_id: parse_message_ids(&header_field(&headers, "message-id").unwrap_or_default())
                .into_iter()
                .next(),
            references: parse_message_ids(&header_field(&headers, "references").unwrap_or_default()),
            in_reply_to: parse_message_ids(
                &header_field(&headers, "in-reply-to").unwrap_or_default(),
            ),
        })
    }

    /// UIDs whose `field` header contains `value` — a substring match, which
    /// is how a message-id is found inside a `References` list.
    pub fn uid_search_header(
        &mut self,
        mailbox: &str,
        field: &str,
        value: &str,
    ) -> CoreResult<Vec<u32>> {
        self.select(mailbox)?;
        let lines = self.command(&format!(
            "UID SEARCH HEADER {} {}",
            imap_quoted(field),
            imap_quoted(value)
        ))?;
        Ok(collect_search_uids(&lines))
    }

    /// Append a ready-made message to `mailbox` with the given flags. Unlike
    /// every other command here, this sends a literal *to* the server: the
    /// command line ends with `{N}`, the server answers with a `+`
    /// continuation, and only then does the message travel.
    pub fn append(&mut self, mailbox: &str, flags: &str, message: &str) -> CoreResult<()> {
        self.next_tag += 1;
        let tag = format!("t{}", self.next_tag);
        self.transport.send_line(&format!(
            "{tag} APPEND {} ({flags}) {{{}}}",
            imap_quoted(mailbox),
            message.len()
        ))?;

        let continuation = self.transport.read_line()?;
        if !continuation.starts_with('+') {
            return Err(CoreError::ProviderFailure(format!(
                "APPEND was refused: {continuation}"
            )));
        }

        // `send_line` appends the CRLF that both completes the literal's bytes
        // and terminates the command, so the announced length must be the
        // message alone.
        self.transport.send_line(message)?;

        loop {
            let line = self.transport.read_line()?;
            if let Some(rest) = line.strip_prefix(&format!("{tag} ")) {
                if rest.starts_with("OK") {
                    return Ok(());
                }
                return Err(CoreError::ProviderFailure(format!("APPEND failed: {rest}")));
            }
        }
    }

    pub fn uid_store(&mut self, mailbox: &str, uid: u32, change: MarkChange) -> CoreResult<()> {
        self.select(mailbox)?;
        let (sign, flag) = match change {
            MarkChange::Seen => ('+', "\\Seen"),
            MarkChange::Unseen => ('-', "\\Seen"),
            MarkChange::Flagged => ('+', "\\Flagged"),
            MarkChange::Unflagged => ('-', "\\Flagged"),
        };
        self.command(&format!("UID STORE {uid} {sign}FLAGS ({flag})"))?;
        Ok(())
    }

    /// Move one message to another mailbox (RFC 6851). MOVE both copies and
    /// removes in one atomic step, so a soft delete — a move to Trash — cannot
    /// leave the message in two places.
    pub fn uid_move(&mut self, mailbox: &str, uid: u32, target: &str) -> CoreResult<()> {
        self.select(mailbox)?;
        self.command(&format!("UID MOVE {uid} {}", imap_quoted(target)))?;
        Ok(())
    }

    /// Delete one message for good: flag it `\Deleted`, then expunge that one
    /// UID (RFC 4315), so no other flagged message in the mailbox is caught up
    /// in it.
    pub fn uid_expunge(&mut self, mailbox: &str, uid: u32) -> CoreResult<()> {
        self.select(mailbox)?;
        self.command(&format!("UID STORE {uid} +FLAGS (\\Deleted)"))?;
        self.command(&format!("UID EXPUNGE {uid}"))?;
        Ok(())
    }

    fn select(&mut self, mailbox: &str) -> CoreResult<()> {
        if self.selected.as_deref() == Some(mailbox) {
            return Ok(());
        }
        let lines = self.command(&format!("SELECT {}", imap_quoted(mailbox)))?;
        // `* OK [UIDVALIDITY 123] …` — mandatory per RFC 3501, but a server
        // that omits it simply leaves the generation unknown.
        for line in &lines {
            if let Some(value) = parse_uidvalidity(&line.text) {
                self.uidvalidity.insert(mailbox.to_owned(), value);
            }
        }
        self.selected = Some(mailbox.to_owned());
        Ok(())
    }

    /// The UIDVALIDITY the server stated for `mailbox`, if it was ever
    /// selected on this session.
    pub fn uidvalidity(&self, mailbox: &str) -> Option<u32> {
        self.uidvalidity.get(mailbox).copied()
    }

    /// Sends `command` and reads until its tagged answer. The two failure
    /// shapes stay apart on purpose: the outer `Err` is transport — the
    /// connection broke on the way and the server never got to answer —
    /// while the inner `Err` is the server speaking a tagged `NO` or `BAD`.
    /// Only a refusal the server actually spoke can mean the credentials are
    /// wrong.
    fn command_or_refusal(
        &mut self,
        command: &str,
    ) -> CoreResult<Result<Vec<ResponseLine>, Refusal>> {
        self.next_tag += 1;
        let tag = format!("t{}", self.next_tag);
        self.transport.send_line(&format!("{tag} {command}"))?;

        let mut lines = Vec::new();
        let mut alert = None;
        loop {
            let mut text = self.transport.read_line()?;
            let mut literals = Vec::new();
            while let Some(count) = trailing_literal_size(&text) {
                literals.push(self.transport.read_bytes(count)?);
                let continuation = self.transport.read_line()?;
                text.push(' ');
                text.push_str(&continuation);
            }

            if let Some(rest) = text.strip_prefix(&format!("{tag} ")) {
                if rest.starts_with("OK") {
                    return Ok(Ok(lines));
                }
                return Ok(Err(Refusal::parse(rest, alert.as_deref())));
            }
            if alert.is_none() {
                alert = untagged_alert(&text).map(str::to_owned);
            }
            lines.push(ResponseLine { text, literals });
        }
    }

    fn command(&mut self, command: &str) -> CoreResult<Vec<ResponseLine>> {
        self.command_or_refusal(command)?.map_err(|refusal| {
            CoreError::ProviderFailure(format!("IMAP command refused: {refusal}"))
        })
    }
}

/// `MailProvider` over a live IMAP session. Message ids are
/// `{mailbox}/{uid}` so a hit can travel to an MCP client and come back
/// without extra state; mailbox names may contain slashes, so the id splits
/// at the last one.
pub struct ImapMailProvider<T: ImapTransport> {
    account_id: AccountId,
    client: RefCell<ImapClient<T>>,
}

impl<T: ImapTransport> ImapMailProvider<T> {
    pub fn new(account_id: AccountId, client: ImapClient<T>) -> Self {
        Self {
            account_id,
            client: RefCell::new(client),
        }
    }

    fn guard(&self, account_id: &AccountId) -> CoreResult<()> {
        if account_id == &self.account_id {
            Ok(())
        } else {
            Err(CoreError::AccountNotFound(account_id.clone()))
        }
    }

    fn split_message_id<'a>(&self, message_id: &'a str) -> CoreResult<(&'a str, u32)> {
        message_id
            .rsplit_once('/')
            .and_then(|(mailbox, uid)| Some((mailbox, uid.parse().ok()?)))
            .ok_or_else(|| CoreError::MessageNotFound(message_id.to_owned()))
    }
}

impl<T: ImapTransport> MailProvider for ImapMailProvider<T> {
    fn search(
        &self,
        account_id: &AccountId,
        query: &str,
        mailbox: Option<&str>,
        limit: usize,
        window: &SearchWindow,
    ) -> CoreResult<Vec<SearchHit>> {
        self.guard(account_id)?;
        let mut client = self.client.borrow_mut();
        // A search with neither text nor date means "the latest mail", and
        // the latest mail means the inbox: UIDs only rank messages within one
        // mailbox, so folding several together would order them by nothing at
        // all. A date-only search still has a criterion, so it fans out.
        let mailboxes = match mailbox {
            Some(name) => vec![name.to_owned()],
            None if query.trim().is_empty() && window.is_unbounded() => vec!["INBOX".to_owned()],
            None => client.list_mailboxes()?,
        };

        let mut hits = Vec::new();
        for mailbox in mailboxes {
            if hits.len() >= limit {
                break;
            }
            // Newest first: UIDs ascend with arrival, and a limit has to cut
            // off the oldest messages, not the ones the caller asked about.
            let mut uids = client.uid_search(&mailbox, query, window)?;
            uids.reverse();
            uids.truncate(limit - hits.len());
            // Headers only, and all of them in one round trip: the snippet a
            // full body would buy is not part of a search result, and one
            // command per message would pay the network once per hit.
            for summary in client.uid_fetch_summaries(&mailbox, &uids)? {
                hits.push(
                    SearchHit::new(
                        format!("{mailbox}/{}", summary.uid),
                        mailbox.clone(),
                        summary.subject,
                        summary.sender,
                        String::new(),
                    )
                    .with_date(summary.date),
                );
            }
        }
        Ok(hits)
    }

    fn get_message(&self, account_id: &AccountId, message_id: &str) -> CoreResult<StoredMessage> {
        self.guard(account_id)?;
        let (mailbox, uid) = self.split_message_id(message_id)?;
        let fetched = self.client.borrow_mut().uid_fetch(mailbox, uid)?;

        let preview = snippet(&fetched.body);
        let attachments = crate::mime::list_attachments(
            &fetched.raw_body,
            &fetched.content_type,
            &fetched.transfer_encoding,
        )
        .into_iter()
        .map(|part| {
            crate::AttachmentInfo::new(
                part.id,
                part.filename,
                part.media_type,
                part.size_bytes,
                part.inline,
            )
        })
        .collect();
        let mut message = StoredMessage::new(
            account_id.clone(),
            mailbox,
            message_id,
            // Plain IMAP has no thread identity; the message stands alone
            // until a threading extension arrives.
            message_id,
            fetched.subject,
            fetched.sender,
            preview,
            fetched.body,
        )
        .with_date(fetched.date)
        .with_attachment_infos(attachments);
        message.seen = fetched.seen;
        message.flagged = fetched.flagged;
        Ok(message)
    }

    fn mailbox_generation(&self, account_id: &AccountId, mailbox: &str) -> Option<u32> {
        if self.guard(account_id).is_err() {
            return None;
        }
        // Selecting is what teaches the client the value; reuse is free when
        // the mailbox is already selected.
        let mut client = self.client.borrow_mut();
        let _ = client.select(mailbox);
        client.uidvalidity(mailbox)
    }

    fn get_attachment(
        &self,
        account_id: &AccountId,
        message_id: &str,
        attachment_id: &str,
    ) -> CoreResult<AttachmentPayload> {
        self.guard(account_id)?;
        let (mailbox, uid) = self.split_message_id(message_id)?;
        // A fresh fetch rather than held state: the provider stays stateless
        // between calls, and the walk reads the same raw body the listing saw.
        let fetched = self.client.borrow_mut().uid_fetch(mailbox, uid)?;
        let (info, bytes) = crate::mime::extract_attachment_bytes(
            &fetched.raw_body,
            &fetched.content_type,
            &fetched.transfer_encoding,
            attachment_id,
        )
        .ok_or_else(|| CoreError::AttachmentNotFound {
            message_id: message_id.to_owned(),
            attachment_id: attachment_id.to_owned(),
        })?;
        Ok(AttachmentPayload::new(
            mailbox,
            info.filename,
            info.media_type,
            bytes,
        ))
    }

    /// The conversation `thread_id` (a `mailbox/uid`) belongs to, reconstructed
    /// from headers: the message-ids that identify it, and every message in the
    /// same mailbox that carries one of them in `Message-ID` or `References`.
    /// Threads that cross mailboxes are not chased — a UID names a mailbox, so
    /// staying in one keeps the ordering honest.
    fn get_thread(
        &self,
        account_id: &AccountId,
        thread_id: &str,
    ) -> CoreResult<Vec<StoredMessage>> {
        self.guard(account_id)?;
        let (mailbox, uid) = self.split_message_id(thread_id)?;

        // The ids that identify this conversation: the message's own, and the
        // ancestors it names.
        let mut ids;
        {
            let mut client = self.client.borrow_mut();
            let headers = client.uid_fetch_reference_headers(mailbox, uid)?;
            ids = headers.references;
            ids.extend(headers.in_reply_to);
            if let Some(message_id) = headers.message_id {
                ids.push(message_id);
            }
        }

        // Every uid that is one of those ids, or lists one among its
        // references. The message asked about is always in.
        let mut uids = BTreeSet::new();
        uids.insert(uid);
        {
            let mut client = self.client.borrow_mut();
            for id in &ids {
                for found in client.uid_search_header(mailbox, "Message-ID", id)? {
                    uids.insert(found);
                }
                for found in client.uid_search_header(mailbox, "References", id)? {
                    uids.insert(found);
                }
            }
        }

        // Oldest first: uids ascend with arrival. Fetch outside any client
        // borrow — `get_message` takes its own.
        let mut messages = Vec::with_capacity(uids.len());
        for found in uids {
            messages.push(self.get_message(account_id, &format!("{mailbox}/{found}"))?);
        }
        Ok(messages)
    }

    fn mark(
        &mut self,
        account_id: &AccountId,
        message_id: &str,
        change: MarkChange,
    ) -> CoreResult<()> {
        self.guard(account_id)?;
        let (mailbox, uid) = self.split_message_id(message_id)?;
        self.client.borrow_mut().uid_store(mailbox, uid, change)
    }

    fn list_mailboxes(&self, account_id: &AccountId) -> CoreResult<Vec<String>> {
        self.guard(account_id)?;
        self.client.borrow_mut().list_mailboxes()
    }

    fn append_draft(
        &mut self,
        account_id: &AccountId,
        mailbox: &str,
        message: &str,
    ) -> CoreResult<()> {
        self.guard(account_id)?;
        self.client.borrow_mut().append(mailbox, "\\Draft", message)
    }

    fn move_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
        target: &str,
    ) -> CoreResult<()> {
        self.guard(account_id)?;
        let mut client = self.client.borrow_mut();
        for message_id in message_ids {
            let (mailbox, uid) = self.split_message_id(message_id)?;
            client.uid_move(mailbox, uid, target)?;
        }
        Ok(())
    }

    fn expunge_messages(
        &mut self,
        account_id: &AccountId,
        message_ids: &[String],
    ) -> CoreResult<()> {
        self.guard(account_id)?;
        let mut client = self.client.borrow_mut();
        for message_id in message_ids {
            let (mailbox, uid) = self.split_message_id(message_id)?;
            client.uid_expunge(mailbox, uid)?;
        }
        Ok(())
    }
}

/// Buffered line/literal transport over any duplex byte stream — plaintext
/// TCP in development, TLS in production (`torromail-imap-tls`). Reads are
/// buffered; writes go straight through to the underlying stream.
pub struct StreamImapTransport<S: Read + Write> {
    stream: BufReader<S>,
}

impl<S: Read + Write> StreamImapTransport<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream: BufReader::new(stream),
        }
    }
}

impl<S: Read + Write> ImapTransport for StreamImapTransport<S> {
    fn send_line(&mut self, line: &str) -> CoreResult<()> {
        let writer = self.stream.get_mut();
        writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.write_all(b"\r\n"))
            .and_then(|()| writer.flush())
            .map_err(io_failure)
    }

    fn read_line(&mut self) -> CoreResult<String> {
        let mut line = String::new();
        let read = self.stream.read_line(&mut line).map_err(io_failure)?;
        if read == 0 {
            return Err(CoreError::ProviderFailure(
                "IMAP connection closed".to_owned(),
            ));
        }
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        Ok(line)
    }

    fn read_bytes(&mut self, count: usize) -> CoreResult<Vec<u8>> {
        let mut bytes = vec![0; count];
        self.stream.read_exact(&mut bytes).map_err(io_failure)?;
        Ok(bytes)
    }
}

/// Plaintext TCP — development against local test servers only. Real
/// accounts use the TLS transport in `torromail-imap-tls`.
pub type TcpImapTransport = StreamImapTransport<TcpStream>;

impl StreamImapTransport<TcpStream> {
    pub fn connect(host: &str, port: u16) -> CoreResult<Self> {
        let stream = TcpStream::connect((host, port)).map_err(io_failure)?;
        Ok(Self::new(stream))
    }
}

fn io_failure(error: std::io::Error) -> CoreError {
    CoreError::ProviderFailure(error.to_string())
}

/// IMAP quoted-string with the two escapes the grammar knows.
fn imap_quoted(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        if character == '"' || character == '\\' {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// The UIDs an untagged `* SEARCH` line carries. Shared by the plain search
/// and the header search — the reply format is the same.
fn collect_search_uids(lines: &[ResponseLine]) -> Vec<u32> {
    let mut uids = Vec::new();
    for line in lines {
        if let Some(rest) = line.text.strip_prefix("* SEARCH") {
            uids.extend(rest.split_whitespace().filter_map(|token| token.parse::<u32>().ok()));
        }
    }
    uids
}

/// The `<...>` message-ids in a header value. `Message-ID` holds one;
/// `References` holds a whitespace-separated list. Anything not in angle
/// brackets is ignored.
fn parse_message_ids(value: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            break;
        };
        let id = after[..close].trim();
        if !id.is_empty() {
            ids.push(id.to_owned());
        }
        rest = &after[close + 1..];
    }
    ids
}

/// `{N}` at the end of a line announces N literal bytes.
fn trailing_literal_size(text: &str) -> Option<usize> {
    let stripped = text.strip_suffix('}')?;
    let open = stripped.rfind('{')?;
    stripped[open + 1..].parse().ok()
}

/// `* LIST (\Flags) "/" "Name"` → `Name`.
/// `* 1 FETCH (UID 8 …` → 8.
fn parse_fetch_uid(text: &str) -> Option<u32> {
    let (_, after) = text.split_once("UID ")?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `* OK [UIDVALIDITY 123] UIDs valid` → 123.
fn parse_uidvalidity(text: &str) -> Option<u32> {
    let (_, after) = text.split_once("[UIDVALIDITY ")?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn parse_list_mailbox(text: &str) -> Option<String> {
    let rest = text.strip_prefix("* LIST ")?;
    let after_flags = rest.split_once(')')?.1.trim_start();
    let name_part = if let Some(quoted) = after_flags.strip_prefix('"') {
        quoted.split_once('"')?.1.trim_start()
    } else {
        after_flags.split_once(' ')?.1.trim_start()
    };
    Some(unquoted(name_part))
}

fn unquoted(value: &str) -> String {
    let trimmed = value.trim();
    let Some(inner) = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return trimmed.to_owned();
    };
    let mut result = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            result.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            result.push(character);
        }
    }
    result
}

/// First value of a header field in a HEADER.FIELDS block, case-insensitive,
/// unfolded and decoded.
///
/// Long headers are split across lines with their continuations indented
/// (RFC 5322 §2.2.3), so a subject read one line at a time silently loses
/// its tail.
fn header_field(headers: &str, name: &str) -> Option<String> {
    let mut lines = headers.lines();
    while let Some(line) = lines.next() {
        // Only a line starting at the margin names a field; an indented one
        // belongs to the field above.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        if !field.trim().eq_ignore_ascii_case(name) {
            continue;
        }

        let mut value = value.trim().to_owned();
        for continuation in lines.by_ref() {
            if !continuation.starts_with([' ', '\t']) {
                break;
            }
            value.push(' ');
            value.push_str(continuation.trim());
        }
        return Some(decode_encoded_words(&value));
    }
    None
}

/// RFC 2047 encoded-words to text.
///
/// Headers are ASCII on the wire, so anything else travels as
/// `=?charset?B?…?=` (base64) or `=?charset?Q?…?=` (quoted-printable). An
/// assistant handed the raw form sees noise where the subject should be.
/// Text that is not an encoded-word passes through untouched, and so does
/// one whose charset we cannot honour — a visibly encoded subject beats a
/// silently mangled one.
fn decode_encoded_words(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    let mut previous_was_encoded = false;

    while let Some(start) = rest.find("=?") {
        let (before, candidate) = rest.split_at(start);

        let Some((decoded, remainder)) = decode_one_encoded_word(candidate) else {
            // Not an encoded-word after all: it is literal text.
            result.push_str(before);
            result.push_str("=?");
            rest = &candidate["=?".len()..];
            previous_was_encoded = false;
            continue;
        };

        // Whitespace between two encoded-words separates them, it is not
        // content (RFC 2047 §6.2) — that is how one long subject folded
        // into several words joins back up without gaps.
        let is_separator = previous_was_encoded && !before.is_empty() && before.trim().is_empty();
        if !is_separator {
            result.push_str(before);
        }
        result.push_str(&decoded);
        rest = remainder;
        previous_was_encoded = true;
    }

    result.push_str(rest);
    result
}

/// One `=?charset?encoding?payload?=` at the start of `text`, plus whatever
/// follows it. `None` when this is not an encoded-word we can decode.
fn decode_one_encoded_word(text: &str) -> Option<(String, &str)> {
    let body = text.strip_prefix("=?")?;
    let (charset, rest) = body.split_once('?')?;
    let (encoding, rest) = rest.split_once('?')?;

    // The payload runs to the first `?`, which it cannot contain itself.
    // Scanning for the `?=` terminator directly would trip over a Q payload
    // that opens with an escape — `?Q?=F0=9F…` — and cut the word in half.
    let end = rest.find('?')?;
    let (payload, remainder) = rest.split_at(end);
    let remainder = remainder.strip_prefix("?=")?;

    let bytes = match encoding {
        "B" | "b" => crate::mime::decode_base64(payload)?,
        "Q" | "q" => decode_q_encoding(payload)?,
        _ => return None,
    };
    Some((crate::mime::decode_charset(charset, &bytes)?, remainder))
}

/// Standard base64, padded. Hand-rolled for the same reason the MIME decoders
/// are: torromail-core carries no dependencies, and this is twenty lines.
fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut buffer = [0u8; 3];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let packed = (u32::from(buffer[0]) << 16) | (u32::from(buffer[1]) << 8) | u32::from(buffer[2]);

        for index in 0..4 {
            // Each input byte carries into two output characters, so a
            // 1-byte tail fills 2 characters and a 2-byte tail fills 3; the
            // rest is padding.
            if index <= chunk.len() {
                let shift = 18 - index * 6;
                let value = ((packed >> shift) & 0x3F) as usize;
                encoded.push(char::from(ALPHABET[value]));
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

/// Quoted-printable as encoded-words use it: `=XX` hex escapes, plus the one
/// shorthand where `_` stands for a space.
fn decode_q_encoding(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut characters = text.chars();

    while let Some(character) = characters.next() {
        match character {
            '_' => bytes.push(b' '),
            '=' => {
                let high = characters.next()?.to_digit(16)?;
                let low = characters.next()?.to_digit(16)?;
                bytes.push((high * 16 + low) as u8);
            }
            _ => {
                let mut buffer = [0; 4];
                bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    Some(bytes)
}

fn snippet(body: &str) -> String {
    body.trim().chars().take(120).collect()
}

#[cfg(test)]
mod base64_tests {
    use super::encode_base64;

    #[test]
    fn encodes_the_rfc4648_vectors() {
        // RFC 4648 §10 — the padding cases are what a hand-rolled encoder
        // gets wrong, and XOAUTH2 blobs land on all three lengths.
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encodes_an_xoauth2_blob_with_its_control_bytes() {
        let blob = encode_base64(b"user=a@b.com\x01auth=Bearer tok\x01\x01");
        assert_eq!(blob, "dXNlcj1hQGIuY29tAWF1dGg9QmVhcmVyIHRvawEB");
    }
}
