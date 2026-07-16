//! TLS transport for the IMAP protocol core.
//!
//! Kept out of `torromail-core` so the domain crate stays dependency-free —
//! everything here is wiring rustls onto the `ImapTransport` seam. Uses the
//! bundled Mozilla roots; custom or enterprise CAs are not honored yet, that
//! arrives with platform verification.

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
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|error| CoreError::ProviderFailure(format!("invalid IMAP host: {error}")))?;
    let connection = ClientConnection::new(Arc::new(config), server_name)
        .map_err(|error| CoreError::ProviderFailure(format!("TLS setup failed: {error}")))?;
    let tcp = TcpStream::connect((host, port)).map_err(|error| {
        CoreError::ProviderFailure(format!("connecting {host}:{port} failed: {error}"))
    })?;

    Ok(StreamImapTransport::new(StreamOwned::new(connection, tcp)))
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

/// An authenticated SMTP submission session over implicit TLS (usually port
/// 465). `ehlo_domain` is what the client announces itself as; the secret and
/// auth arrive already resolved.
pub fn connect_smtp(
    host: &str,
    port: u16,
    ehlo_domain: &str,
    auth: SmtpAuth,
) -> CoreResult<TlsSmtpClient> {
    let transport = connect_transport(host, port)?;
    SmtpClient::connect(transport, ehlo_domain, auth)
}
