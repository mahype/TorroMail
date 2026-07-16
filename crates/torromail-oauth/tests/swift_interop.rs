//! The seam between the app and the server: Swift writes this blob into the
//! keychain, Rust reads it back. Nothing else checks that the two agree, and
//! a silent disagreement here means every OAuth account fails to connect.
use torromail_oauth::TokenSet;

#[test]
fn rust_reads_the_token_blob_the_swift_app_writes() {
    // Captured verbatim from OAuthTokens.keychainPayload(). Note the escaped
    // slashes: Foundation's JSONSerialization emits them and serde must not
    // choke on them.
    let written_by_swift = r#"{"access_token":"ya29.access","expires_at":1800000000,"refresh_token":"1\/\/refresh","type":"oauth2"}"#;

    let tokens = TokenSet::from_json(written_by_swift).expect("the app's blob parses");

    assert_eq!(tokens.access_token, "ya29.access");
    assert_eq!(tokens.refresh_token, "1//refresh");
    assert_eq!(tokens.expires_at, 1_800_000_000);
    assert!(TokenSet::looks_like_token_set(written_by_swift));
}
