//! Configuration boundary for the real IMAP provider.
//!
//! Connection facts live here; the secret itself never does. `SecretRef`
//! points into the platform keychain and refuses to reveal itself in any
//! Debug output, so configs can travel through logs and diagnostics safely.

use std::fmt;

use crate::AccountId;

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
