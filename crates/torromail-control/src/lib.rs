//! The control plane's shared model: the accounts a user has configured, the
//! file they are kept in, and the policy document published from them.
//!
//! Every configuration surface — the macOS app, the terminal UI — edits the
//! same accounts and must publish the same document, so the model and the
//! writer live here once rather than once per surface. Storage sits behind
//! [`state::StateStore`] so the file format can change without the model or
//! its callers noticing.

pub mod account;
pub mod clients;
pub mod policy;
pub mod state;

pub use account::{
    CacheLevel, ConnectionSecurity, FolderRule, LoginMethod, MailAccount, OAuthIssuer,
    PermissionSet, ReadAccess, SpecialMailboxes, WriteAccess,
};
pub use policy::{ClientAccountAccess, ClientPairing, PolicyContext};
pub use state::{AppState, JsonStateStore, StateStore};

/// Why a stored document could not be understood. Carries the path of the
/// offending value so a broken file says where it is broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError(pub String);

impl std::fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FormatError {}
