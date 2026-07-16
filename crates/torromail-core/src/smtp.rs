//! The SMTP submission core: EHLO, AUTH, MAIL/RCPT/DATA.
//!
//! Like the IMAP core, this speaks the protocol over a line transport it does
//! not own, so the whole exchange is testable against a scripted server with
//! no network. TLS wiring lives in `torromail-imap-tls`.

use std::io::{Read, Write};

use crate::imap_provider::StreamImapTransport;
use crate::{CoreError, CoreResult, ImapTransport};

/// The lines an SMTP session runs over. Deliberately the same two operations
/// the IMAP transport already offers, so the TLS stream serves both.
pub trait SmtpTransport {
    fn send_line(&mut self, line: &str) -> CoreResult<()>;
    fn read_line(&mut self) -> CoreResult<String>;
}

/// The TLS stream that carries an IMAP session carries an SMTP one just as
/// well — both are CRLF line protocols.
impl<S: Read + Write> SmtpTransport for StreamImapTransport<S> {
    fn send_line(&mut self, line: &str) -> CoreResult<()> {
        ImapTransport::send_line(self, line)
    }

    fn read_line(&mut self) -> CoreResult<String> {
        ImapTransport::read_line(self)
    }
}

/// How to authenticate a submission session.
pub enum SmtpAuth {
    /// `AUTH LOGIN` with a password.
    Login { username: String, secret: String },
    /// `AUTH XOAUTH2` with a bearer access token.
    XOAuth2 {
        username: String,
        access_token: String,
    },
}

/// One authenticated SMTP submission session.
pub struct SmtpClient<T: SmtpTransport> {
    transport: T,
}

impl<T: SmtpTransport> SmtpClient<T> {
    /// Read the greeting, greet back, and authenticate — implicit TLS, where
    /// the server speaks first.
    pub fn connect(transport: T, ehlo_domain: &str, auth: SmtpAuth) -> CoreResult<Self> {
        let mut client = Self { transport };

        let greeting = client.transport.read_line()?;
        if !greeting.starts_with("220") {
            return Err(CoreError::ProviderFailure(format!(
                "unexpected SMTP greeting: {greeting}"
            )));
        }
        client.handshake(ehlo_domain, auth)?;
        Ok(client)
    }

    /// Greet and authenticate on a connection whose greeting was already read
    /// — the state right after a STARTTLS upgrade, where the server waits for
    /// EHLO and sends no fresh greeting. Reading for one here would hang.
    pub fn connect_upgraded(transport: T, ehlo_domain: &str, auth: SmtpAuth) -> CoreResult<Self> {
        let mut client = Self { transport };
        client.handshake(ehlo_domain, auth)?;
        Ok(client)
    }

    fn handshake(&mut self, ehlo_domain: &str, auth: SmtpAuth) -> CoreResult<()> {
        self.transport.send_line(&format!("EHLO {ehlo_domain}"))?;
        self.read_reply("250")?;
        self.authenticate(auth)
    }

    /// One message to one or more recipients. The envelope sender and the
    /// recipients are the SMTP addresses; the message is the full RFC 5322
    /// text, dot-stuffed as the protocol requires.
    pub fn send_message(
        &mut self,
        from: &str,
        recipients: &[String],
        message: &str,
    ) -> CoreResult<()> {
        if recipients.is_empty() {
            return Err(CoreError::ProviderFailure(
                "a message needs at least one recipient".to_owned(),
            ));
        }

        self.transport.send_line(&format!("MAIL FROM:<{from}>"))?;
        self.read_reply("250")?;
        for recipient in recipients {
            self.transport.send_line(&format!("RCPT TO:<{recipient}>"))?;
            // 250 accepted, 251 will-forward — both are a yes.
            self.read_reply("25")?;
        }

        self.transport.send_line("DATA")?;
        self.read_reply("354")?;
        for line in message.split("\r\n") {
            // Dot-stuffing: a line that begins with a dot gets a second one so
            // it cannot be read as the end-of-data marker.
            if line.starts_with('.') {
                self.transport.send_line(&format!(".{line}"))?;
            } else {
                self.transport.send_line(line)?;
            }
        }
        self.transport.send_line(".")?;
        self.read_reply("250")?;
        Ok(())
    }

    /// End the session politely. A failure here does not undo a sent message,
    /// so it is not worth surfacing.
    pub fn quit(&mut self) {
        let _ = self.transport.send_line("QUIT");
    }

    fn authenticate(&mut self, auth: SmtpAuth) -> CoreResult<()> {
        match auth {
            SmtpAuth::Login { username, secret } => {
                self.transport.send_line("AUTH LOGIN")?;
                self.read_reply("334")?;
                self.transport
                    .send_line(&crate::mime::encode_base64(username.as_bytes()))?;
                self.read_reply("334")?;
                self.transport
                    .send_line(&crate::mime::encode_base64(secret.as_bytes()))?;
                self.read_reply("235")?;
            }
            SmtpAuth::XOAuth2 {
                username,
                access_token,
            } => {
                let blob = crate::mime::encode_base64(
                    format!("user={username}\x01auth=Bearer {access_token}\x01\x01").as_bytes(),
                );
                self.transport.send_line(&format!("AUTH XOAUTH2 {blob}"))?;

                // Success is 235. A rejection comes as 334 with a base64 error
                // and, like IMAP, waits for an empty line before it will send
                // the final failure.
                let reply = self.transport.read_line()?;
                if reply.starts_with("235") {
                    return Ok(());
                }
                if reply.starts_with("334") {
                    self.transport.send_line("")?;
                    let failure = self.transport.read_line()?;
                    return Err(CoreError::ProviderFailure(format!(
                        "SMTP XOAUTH2 rejected: {failure}"
                    )));
                }
                return Err(CoreError::ProviderFailure(format!(
                    "SMTP AUTH failed: {reply}"
                )));
            }
        }
        Ok(())
    }

    /// Read a reply, skipping continuation lines, and check its status code.
    /// A reply is one or more lines all carrying the code; a `-` right after
    /// the code marks a line as continued, a space marks the last.
    fn read_reply(&mut self, expected_prefix: &str) -> CoreResult<String> {
        loop {
            let line = self.transport.read_line()?;
            let continued = line.as_bytes().get(3) == Some(&b'-');
            if continued {
                continue;
            }
            if !line.starts_with(expected_prefix) {
                return Err(CoreError::ProviderFailure(format!(
                    "SMTP expected {expected_prefix}, got: {line}"
                )));
            }
            return Ok(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;

    /// A scripted transport: canned replies out, sent lines recorded.
    struct Scripted {
        incoming: VecDeque<String>,
        sent: Rc<RefCell<Vec<String>>>,
    }

    impl Scripted {
        fn new(script: &[&str], sent: Rc<RefCell<Vec<String>>>) -> Self {
            Self {
                incoming: script.iter().map(|line| (*line).to_owned()).collect(),
                sent,
            }
        }
    }

    impl SmtpTransport for Scripted {
        fn send_line(&mut self, line: &str) -> CoreResult<()> {
            self.sent.borrow_mut().push(line.to_owned());
            Ok(())
        }
        fn read_line(&mut self) -> CoreResult<String> {
            self.incoming
                .pop_front()
                .ok_or_else(|| CoreError::ProviderFailure("script exhausted".to_owned()))
        }
    }

    #[test]
    fn a_login_submission_walks_the_whole_exchange() {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let transport = Scripted::new(
            &[
                "220 smtp.example.com ready",
                "250-smtp.example.com",
                "250 AUTH LOGIN",
                "334 VXNlcm5hbWU6",
                "334 UGFzc3dvcmQ6",
                "235 authenticated",
                "250 sender ok",
                "250 recipient ok",
                "354 go ahead",
                "250 queued",
            ],
            sent.clone(),
        );

        let mut client = SmtpClient::connect(
            transport,
            "torro.local",
            SmtpAuth::Login {
                username: "me@example.com".to_owned(),
                secret: "secret".to_owned(),
            },
        )
        .expect("connect + auth");
        client
            .send_message(
                "me@example.com",
                &["you@example.com".to_owned()],
                "Subject: Hi\r\n\r\nHello.",
            )
            .expect("send");
        client.quit();

        let sent = sent.borrow().clone();
        assert_eq!(sent[0], "EHLO torro.local");
        assert_eq!(sent[1], "AUTH LOGIN");
        assert_eq!(sent[2], "bWVAZXhhbXBsZS5jb20="); // base64("me@example.com")
        assert_eq!(sent[3], "c2VjcmV0"); // base64("secret")
        assert_eq!(sent[4], "MAIL FROM:<me@example.com>");
        assert_eq!(sent[5], "RCPT TO:<you@example.com>");
        assert_eq!(sent[6], "DATA");
        assert_eq!(sent[sent.len() - 2], ".");
        assert_eq!(sent[sent.len() - 1], "QUIT");
    }

    #[test]
    fn a_dot_line_in_the_body_is_stuffed() {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let transport = Scripted::new(
            &[
                "220 ready",
                "250 ok",
                "235 ok",
                "250 ok",
                "250 ok",
                "354 ok",
                "250 queued",
            ],
            sent.clone(),
        );
        let mut client = SmtpClient::connect(
            transport,
            "torro.local",
            SmtpAuth::XOAuth2 {
                username: "me@example.com".to_owned(),
                access_token: "tok".to_owned(),
            },
        )
        .expect("connect");
        client
            .send_message(
                "me@example.com",
                &["you@example.com".to_owned()],
                "line one\r\n.hidden\r\nlast",
            )
            .expect("send");

        // The body line ".hidden" travelled as "..hidden".
        assert!(sent.borrow().iter().any(|line| line == "..hidden"));
    }

    #[test]
    fn an_upgraded_session_does_not_wait_for_a_second_greeting() {
        // After STARTTLS the server sends no greeting — the script starts at
        // the EHLO reply. Reading for a greeting here would block forever.
        let sent = Rc::new(RefCell::new(Vec::new()));
        let transport = Scripted::new(
            &[
                "250-mail.example.com",
                "250 AUTH LOGIN",
                "334 VXNlcm5hbWU6",
                "334 UGFzc3dvcmQ6",
                "235 authenticated",
            ],
            sent.clone(),
        );

        let client = SmtpClient::connect_upgraded(
            transport,
            "torro.local",
            SmtpAuth::Login {
                username: "me@example.com".to_owned(),
                secret: "secret".to_owned(),
            },
        );
        assert!(client.is_ok(), "upgraded connect authenticates");
        assert_eq!(sent.borrow()[0], "EHLO torro.local");
    }

    #[test]
    fn a_rejected_recipient_surfaces_the_server_reply() {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let transport = Scripted::new(
            &[
                "220 ready",
                "250 ok",
                "334 user",
                "334 pass",
                "235 ok",
                "250 sender ok",
                "550 no such user",
            ],
            sent.clone(),
        );
        let mut client = SmtpClient::connect(
            transport,
            "torro.local",
            SmtpAuth::Login {
                username: "me@example.com".to_owned(),
                secret: "s".to_owned(),
            },
        )
        .expect("connect");

        let result = client.send_message(
            "me@example.com",
            &["ghost@example.com".to_owned()],
            "Subject: x\r\n\r\nhi",
        );
        let error = match result {
            Ok(()) => panic!("a rejected recipient must fail the send"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("550"), "got: {error}");
    }
}
