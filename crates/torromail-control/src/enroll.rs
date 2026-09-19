//! Adding an account, proven before it exists. The wizard needs the real
//! answer — TLS, login — from an account that is not on any list yet, and
//! publishing it first would make an unproven account briefly reachable by
//! assistants. So the candidate is checked against a throwaway policy
//! document that names nothing but itself, and only a candidate that passed
//! is stored. A candidate that failed leaves nothing behind.

use std::path::Path;

use crate::account::{CacheLevel, LoginMethod, MailAccount, PermissionSet, SpecialMailboxes};
use crate::clients;
use crate::policy::{self, ClientAccountAccess, ClientPairing, PolicyContext};
use crate::providers::DiscoveredConfig;
use crate::save::{self, SaveError};
use crate::secrets::{self, SecretStore};
use crate::state::{JsonStateStore, StateStore};
use crate::paths;

/// The surface itself is a paired client of the server: connection checks log
/// into the real mailbox, so they sit behind the same gate as the tools.
pub const APP_CLIENT_ID: &str = "torromail-app";

/// What the connection check said about a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    Ok,
    /// The server answered and refused the login.
    Rejected(String),
    /// Nothing answered, or not in time.
    Unreachable(String),
}

/// Runs the server's `--check-account` for one account against one policy
/// document, presenting one key. Injected so the rules here are testable
/// without a mail server.
pub type Checker<'a> = &'a dyn Fn(&Path, &str, &str) -> CheckOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollError {
    /// The check failed; the wizard returns to its fields with this.
    Check(CheckOutcome),
    Other(String),
}

impl std::fmt::Display for EnrollError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Check(CheckOutcome::Rejected(reason) | CheckOutcome::Unreachable(reason)) => formatter.write_str(reason),
            Self::Check(CheckOutcome::Ok) => formatter.write_str("ok"),
            Self::Other(reason) => formatter.write_str(reason),
        }
    }
}

impl From<SaveError> for EnrollError {
    fn from(error: SaveError) -> Self {
        Self::Other(error.0)
    }
}

/// A random version-4 UUID in the spelling the macOS app uses for account ids.
pub fn new_account_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("system randomness unavailable: {error}"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    Ok(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

/// The account a password login to a discovered (or hand-entered) server
/// makes, with the everyday rights. The username defaults to the address,
/// which is what nearly every provider expects.
#[must_use]
pub fn password_account(id: String, name: &str, email: &str, config: &DiscoveredConfig) -> MailAccount {
    MailAccount {
        id,
        name: name.to_owned(),
        email: email.to_owned(),
        provider: config.provider.to_owned(),
        login_method: LoginMethod::Password,
        oauth_issuer: None,
        imap_host: config.imap_host.clone(),
        imap_port: config.imap_port,
        imap_security: config.imap_security,
        smtp_host: config.smtp_host.clone(),
        smtp_port: config.smtp_port,
        smtp_security: config.smtp_security,
        username: email.to_owned(),
        is_verified: false,
        known_mailboxes: vec!["INBOX".to_owned()],
        permissions: PermissionSet::default(),
        special_mailboxes: SpecialMailboxes::default(),
        cache_level: CacheLevel::Headers,
    }
}

/// The surface's own key, minted on first use.
pub fn app_token(secrets: &dyn SecretStore) -> Result<String, String> {
    let account = secrets::client_key_account(APP_CLIENT_ID);
    if let Some(token) = secrets.get(secrets::SERVICE, &account).map_err(|error| error.to_string())? {
        return Ok(token);
    }
    let token = clients::mint_token(APP_CLIENT_ID)?;
    secrets.set(secrets::SERVICE, &account, &token).map_err(|error| error.to_string())?;
    Ok(token)
}

fn app_pairing(token: &str) -> ClientPairing {
    ClientPairing {
        id: APP_CLIENT_ID.to_owned(),
        name: "TorroMail".to_owned(),
        token_sha256: clients::sha256_hex(token),
        account_access: ClientAccountAccess::All,
    }
}

/// Proves the candidate, then stores it. On any failure the password is taken
/// out of the secret store again and nothing else was ever written.
pub fn enroll(
    data_directory: &Path,
    secrets: &dyn SecretStore,
    context: &PolicyContext,
    mut account: MailAccount,
    password: &str,
    check: Checker<'_>,
) -> Result<MailAccount, EnrollError> {
    let mut pairings = save::load_pairings(data_directory)?;
    let store = JsonStateStore::new(data_directory.join(paths::STATE_FILE));
    if store.load().accounts.iter().any(|stored| stored.id == account.id) {
        return Err(EnrollError::Other(format!("account `{}` already exists", account.id)));
    }
    let token = app_token(secrets).map_err(EnrollError::Other)?;
    secrets.set(secrets::SERVICE, &account.id, password).map_err(|error| EnrollError::Other(error.to_string()))?;

    // A document that admits exactly one client — this surface — and names
    // exactly one account: the candidate.
    let trial = data_directory.join(format!("trial-{}.json", account.id));
    let outcome = policy::publish(&trial, std::slice::from_ref(&account), &[app_pairing(&token)], context)
        .map_err(|error| EnrollError::Other(format!("the trial document could not be written: {error}")))
        .map(|()| check(&trial, &token, &account.id));
    let _ = std::fs::remove_file(&trial);
    match outcome {
        Ok(CheckOutcome::Ok) => {}
        Ok(failed) => {
            let _ = secrets.delete(secrets::SERVICE, &account.id);
            return Err(EnrollError::Check(failed));
        }
        Err(error) => {
            let _ = secrets.delete(secrets::SERVICE, &account.id);
            return Err(error);
        }
    }

    account.is_verified = true;
    let mut state = store.load();
    state.accounts.push(account.clone());
    store.save(&state).map_err(|error| EnrollError::Other(format!("the state could not be written: {error}")))?;
    // The surface stays on the allowlist, so "test connection" keeps working
    // after the wizard is gone. Its hash is refreshed in case the key changed.
    pairings.retain(|pairing| pairing.id != APP_CLIENT_ID);
    pairings.push(app_pairing(&token));
    save::publish(data_directory, &pairings, context)?;
    Ok(account)
}
