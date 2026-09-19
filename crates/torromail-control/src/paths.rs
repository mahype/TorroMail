//! Where TorroMail keeps its files. One directory holds everything the
//! configuration surface and every `torromail-mcp` process share — the state,
//! the policy document published from it, and the three logs — because the
//! server finds the logs and the cache as siblings of the policy document.
//!
//! macOS: `~/Library/Application Support/TorroMail`, where the app has always
//! kept them. Elsewhere: `$XDG_STATE_HOME/torromail`, falling back to
//! `~/.local/state/torromail`.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::clients::Platform;

/// The shared directory, from the values a process would read out of its
/// environment. `None` when there is no home to put it under — a caller must
/// treat that as an error, never invent a location.
#[must_use]
pub fn data_directory(platform: Platform, home: Option<OsString>, xdg_state_home: Option<OsString>) -> Option<PathBuf> {
    match platform {
        Platform::MacOs => Some(PathBuf::from(home?).join("Library/Application Support/TorroMail")),
        Platform::Linux => {
            // The specification says a relative value is invalid and ignored.
            let stated = xdg_state_home.map(PathBuf::from).filter(|path| path.is_absolute());
            let base = match stated {
                Some(path) => path,
                None => PathBuf::from(home?).join(".local/state"),
            };
            Some(base.join("torromail"))
        }
    }
}

/// The shared directory for the running process.
#[must_use]
pub fn default_data_directory() -> Option<PathBuf> {
    data_directory(Platform::current(), std::env::var_os("HOME"), std::env::var_os("XDG_STATE_HOME"))
}

pub const STATE_FILE: &str = "state.json";
pub const POLICY_FILE: &str = "policy.json";
pub const PAIRINGS_FILE: &str = "clients.json";
pub const AUDIT_LOG: &str = "audit.jsonl";
pub const CONNECTIONS_LOG: &str = "connections.jsonl";
pub const HEALTH_LOG: &str = "health.jsonl";
