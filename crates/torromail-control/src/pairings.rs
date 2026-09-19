//! Which clients are paired, kept in `clients.json`. Only the *hash* of a
//! client's key is here; the key itself lives in the secret store, and the
//! allowlist is never derived from what that store happens to contain — a
//! secret service offers no per-application scoping to build such trust on.
//!
//! Reading is strict. A pairing file that cannot be understood is an error,
//! because every alternative is worse: treating it as empty would lock every
//! assistant out on the next publication, and guessing could widen a grant.

use std::path::Path;

use serde_json::{Value, json};

use crate::FormatError;
use crate::policy::ClientPairing;

pub const PAIRINGS_VERSION: u64 = 1;

fn parse(text: &str, list_key: &str) -> Result<Vec<ClientPairing>, FormatError> {
    let value: Value = serde_json::from_str(text).map_err(|error| FormatError(format!("not JSON: {error}")))?;
    match value.get(list_key) {
        None => Ok(Vec::new()),
        Some(list) => list
            .as_array()
            .ok_or_else(|| FormatError(format!("{list_key} is not a list")))?
            .iter()
            .map(ClientPairing::from_json)
            .collect(),
    }
}

/// The paired clients. Before `clients.json` exists, the allowlist of the
/// policy document already in place is adopted — whoever was allowed in stays
/// allowed in across the first save from a surface that keeps this file.
pub fn load(pairings_path: &Path, policy_path: &Path) -> Result<Vec<ClientPairing>, FormatError> {
    match std::fs::read_to_string(pairings_path) {
        Ok(text) => parse(&text, "clients").map_err(|error| FormatError(format!("clients.json: {error}"))),
        Err(_) => match std::fs::read_to_string(policy_path) {
            Ok(text) => parse(&text, "clients").map_err(|error| FormatError(format!("policy.json: {error}"))),
            Err(_) => Ok(Vec::new()),
        },
    }
}

pub fn save(pairings_path: &Path, pairings: &[ClientPairing]) -> std::io::Result<()> {
    let document = json!({
        "version": PAIRINGS_VERSION,
        "clients": pairings.iter().map(ClientPairing::to_json).collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&document).map_err(std::io::Error::other)?;
    crate::state::write_atomically(pairings_path, text.as_bytes())
}
