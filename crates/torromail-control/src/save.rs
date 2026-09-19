//! Saving a change, whole: the state file first, then the policy document
//! published from exactly that state, in one call. A surface never writes one
//! without the other, so what the user sees and what the server enforces
//! cannot come apart by a forgotten second step.

use std::path::Path;

use crate::account::MailAccount;
use crate::policy::{self, ClientPairing, PolicyContext};
use crate::state::{JsonStateStore, StateStore};
use crate::{pairings, paths};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveError(pub String);

impl std::fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SaveError {}

/// What a build without OAuth registrations carries. The server never uses
/// them for a password account, and an OAuth account cannot be created
/// without real ones.
pub const UNCONFIGURED_GOOGLE_CLIENT_ID: &str = "UNCONFIGURED.apps.googleusercontent.com";
pub const UNCONFIGURED_MICROSOFT_CLIENT_ID: &str = "UNCONFIGURED-microsoft-client-id";

#[must_use]
pub fn default_context() -> PolicyContext {
    PolicyContext {
        secret_ref_prefix: format!("keychain://{}/", crate::secrets::SERVICE),
        google_client_id: UNCONFIGURED_GOOGLE_CLIENT_ID.to_owned(),
        microsoft_client_id: UNCONFIGURED_MICROSOFT_CLIENT_ID.to_owned(),
    }
}

/// Publishes the policy document from what is stored right now, with the
/// given pairings, and keeps the pairing file in step. Every change to either
/// side ends here, so the document can never lag behind one of them.
pub fn publish(data_directory: &Path, clients: &[ClientPairing], context: &PolicyContext) -> Result<(), SaveError> {
    let state = JsonStateStore::new(data_directory.join(paths::STATE_FILE)).load();
    policy::publish(&data_directory.join(paths::POLICY_FILE), &state.accounts, clients, context)
        .map_err(|error| SaveError(format!("the policy document could not be written: {error}")))?;
    pairings::save(&data_directory.join(paths::PAIRINGS_FILE), clients)
        .map_err(|error| SaveError(format!("the paired clients could not be written: {error}")))
}

/// The paired clients, or the reason nothing may be written: publishing
/// without them would silently lock every assistant out.
pub fn load_pairings(data_directory: &Path) -> Result<Vec<ClientPairing>, SaveError> {
    pairings::load(&data_directory.join(paths::PAIRINGS_FILE), &data_directory.join(paths::POLICY_FILE))
        .map_err(|error| SaveError(format!("the paired clients could not be read, so nothing was saved: {error}")))
}

/// Replaces one account — matched by id — and publishes. The state is read
/// fresh so a change another surface saved a moment ago is not overwritten by
/// a stale copy of everything else.
pub fn save_account(data_directory: &Path, account: &MailAccount, context: &PolicyContext) -> Result<(), SaveError> {
    let clients = load_pairings(data_directory)?;
    let store = JsonStateStore::new(data_directory.join(paths::STATE_FILE));
    let mut state = store.load();
    let Some(slot) = state.accounts.iter_mut().find(|stored| stored.id == account.id) else {
        return Err(SaveError(format!("account `{}` no longer exists", account.id)));
    };
    *slot = account.clone();
    store.save(&state).map_err(|error| SaveError(format!("the state could not be written: {error}")))?;
    publish(data_directory, &clients, context)
}
