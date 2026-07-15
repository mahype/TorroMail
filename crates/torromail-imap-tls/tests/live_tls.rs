//! Live TLS handshake against a public IMAP endpoint. Ignored by default;
//! run explicitly with: cargo test -p torromail-imap-tls -- --ignored

use torromail_core::ImapTransport;

#[test]
#[ignore = "reaches the public network"]
fn tls_handshake_reaches_a_real_imap_greeting() {
    // Any large public IMAP endpoint will do — the assertion is only that
    // TLS and the greeting work; no credentials are involved.
    let mut transport =
        torromail_imap_tls::connect_transport("imap.gmail.com", 993).expect("TLS connect succeeds");
    let greeting = transport.read_line().expect("greeting arrives");

    assert!(greeting.starts_with("* OK"), "greeting was: {greeting}");
}
