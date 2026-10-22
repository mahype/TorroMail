//! TLS transport for the IMAP protocol core.
//!
//! Kept out of `torromail-core` so the domain crate stays dependency-free —
//! everything here is wiring rustls onto the `ImapTransport` seam. Uses the
//! bundled Mozilla roots; custom or enterprise CAs are not honored yet, that
//! arrives with platform verification. Until then — and for servers no CA
//! will ever vouch for, like a local Proton Mail Bridge — a certificate the
//! user chose to trust is accepted by its SHA-256 fingerprint.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore,
    SignatureScheme, StreamOwned,
};
use sha2::{Digest, Sha256};
use torromail_core::smtp::{SmtpAuth, SmtpClient};
use torromail_core::{
    ConnectionSecurity, CoreError, CoreResult, ImapClient, ImapMailProvider, ImapProviderConfig,
    StreamImapTransport, UntrustedCertificate,
};

/// The TLS stream the transport runs over.
pub type TlsStream = StreamOwned<ClientConnection, TcpStream>;

/// A provider connected over implicit TLS, as `connect_account` builds it.
pub type TlsImapMailProvider = ImapMailProvider<StreamImapTransport<TlsStream>>;

/// An SMTP submission session over implicit TLS.
pub type TlsSmtpClient = SmtpClient<StreamImapTransport<TlsStream>>;

/// Implicit-TLS IMAP (usually port 993) with the bundled Mozilla roots.
pub fn connect_transport(host: &str, port: u16) -> CoreResult<StreamImapTransport<TlsStream>> {
    connect_trusting(host, port, &[])
}

/// Implicit TLS, also accepting the certificates the user chose to trust (by
/// SHA-256 fingerprint). A refused certificate answers
/// `CoreError::UntrustedCertificate`, carrying the fingerprint to decide on.
pub fn connect_trusting(
    host: &str,
    port: u16,
    trusted: &[String],
) -> CoreResult<StreamImapTransport<TlsStream>> {
    let tcp = TcpStream::connect((host, port)).map_err(|error| {
        CoreError::ProviderFailure(format!("connecting {host}:{port} failed: {error}"))
    })?;
    Ok(StreamImapTransport::new(tls_over(tcp, host, port, trusted)?))
}

/// Wrap an already-connected TCP stream in TLS, with the bundled Mozilla
/// roots plus the user's trusted fingerprints. Shared by implicit TLS and the
/// STARTTLS upgrade.
///
/// The handshake runs here, before anything is read or written: left to the
/// first read, a refused certificate would surface as an anonymous I/O error
/// somewhere in the protocol code, and "the certificate is not trusted" could
/// no longer be told apart from "the network dropped".
fn tls_over(
    mut tcp: TcpStream,
    host: &str,
    port: u16,
    trusted: &[String],
) -> CoreResult<TlsStream> {
    let verifier = Arc::new(TrustingVerifier::new(trusted)?);
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();

    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|error| CoreError::ProviderFailure(format!("invalid host: {error}")))?;
    let mut connection = ClientConnection::new(Arc::new(config), server_name)
        .map_err(|error| CoreError::ProviderFailure(format!("TLS setup failed: {error}")))?;

    while connection.is_handshaking() {
        if let Err(error) = connection.complete_io(&mut tcp) {
            if let Some((reason, sha256)) = verifier.refused() {
                return Err(CoreError::UntrustedCertificate(UntrustedCertificate {
                    server: format!("{host}:{port}"),
                    reason,
                    sha256,
                }));
            }
            return Err(CoreError::ProviderFailure(format!(
                "TLS handshake with {host}:{port} failed: {error}"
            )));
        }
    }
    Ok(StreamOwned::new(connection, tcp))
}

/// Normal web PKI verification first; only a certificate it refuses is
/// compared against the fingerprints the user trusts. So a trusted entry never
/// weakens a server whose certificate is valid anyway, and a refused one is
/// remembered — reason and fingerprint — for the error the user decides on.
#[derive(Debug)]
struct TrustingVerifier {
    inner: Arc<WebPkiServerVerifier>,
    trusted: Vec<String>,
    refused: Mutex<Option<(String, String)>>,
}

impl TrustingVerifier {
    fn new(trusted: &[String]) -> CoreResult<Self> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let inner = WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .map_err(|error| CoreError::ProviderFailure(format!("TLS setup failed: {error}")))?;
        Ok(Self {
            inner,
            trusted: trusted.iter().map(|entry| normalize_fingerprint(entry)).collect(),
            refused: Mutex::new(None),
        })
    }

    fn refused(&self) -> Option<(String, String)> {
        self.refused.lock().ok().and_then(|slot| slot.clone())
    }
}

impl ServerCertVerifier for TrustingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let refusal = match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => return Ok(verified),
            Err(rustls::Error::InvalidCertificate(refusal)) => refusal,
            Err(other) => return Err(other),
        };

        let sha256 = certificate_fingerprint(end_entity.as_ref());
        if self.trusted.contains(&sha256) {
            return Ok(ServerCertVerified::assertion());
        }
        if let Ok(mut slot) = self.refused.lock() {
            *slot = Some((reason_word(&refusal).to_owned(), sha256));
        }
        Err(rustls::Error::InvalidCertificate(refusal))
    }

    // The handshake signatures are checked against the presented certificate
    // either way: trusting a certificate is trusting its key, not skipping the
    // proof that the server holds it.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// SHA-256 of a DER certificate, lowercase hex without separators — the same
/// number `openssl x509 -fingerprint -sha256` prints with colons.
#[must_use]
pub fn certificate_fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Separators and case are presentation; a fingerprint pasted with colons or
/// in capitals is still the same certificate.
fn normalize_fingerprint(entry: &str) -> String {
    entry
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// Why verification refused a certificate, as the one word the surfaces
/// translate. A self-signed certificate arrives as an unknown issuer — or, when
/// it is marked as a CA the way local bridges generate theirs, as webpki's
/// `CaUsedAsEndEntity`, which only reaches us wrapped in `Other`.
fn reason_word(error: &CertificateError) -> &'static str {
    match error {
        CertificateError::UnknownIssuer => "unknown_issuer",
        CertificateError::Other(other) if format!("{other:?}").contains("CaUsedAsEndEntity") => {
            "unknown_issuer"
        }
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => "expired",
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
            "not_yet_valid"
        }
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
            "wrong_name"
        }
        _ => "invalid",
    }
}

/// One logged-in provider for a configured account: TLS, login, done. The
/// secret arrives already resolved — how it is resolved (keychain) is the
/// caller's business, never this crate's.
///
/// Which TLS is a fact carried by the config, not a guess from the port: a
/// server offering implicit TLS on an unusual port used to be unreachable
/// because 993 was the only thing that counted as "secure".
pub fn connect_account(
    config: &ImapProviderConfig,
    secret: &str,
) -> CoreResult<TlsImapMailProvider> {
    let client = match config.security {
        ConnectionSecurity::Tls => ImapClient::connect_with(
            connect_trusting(&config.host, config.port, &config.trusted_certificates)?,
            &config.username,
            secret,
            config.auth,
        )?,
        // The greeting was read in the clear before the upgrade, so the TLS
        // session opens straight at the login.
        ConnectionSecurity::StartTls => ImapClient::connect_upgraded_with(
            imap_starttls(&config.host, config.port, &config.trusted_certificates)?,
            &config.username,
            secret,
            config.auth,
        )?,
    };
    Ok(ImapMailProvider::new(config.account_id.clone(), client))
}

/// The plaintext IMAP greeting, then `STARTTLS`, then the upgrade. Raw line
/// I/O for the same reason as the SMTP one below: nothing may be buffered
/// past the server's go-ahead, or it is lost when the socket becomes TLS.
fn imap_starttls(
    host: &str,
    port: u16,
    trusted: &[String],
) -> CoreResult<StreamImapTransport<TlsStream>> {
    let mut tcp = TcpStream::connect((host, port)).map_err(|error| {
        CoreError::ProviderFailure(format!("connecting {host}:{port} failed: {error}"))
    })?;

    let greeting = read_line(&mut tcp)?;
    if !greeting.starts_with("* OK") {
        return Err(CoreError::ProviderFailure(format!(
            "unexpected IMAP greeting: {greeting}"
        )));
    }

    write_line(&mut tcp, "t0 STARTTLS")?;
    // Untagged chatter may precede the verdict; only the tagged line counts.
    loop {
        let line = read_line(&mut tcp)?;
        if let Some(rest) = line.strip_prefix("t0 ") {
            if rest.starts_with("OK") {
                break;
            }
            return Err(CoreError::ProviderFailure(format!(
                "STARTTLS refused: {rest}"
            )));
        }
        if line.is_empty() {
            return Err(CoreError::ProviderFailure(
                "the server closed the connection during STARTTLS".to_owned(),
            ));
        }
    }

    Ok(StreamImapTransport::new(tls_over(tcp, host, port, trusted)?))
}

/// An authenticated SMTP submission session. `security` says which TLS the
/// server speaks; STARTTLS upgrades before a single byte of credentials
/// moves. `ehlo_domain` is what the client announces itself as; the secret
/// and auth arrive already resolved. `trusted` are the certificate
/// fingerprints the user accepted for this account.
pub fn connect_smtp(
    host: &str,
    port: u16,
    security: ConnectionSecurity,
    trusted: &[String],
    ehlo_domain: &str,
    auth: SmtpAuth,
) -> CoreResult<TlsSmtpClient> {
    match security {
        // Implicit TLS: the server sends a greeting first.
        ConnectionSecurity::Tls => {
            SmtpClient::connect(connect_trusting(host, port, trusted)?, ehlo_domain, auth)
        }
        // STARTTLS consumed the greeting before the upgrade, so the TLS
        // session opens straight at EHLO.
        ConnectionSecurity::StartTls => SmtpClient::connect_upgraded(
            smtp_starttls(host, port, trusted, ehlo_domain)?,
            ehlo_domain,
            auth,
        ),
    }
}

/// The plaintext SMTP handshake up to STARTTLS, then the TLS upgrade. Raw
/// line I/O — no buffering — so no plaintext is read past the server's
/// go-ahead and lost when the socket becomes the TLS stream.
fn smtp_starttls(
    host: &str,
    port: u16,
    trusted: &[String],
    ehlo_domain: &str,
) -> CoreResult<StreamImapTransport<TlsStream>> {
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

    Ok(StreamImapTransport::new(tls_over(tcp, host, port, trusted)?))
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
