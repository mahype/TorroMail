//! The refresh actually going over the wire — the parse tests cover the
//! response handling, this covers that ureq talks to a token endpoint at all.
//! A tiny handwritten server stands in for Google.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use torromail_oauth::{refresh, TokenSet};

#[test]
fn a_refresh_reaches_a_token_endpoint_and_comes_back_renewed() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        // The body can arrive in a separate segment from the headers, so read
        // until the request has one (it ends the message; the connection stays
        // open).
        let mut collected = Vec::new();
        let mut buffer = [0u8; 1024];
        while !String::from_utf8_lossy(&collected).contains("grant_type") {
            let read = stream.read(&mut buffer).expect("read");
            if read == 0 {
                break;
            }
            collected.extend_from_slice(&buffer[..read]);
        }
        let request = String::from_utf8_lossy(&collected);
        // The request body must carry the refresh grant — proof the client
        // sent what a real endpoint needs.
        assert!(request.contains("grant_type=refresh_token"), "req: {request}");
        assert!(request.contains("client_id=test-client"));

        let body = r#"{"access_token":"fresh-access","expires_in":3600,"token_type":"Bearer"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).expect("write");
    });

    let stale = TokenSet {
        access_token: "stale".to_owned(),
        refresh_token: "the-refresh".to_owned(),
        expires_at: 0,
    };
    let endpoint = format!("http://127.0.0.1:{port}/token");
    let fresh = refresh(&endpoint, "test-client", &stale).expect("refresh succeeds");

    server.join().expect("server thread");
    assert_eq!(fresh.access_token, "fresh-access");
    // No new refresh token came back, so the old one is kept — losing it would
    // strand the account.
    assert_eq!(fresh.refresh_token, "the-refresh");
    assert!(fresh.expires_at > 0);
}
