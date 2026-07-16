//! Configuration boundary for the real IMAP provider.
//!
//! Connection facts live here; the secret itself never does. `SecretRef`
//! points into the platform keychain and refuses to reveal itself in any
//! Debug output, so configs can travel through logs and diagnostics safely.

use std::cell::RefCell;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use crate::{
    AccountId, CoreError, CoreResult, MailProvider, MarkChange, SearchHit, SearchWindow,
    StoredMessage,
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
        }
    }

    #[must_use]
    pub fn with_auth(mut self, auth: ImapAuth) -> Self {
        self.auth = auth;
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

/// One untagged response line, with any literals it carried in order.
struct ResponseLine {
    text: String,
    literals: Vec<Vec<u8>>,
}

/// One logged-in IMAP session speaking the smallest useful IMAP4rev1
/// subset: LIST, SELECT, UID SEARCH, UID FETCH, UID STORE. Bodies are
/// fetched with PEEK — reading never sets flags; changing flags is what the
/// mark permission is for.
pub struct ImapClient<T: ImapTransport> {
    transport: T,
    next_tag: u32,
    selected: Option<String>,
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
        };
        let greeting = client.transport.read_line()?;
        if !greeting.starts_with("* OK") {
            return Err(CoreError::ProviderFailure(format!(
                "unexpected IMAP greeting: {greeting}"
            )));
        }

        match auth {
            ImapAuth::Password => {
                client.command(&format!(
                    "LOGIN {} {}",
                    imap_quoted(username),
                    imap_quoted(secret)
                ))?;
            }
            ImapAuth::XOAuth2 => client.authenticate_xoauth2(username, secret)?,
        }
        Ok(client)
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

        loop {
            let line = self.transport.read_line()?;

            if let Some(rest) = line.strip_prefix(&format!("{tag} ")) {
                if rest.starts_with("OK") {
                    return Ok(());
                }
                return Err(CoreError::ProviderFailure(format!(
                    "IMAP rejected the access token: {rest}"
                )));
            }

            if line.starts_with('+') {
                // The rejection detail, and a server waiting on us. `command`
                // cannot be used for this exchange precisely because it would
                // sit here waiting for a tagged line that only arrives after
                // this empty reply.
                self.transport.send_line("")?;
                continue;
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
        let mut uids = Vec::new();
        for line in &lines {
            if let Some(rest) = line.text.strip_prefix("* SEARCH") {
                for token in rest.split_whitespace() {
                    if let Ok(uid) = token.parse() {
                        uids.push(uid);
                    }
                }
            }
        }
        Ok(uids)
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
        // has to know MIME.
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
            seen: fetch.text.contains("\\Seen"),
            flagged: fetch.text.contains("\\Flagged"),
        })
    }

    /// Just the headers a search hit shows — no `BODY[TEXT]`, so a search over
    /// many mailboxes stops pulling every match in full.
    pub fn uid_fetch_summary(&mut self, mailbox: &str, uid: u32) -> CoreResult<FetchedSummary> {
        self.select(mailbox)?;
        let lines = self.command(&format!(
            "UID FETCH {uid} (UID BODY.PEEK[HEADER.FIELDS (SUBJECT FROM DATE)])"
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

        Ok(FetchedSummary {
            uid,
            subject: header_field(&headers, "subject").unwrap_or_default(),
            sender: header_field(&headers, "from").unwrap_or_default(),
            date: header_field(&headers, "date").unwrap_or_default(),
        })
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

    fn select(&mut self, mailbox: &str) -> CoreResult<()> {
        if self.selected.as_deref() == Some(mailbox) {
            return Ok(());
        }
        self.command(&format!("SELECT {}", imap_quoted(mailbox)))?;
        self.selected = Some(mailbox.to_owned());
        Ok(())
    }

    fn command(&mut self, command: &str) -> CoreResult<Vec<ResponseLine>> {
        self.next_tag += 1;
        let tag = format!("t{}", self.next_tag);
        self.transport.send_line(&format!("{tag} {command}"))?;

        let mut lines = Vec::new();
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
                    return Ok(lines);
                }
                return Err(CoreError::ProviderFailure(format!(
                    "IMAP command refused: {rest}"
                )));
            }
            lines.push(ResponseLine { text, literals });
        }
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
            for uid in uids {
                if hits.len() >= limit {
                    break;
                }
                // Headers only: the snippet a full body would buy is not part
                // of a search result, so paying for the body is pure waste.
                let summary = client.uid_fetch_summary(&mailbox, uid)?;
                hits.push(
                    SearchHit::new(
                        format!("{mailbox}/{uid}"),
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
        .with_date(fetched.date);
        message.seen = fetched.seen;
        message.flagged = fetched.flagged;
        Ok(message)
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

/// `{N}` at the end of a line announces N literal bytes.
fn trailing_literal_size(text: &str) -> Option<usize> {
    let stripped = text.strip_suffix('}')?;
    let open = stripped.rfind('{')?;
    stripped[open + 1..].parse().ok()
}

/// `* LIST (\Flags) "/" "Name"` → `Name`.
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
