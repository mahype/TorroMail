//! TLS transport for the IMAP protocol core.
//!
//! Kept out of `torromail-core` so the domain crate stays dependency-free —
//! everything here is wiring rustls onto the `ImapTransport` seam. Uses the
//! bundled Mozilla roots; custom or enterprise CAs are not honored yet, that
//! arrives with platform verification.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use torromail_core::smtp::{SmtpAuth, SmtpClient};
use torromail_core::{
    CoreError, CoreResult, ImapClient, ImapMailProvider, ImapProviderConfig, StreamImapTransport,
};

/// The TLS stream the transport runs over.
pub type TlsStream = StreamOwned<ClientConnection, TcpStream>;

/// A provider connected over implicit TLS, as `connect_account` builds it.
pub type TlsImapMailProvider = ImapMailProvider<StreamImapTransport<TlsStream>>;

/// An SMTP submission session over implicit TLS.
pub type TlsSmtpClient = SmtpClient<StreamImapTransport<TlsStream>>;

/// Implicit-TLS IMAP (usually port 993) with the bundled Mozilla roots.
pub fn connect_transport(host: &str, port: u16) -> CoreResult<StreamImapTransport<TlsStream>> {
    let tcp = TcpStream::connect((host, port)).map_err(|error| {
        CoreError::ProviderFailure(format!("connecting {host}:{port} failed: {error}"))
    })?;
    Ok(StreamImapTransport::new(tls_over(tcp, host)?))
}

/// Wrap an already-connected TCP stream in TLS, with the bundled Mozilla
/// roots. Shared by implicit TLS and the STARTTLS upgrade.
fn tls_over(tcp: TcpStream, host: &str) -> CoreResult<TlsStream> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|error| CoreError::ProviderFailure(format!("invalid host: {error}")))?;
    let connection = ClientConnection::new(Arc::new(config), server_name)
        .map_err(|error| CoreError::ProviderFailure(format!("TLS setup failed: {error}")))?;
    Ok(StreamOwned::new(connection, tcp))
}

/// One logged-in provider for a configured account: TLS, LOGIN, done. The
/// secret arrives already resolved — how it is resolved (keychain) is the
/// caller's business, never this crate's.
pub fn connect_account(
    config: &ImapProviderConfig,
    secret: &str,
) -> CoreResult<TlsImapMailProvider> {
    let transport = connect_transport(&config.host, config.port)?;
    let client = ImapClient::connect(transport, &config.username, secret)?;
    Ok(ImapMailProvider::new(config.account_id.clone(), client))
}

/// An authenticated SMTP submission session. Port 465 is implicit TLS; any
/// other port (typically 587) is plaintext with a STARTTLS upgrade before a
/// single byte of credentials moves. `ehlo_domain` is what the client
/// announces itself as; the secret and auth arrive already resolved.
pub fn connect_smtp(
    host: &str,
    port: u16,
    ehlo_domain: &str,
    auth: SmtpAuth,
) -> CoreResult<TlsSmtpClient> {
    if port == 465 {
        // Implicit TLS: the server sends a greeting first.
        SmtpClient::connect(connect_transport(host, port)?, ehlo_domain, auth)
    } else {
        // STARTTLS consumed the greeting before the upgrade, so the TLS
        // session opens straight at EHLO.
        SmtpClient::connect_upgraded(starttls(host, port, ehlo_domain)?, ehlo_domain, auth)
    }
}

/// The plaintext SMTP handshake up to STARTTLS, then the TLS upgrade. Raw
/// line I/O — no buffering — so no plaintext is read past the server's
/// go-ahead and lost when the socket becomes the TLS stream.
fn starttls(host: &str, port: u16, ehlo_domain: &str) -> CoreResult<StreamImapTransport<TlsStream>> {
    let mut tcp = TcpStream::connect((host, port)).map_err(|error| {
        CoreError::ProviderFailure(format!("connecting {host}:{port} failed: {error}"))
    })?;

    let greeting = read_line(&mut tcp)?;
    if !greeting.starts_with("220") {
        return Err(CoreError::ProviderFailure(format!(
            "unexpected SMTP greeting: {greeting}"
        )));
    }
    write_line(&mut tcp, &format!("EHLO {ehlo_domain}"))?;
    read_reply(&mut tcp)?;
    write_line(&mut tcp, "STARTTLS")?;
    let ready = read_line(&mut tcp)?;
    if !ready.starts_with("220") {
        return Err(CoreError::ProviderFailure(format!(
            "STARTTLS refused: {ready}"
        )));
    }

    Ok(StreamImapTransport::new(tls_over(tcp, host)?))
}

/// One CRLF-terminated line, read a byte at a time so nothing after it is
/// consumed.
fn read_line(stream: &mut TcpStream) -> CoreResult<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let read = stream
            .read(&mut byte)
            .map_err(|error| CoreError::ProviderFailure(error.to_string()))?;
        if read == 0 || byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            line.push(byte[0]);
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

fn write_line(stream: &mut TcpStream, line: &str) -> CoreResult<()> {
    stream
        .write_all(line.as_bytes())
        .and_then(|()| stream.write_all(b"\r\n"))
        .and_then(|()| stream.flush())
        .map_err(|error| CoreError::ProviderFailure(error.to_string()))
}

/// Read a multiline SMTP reply, discarding the continuation lines.
fn read_reply(stream: &mut TcpStream) -> CoreResult<()> {
    loop {
        let line = read_line(stream)?;
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok(());
        }
    }
}
