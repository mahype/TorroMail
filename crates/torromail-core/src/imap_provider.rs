//! Configuration boundary for the real IMAP provider.
//!
//! Connection facts live here; the secret itself never does. `SecretRef`
//! points into the platform keychain and refuses to reveal itself in any
//! Debug output, so configs can travel through logs and diagnostics safely.

use std::cell::RefCell;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use crate::{AccountId, CoreError, CoreResult, MailProvider, MarkChange, SearchHit, StoredMessage};

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
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretRef(<redacted>)")
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
        }
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
    pub body: String,
    pub seen: bool,
    pub flagged: bool,
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
    /// Reads the greeting and logs in.
    pub fn connect(transport: T, username: &str, secret: &str) -> CoreResult<Self> {
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
        client.command(&format!(
            "LOGIN {} {}",
            imap_quoted(username),
            imap_quoted(secret)
        ))?;
        Ok(client)
    }

    pub fn list_mailboxes(&mut self) -> CoreResult<Vec<String>> {
        let lines = self.command("LIST \"\" \"*\"")?;
        Ok(lines
            .iter()
            .filter_map(|line| parse_list_mailbox(&line.text))
            .collect())
    }

    pub fn uid_search(&mut self, mailbox: &str, query: &str) -> CoreResult<Vec<u32>> {
        self.select(mailbox)?;
        let lines = self.command(&format!("UID SEARCH TEXT {}", imap_quoted(query)))?;
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
            "UID FETCH {uid} (UID FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT FROM)] BODY.PEEK[TEXT])"
        ))?;
        let fetch = lines
            .iter()
            .find(|line| line.text.contains("FETCH"))
            .ok_or_else(|| {
                CoreError::ProviderFailure(format!("no FETCH response for uid {uid}"))
            })?;

        // Literals arrive in the order the command asked: the header block
        // first, then the text.
        let headers = fetch
            .literals
            .first()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        let body = fetch
            .literals
            .get(1)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();

        Ok(FetchedMessage {
            uid,
            subject: header_field(&headers, "subject").unwrap_or_default(),
            sender: header_field(&headers, "from").unwrap_or_default(),
            body,
            seen: fetch.text.contains("\\Seen"),
            flagged: fetch.text.contains("\\Flagged"),
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
    ) -> CoreResult<Vec<SearchHit>> {
        self.guard(account_id)?;
        let mut client = self.client.borrow_mut();
        let mailboxes = match mailbox {
            Some(name) => vec![name.to_owned()],
            None => client.list_mailboxes()?,
        };

        let mut hits = Vec::new();
        for mailbox in mailboxes {
            if hits.len() >= limit {
                break;
            }
            for uid in client.uid_search(&mailbox, query)? {
                if hits.len() >= limit {
                    break;
                }
                let message = client.uid_fetch(&mailbox, uid)?;
                hits.push(SearchHit::new(
                    format!("{mailbox}/{uid}"),
                    mailbox.clone(),
                    message.subject,
                    message.sender,
                    snippet(&message.body),
                ));
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
        );
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

/// Plaintext TCP transport — development against local test servers only.
/// Real accounts wait for the TLS transport; `ImapProviderConfig` already
/// carries the connection facts it will need.
pub struct TcpImapTransport {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl TcpImapTransport {
    pub fn connect(host: &str, port: u16) -> CoreResult<Self> {
        let stream = TcpStream::connect((host, port)).map_err(io_failure)?;
        let reader = BufReader::new(stream.try_clone().map_err(io_failure)?);
        Ok(Self {
            reader,
            writer: stream,
        })
    }
}

impl ImapTransport for TcpImapTransport {
    fn send_line(&mut self, line: &str) -> CoreResult<()> {
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\r\n"))
            .and_then(|_| self.writer.flush())
            .map_err(io_failure)
    }

    fn read_line(&mut self) -> CoreResult<String> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).map_err(io_failure)?;
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
        self.reader.read_exact(&mut bytes).map_err(io_failure)?;
        Ok(bytes)
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

/// First value of a header field in a HEADER.FIELDS block, case-insensitive.
fn header_field(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (field, value) = line.split_once(':')?;
        if field.trim().eq_ignore_ascii_case(name) {
            Some(value.trim().to_owned())
        } else {
            None
        }
    })
}

fn snippet(body: &str) -> String {
    body.trim().chars().take(120).collect()
}
