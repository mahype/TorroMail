//! Removing an account, and everything of it that lives on this machine. The
//! mail stays on the server; what goes is the configuration, the password,
//! the local cache and the attachments kept for it, and every grant that
//! named it. The audit log stays as it is — a record of what happened does not
//! change because the account it happened to is gone.

use std::path::Path;

use crate::policy::{ClientAccountAccess, PolicyContext};
use crate::save::{self, SaveError};
use crate::secrets::{self, SecretStore};
use crate::state::{JsonStateStore, StateStore};
use crate::paths;

/// Removes the account from the state and the policy first — from that moment
/// no assistant reaches it — and cleans up after. What could not be cleaned is
/// returned in words rather than failing the removal: the account *is* gone,
/// and a leftover file is a smaller problem than a removal that half happened.
pub fn remove_account(
    data_directory: &Path,
    secrets: &dyn SecretStore,
    context: &PolicyContext,
    account_id: &str,
) -> Result<Vec<String>, SaveError> {
    let mut pairings = save::load_pairings(data_directory)?;
    let store = JsonStateStore::new(data_directory.join(paths::STATE_FILE));
    let mut state = store.load();
    let before = state.accounts.len();
    state.accounts.retain(|account| account.id != account_id);
    if state.accounts.len() == before {
        return Err(SaveError(format!("account `{account_id}` no longer exists")));
    }
    store.save(&state).map_err(|error| SaveError(format!("the state could not be written: {error}")))?;
    for pairing in &mut pairings {
        if let ClientAccountAccess::Selected(ids) = &mut pairing.account_access {
            ids.remove(account_id);
        }
    }
    save::publish(data_directory, &pairings, context)?;

    let mut leftovers = Vec::new();
    if let Err(error) = secrets.delete(secrets::SERVICE, account_id) {
        leftovers.push(format!("the stored password could not be removed: {error}"));
    }
    // An id is a UUID we made, or one the macOS app made — but it came out of
    // a file, so it is not trusted to stay inside the directory.
    if !crate::account::is_plain_id(account_id) {
        leftovers.push("the cache was left alone: the account id is not a plain name".to_owned());
        return Ok(leftovers);
    }
    for suffix in ["", "-wal", "-shm"] {
        let file = data_directory.join("cache").join(format!("{account_id}.sqlite{suffix}"));
        if file.exists() && std::fs::remove_file(&file).is_err() {
            leftovers.push(format!("{} could not be removed", file.display()));
        }
    }
    let attachments = data_directory.join("attachments").join(account_id);
    if attachments.exists() && std::fs::remove_dir_all(&attachments).is_err() {
        leftovers.push(format!("{} could not be removed", attachments.display()));
    }
    Ok(leftovers)
}
