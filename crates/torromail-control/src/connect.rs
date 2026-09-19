//! Pairing an assistant: a key for it, the key's hash on the allowlist, the
//! key in the assistant's configuration, and a policy document that says so.
//! Four places that have to agree — this is the one function that moves them
//! together, and that puts things back when a step in the middle fails.

use std::path::Path;

use crate::clients::{self, ClientKind, Environment, ToolRunner};
use crate::policy::{ClientAccountAccess, ClientPairing, PolicyContext};
use crate::save::{self, SaveError};
use crate::secrets::{self, SecretStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectError(pub String);

impl std::fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConnectError {}

impl From<SaveError> for ConnectError {
    fn from(error: SaveError) -> Self {
        Self(error.0)
    }
}

/// Everything a pairing touches, gathered so the functions below read as the
/// steps they are.
pub struct Pairing<'a> {
    pub data_directory: &'a Path,
    pub environment: &'a Environment,
    pub secrets: &'a dyn SecretStore,
    pub context: &'a PolicyContext,
    pub run: ToolRunner<'a>,
}

impl Pairing<'_> {
    /// Connects a client, or reconnects it with a fresh key. For a client
    /// TorroMail configures itself, its config is written too; for one set up
    /// by hand the key waits in the secret store for the snippet.
    ///
    /// A reconnect keeps the client's account grants: they belong to the
    /// client, not to the key.
    pub fn connect(&self, client_id: &str, command_path: &str) -> Result<(), ConnectError> {
        let descriptor = clients::descriptor(client_id).ok_or_else(|| ConnectError(format!("unknown client `{client_id}`")))?;
        let installed = clients::installed_client(self.environment, client_id);
        if descriptor.kind == ClientKind::Automatic && installed.is_none() {
            return Err(ConnectError(format!("{} is not installed on this machine", descriptor.display_name)));
        }
        let mut pairings = save::load_pairings(self.data_directory)?;

        let account = secrets::client_key_account(client_id);
        let previous = self.secrets.get(secrets::SERVICE, &account).map_err(|error| ConnectError(error.to_string()))?;
        let token = clients::mint_token(client_id).map_err(ConnectError)?;
        self.secrets.set(secrets::SERVICE, &account, &token).map_err(|error| ConnectError(error.to_string()))?;

        if let Some(client) = &installed
            && let Err(error) = clients::add(&client.setup, command_path, &token, self.run)
        {
            // The config still carries the old key (or none): put the secret
            // store back in step with it rather than leave a key nobody holds.
            let _ = match &previous {
                Some(old) => self.secrets.set(secrets::SERVICE, &account, old),
                None => self.secrets.delete(secrets::SERVICE, &account),
            };
            return Err(ConnectError(error.to_string()));
        }

        let access = pairings
            .iter()
            .find(|pairing| pairing.id == client_id)
            .map_or(ClientAccountAccess::All, |pairing| pairing.account_access.clone());
        pairings.retain(|pairing| pairing.id != client_id);
        pairings.push(ClientPairing {
            id: client_id.to_owned(),
            name: descriptor.display_name.to_owned(),
            token_sha256: clients::sha256_hex(&token),
            account_access: access,
        });
        save::publish(self.data_directory, &pairings, self.context)?;
        Ok(())
    }

    /// Takes a client off the allowlist first — from that moment its key no
    /// longer works, whatever else fails afterwards — then out of its config
    /// and out of the secret store.
    pub fn disconnect(&self, client_id: &str) -> Result<(), ConnectError> {
        let mut pairings = save::load_pairings(self.data_directory)?;
        pairings.retain(|pairing| pairing.id != client_id);
        save::publish(self.data_directory, &pairings, self.context)?;

        self.secrets
            .delete(secrets::SERVICE, &secrets::client_key_account(client_id))
            .map_err(|error| ConnectError(error.to_string()))?;
        if let Some(client) = clients::installed_client(self.environment, client_id) {
            clients::remove(&client.setup, self.run).map_err(|error| ConnectError(error.to_string()))?;
        }
        Ok(())
    }

    /// Which accounts a paired client may see. Takes effect on the client's
    /// next tool call; no restart, no new key.
    pub fn set_account_access(&self, client_id: &str, access: ClientAccountAccess) -> Result<(), ConnectError> {
        let mut pairings = save::load_pairings(self.data_directory)?;
        let Some(pairing) = pairings.iter_mut().find(|pairing| pairing.id == client_id) else {
            return Err(ConnectError(format!("`{client_id}` is not connected")));
        };
        pairing.account_access = access;
        save::publish(self.data_directory, &pairings, self.context)?;
        Ok(())
    }

    /// The client's key, for the snippet that leaves the screen.
    pub fn token(&self, client_id: &str) -> Result<Option<String>, ConnectError> {
        self.secrets
            .get(secrets::SERVICE, &secrets::client_key_account(client_id))
            .map_err(|error| ConnectError(error.to_string()))
    }
}
