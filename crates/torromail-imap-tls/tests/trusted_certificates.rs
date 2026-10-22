//! Certificates the user chose to trust, against a local server presenting a
//! self-signed certificate — the shape of a Proton Mail Bridge on 127.0.0.1.
//! The fixture is marked `CA:TRUE`, as such bridges generate theirs.

use std::net::TcpListener;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection};
use torromail_core::{CoreError, ImapTransport};

const CERTIFICATE: &[u8] = include_bytes!("fixtures/self-signed.der");
const KEY: &[u8] = include_bytes!("fixtures/self-signed.key.der");
const FINGERPRINT: &str = "9d703534b1e0af0b80951afeb9b5522fb0dc9b599464d8dc6d6f3b5aaae1f1ba";

/// One implicit-TLS connection: handshake, a greeting, done. A refused
/// handshake simply ends the thread.
fn serve_once() -> u16 {
    let config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("protocol versions")
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(CERTIFICATE.to_vec())],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())),
    )
    .expect("server certificate");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    std::thread::spawn(move || {
        let (mut tcp, _) = listener.accept().expect("accept");
        let mut connection = ServerConnection::new(Arc::new(config)).expect("server connection");
        while connection.is_handshaking() {
            if connection.complete_io(&mut tcp).is_err() {
                return;
            }
        }
        let mut stream = rustls::StreamOwned::new(connection, tcp);
        let _ = std::io::Write::write_all(&mut stream, b"* OK ready\r\n");
        let _ = std::io::Write::flush(&mut stream);
    });
    port
}

#[test]
fn a_self_signed_certificate_is_refused_with_its_fingerprint() {
    let port = serve_once();
    let Err(error) = torromail_imap_tls::connect_trusting("127.0.0.1", port, &[]) else {
        panic!("an unknown self-signed certificate must not connect");
    };
    let CoreError::UntrustedCertificate(certificate) = error else {
        panic!("expected UntrustedCertificate, got {error:?}");
    };
    assert_eq!(certificate.sha256, FINGERPRINT);
    assert_eq!(certificate.reason, "unknown_issuer");
    assert_eq!(certificate.server, format!("127.0.0.1:{port}"));
}

#[test]
fn a_trusted_fingerprint_connects_in_any_spelling() {
    // Colons and capitals, as `openssl x509 -fingerprint` prints it.
    let spelled = FINGERPRINT
        .to_ascii_uppercase()
        .as_bytes()
        .chunks(2)
        .map(|pair| std::str::from_utf8(pair).expect("ascii"))
        .collect::<Vec<_>>()
        .join(":");
    for trusted in [FINGERPRINT.to_owned(), spelled] {
        let port = serve_once();
        let mut transport = torromail_imap_tls::connect_trusting("127.0.0.1", port, &[trusted])
            .expect("a trusted certificate connects");
        assert_eq!(transport.read_line().expect("greeting"), "* OK ready");
    }
}

#[test]
fn another_trusted_fingerprint_does_not_admit_this_certificate() {
    let port = serve_once();
    let result = torromail_imap_tls::connect_trusting("127.0.0.1", port, &["00".repeat(32)]);
    assert!(
        matches!(result, Err(CoreError::UntrustedCertificate(_))),
        "a fingerprint for some other certificate must not trust this one"
    );
}

#[test]
fn a_dead_port_is_not_a_certificate_problem() {
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("address")
        .port();
    let result = torromail_imap_tls::connect_trusting("127.0.0.1", port, &[]);
    assert!(matches!(result, Err(CoreError::ProviderFailure(_))), "got {:?}", result.err());
}
