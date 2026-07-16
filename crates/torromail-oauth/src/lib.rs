//! Keeping an OAuth access token usable.
//!
//! Split out for the same reason `torromail-imap-tls` is: this is the only
//! part of the server that talks HTTP, and `torromail-core` stays free of a
//! network stack.
//!
//! The MCP server needs this because it runs on its own. Claude Desktop spawns
//! the binary directly, so an access token that expired while the app was
//! closed — an hour is all Google gives one — must be renewable from here, or
//! the assistant simply stops having a mailbox.
//!
//! Only the refresh half of OAuth lives here. Getting the *first* token needs
//! a browser and a user, and that stays in the app where the user is.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// What the app stored in the keychain for an OAuth account. This is the
/// whole secret — `secret_ref` points at this blob rather than a password, so
/// the keychain contract does not change shape between account kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds. Absolute rather than a lifetime, because whoever reads
    /// this next has no idea when it was written.
    pub expires_at: u64,
}

/// A token is swapped early: one that expires mid-handshake is as useless as
/// one that expired an hour ago, and the clocks involved are not the same.
const EXPIRY_MARGIN: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub enum OAuthError {
    /// The keychain blob is not a token set — most likely a plain password on
    /// an account the document claims is OAuth.
    MalformedTokenSet(String),
    /// The provider refused to renew. Usually terminal: the user revoked
    /// access, or changed their password.
    RefreshRejected(String),
    Transport(String),
}

impl fmt::Display for OAuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedTokenSet(detail) => {
                write!(formatter, "stored OAuth token is unreadable: {detail}")
            }
            Self::RefreshRejected(detail) => write!(
                formatter,
                "the provider refused to renew access ({detail}) — sign in again in TorroMail"
            ),
            Self::Transport(detail) => write!(formatter, "reaching the token endpoint failed: {detail}"),
        }
    }
}

impl std::error::Error for OAuthError {}

impl TokenSet {
    pub fn from_json(text: &str) -> Result<Self, OAuthError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| OAuthError::MalformedTokenSet(error.to_string()))?;

        let field = |name: &str| -> Result<String, OAuthError> {
            value[name]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| OAuthError::MalformedTokenSet(format!("no {name}")))
        };

        Ok(Self {
            access_token: field("access_token")?,
            refresh_token: field("refresh_token")?,
            expires_at: value["expires_at"].as_u64().unwrap_or(0),
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::json!({
            "type": "oauth2",
            "access_token": self.access_token,
            "refresh_token": self.refresh_token,
            "expires_at": self.expires_at,
        })
        .to_string()
    }

    /// Whether this token is worth trying, `now` in unix seconds.
    pub fn is_usable_at(&self, now: u64) -> bool {
        self.expires_at > now.saturating_add(EXPIRY_MARGIN.as_secs())
    }

    /// A keychain blob only looks like this if the app wrote OAuth into it.
    /// Lets a caller tell a token set from a password without parsing.
    pub fn looks_like_token_set(text: &str) -> bool {
        text.trim_start().starts_with('{') && text.contains("refresh_token")
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Trades the refresh token for a fresh access token.
///
/// Public clients under RFC 8252 send no secret — PKCE protected the original
/// grant, and the client id is all the endpoint wants now.
pub fn refresh(token_endpoint: &str, client_id: &str, tokens: &TokenSet) -> Result<TokenSet, OAuthError> {
    let response = ureq::post(token_endpoint)
        .send_form([
            ("grant_type", "refresh_token"),
            ("refresh_token", tokens.refresh_token.as_str()),
            ("client_id", client_id),
        ])
        .map_err(|error| classify(&error.to_string()))?;

    let body = response
        .into_body()
        .read_to_string()
        .map_err(|error| OAuthError::Transport(error.to_string()))?;

    parse_refresh_response(&body, tokens, now_unix())
}

/// A 4xx from a token endpoint means the grant is gone, not that the network
/// hiccuped — the two need different words, because only one of them is fixed
/// by signing in again.
fn classify(detail: &str) -> OAuthError {
    if detail.contains("400") || detail.contains("401") {
        OAuthError::RefreshRejected(detail.to_owned())
    } else {
        OAuthError::Transport(detail.to_owned())
    }
}

/// Split out from `refresh` so the response handling is testable without a
/// network: this is where the sharp edge lives.
pub fn parse_refresh_response(
    body: &str,
    previous: &TokenSet,
    now: u64,
) -> Result<TokenSet, OAuthError> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| OAuthError::RefreshRejected(error.to_string()))?;

    if let Some(error) = value["error"].as_str() {
        let description = value["error_description"].as_str().unwrap_or(error);
        return Err(OAuthError::RefreshRejected(description.to_owned()));
    }

    let access_token = value["access_token"]
        .as_str()
        .ok_or_else(|| OAuthError::RefreshRejected("no access_token in the answer".to_owned()))?;
    let lifetime = value["expires_in"].as_u64().unwrap_or(3600);

    Ok(TokenSet {
        access_token: access_token.to_owned(),
        // Google only returns a new refresh token when it rotates one, and
        // Microsoft rotates on every call. Keeping the old one when none came
        // back is what stops a working account from locking itself out.
        refresh_token: value["refresh_token"]
            .as_str()
            .unwrap_or(&previous.refresh_token)
            .to_owned(),
        expires_at: now.saturating_add(lifetime),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> TokenSet {
        TokenSet {
            access_token: "old".to_owned(),
            refresh_token: "keep-me".to_owned(),
            expires_at: 1_000,
        }
    }

    #[test]
    fn a_refresh_without_a_new_refresh_token_keeps_the_old_one() {
        // Google's usual answer. Dropping the refresh token here would log the
        // user out for good an hour later.
        let fresh = parse_refresh_response(
            r#"{"access_token":"new","expires_in":3599,"token_type":"Bearer"}"#,
            &tokens(),
            10_000,
        )
        .expect("a refresh succeeds");

        assert_eq!(fresh.access_token, "new");
        assert_eq!(fresh.refresh_token, "keep-me");
        assert_eq!(fresh.expires_at, 13_599);
    }

    #[test]
    fn a_rotated_refresh_token_replaces_the_old_one() {
        // Microsoft's usual answer.
        let fresh = parse_refresh_response(
            r#"{"access_token":"new","refresh_token":"rotated","expires_in":3600}"#,
            &tokens(),
            0,
        )
        .expect("a refresh succeeds");

        assert_eq!(fresh.refresh_token, "rotated");
    }

    #[test]
    fn a_revoked_grant_says_so_instead_of_looking_like_a_network_problem() {
        let error = parse_refresh_response(
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
            &tokens(),
            0,
        )
        .expect_err("a revoked grant must fail");

        assert!(matches!(error, OAuthError::RefreshRejected(_)));
        assert!(error.to_string().contains("sign in again"));
    }

    #[test]
    fn token_sets_survive_a_round_trip_through_the_keychain() {
        let original = tokens();
        let restored = TokenSet::from_json(&original.to_json()).expect("round trip");
        assert_eq!(restored, original);
    }

    #[test]
    fn a_password_is_not_mistaken_for_a_token_set() {
        assert!(!TokenSet::looks_like_token_set("hunter2"));
        assert!(!TokenSet::looks_like_token_set("{not json"));
        assert!(TokenSet::looks_like_token_set(&tokens().to_json()));
    }

    #[test]
    fn a_token_about_to_expire_is_already_unusable() {
        let set = TokenSet {
            expires_at: 1_000,
            ..tokens()
        };
        assert!(set.is_usable_at(0));
        // Inside the margin: technically valid, practically not.
        assert!(!set.is_usable_at(900));
        assert!(!set.is_usable_at(2_000));
    }
}
