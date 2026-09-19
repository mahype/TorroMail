//! Where the configured accounts are kept.
//!
//! [`StateStore`] is the seam: everything above it works on [`AppState`], and
//! only the implementation knows it is a JSON file. A later store — the
//! broker's database — replaces [`JsonStateStore`] without touching a caller.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::FormatError;
use crate::account::MailAccount;

pub const STATE_VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub version: u64,
    pub accounts: Vec<MailAccount>,
    /// Surface-specific settings (Dock tile, menu bar, …). Opaque here on
    /// purpose: they belong to whichever surface wrote them and must survive
    /// a save by one that has never heard of them.
    pub settings: Value,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            accounts: Vec::new(),
            settings: Value::Object(Map::new()),
        }
    }
}

impl AppState {
    pub fn from_json(text: &str) -> Result<Self, FormatError> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| FormatError(format!("not JSON: {error}")))?;
        let accounts = value
            .get("accounts")
            .and_then(Value::as_array)
            .ok_or_else(|| FormatError("accounts is missing".to_owned()))?
            .iter()
            .map(MailAccount::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            version: value.get("version").and_then(Value::as_u64).unwrap_or(STATE_VERSION),
            accounts,
            settings: value.get("settings").cloned().unwrap_or_else(|| Value::Object(Map::new())),
        })
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "version": self.version,
            "accounts": self.accounts.iter().map(MailAccount::to_json).collect::<Vec<_>>(),
            "settings": self.settings,
        })
    }
}

pub trait StateStore {
    /// Never fails: a first start has nothing stored, and losing the surface
    /// over a read error would help nobody.
    fn load(&self) -> AppState;
    fn save(&self, state: &AppState) -> std::io::Result<()>;
}

/// `state.json`, the file the macOS app has always kept beside the policy
/// document.
#[derive(Debug, Clone)]
pub struct JsonStateStore {
    path: PathBuf,
}

impl JsonStateStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl StateStore for JsonStateStore {
    fn load(&self) -> AppState {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return AppState::default();
        };
        match AppState::from_json(&text) {
            Ok(state) => state,
            Err(_) => {
                // Moved aside rather than silently overwritten by the next
                // save: the accounts in it may still be recoverable by hand.
                let mut broken = self.path.clone().into_os_string();
                broken.push(".broken");
                let _ = std::fs::remove_file(&broken);
                let _ = std::fs::rename(&self.path, &broken);
                AppState::default()
            }
        }
    }

    fn save(&self, state: &AppState) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&state.to_json()).map_err(std::io::Error::other)?;
        write_atomically(&self.path, text.as_bytes())
    }
}

/// Write beside the target, then rename over it: a crash mid-save costs the
/// new version, never the old one, and a reader sees one or the other whole.
/// Owner-only, like every file here that names hosts, usernames or clients.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut temporary = path.to_path_buf().into_os_string();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = options.open(&temporary).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    if let Err(error) = result.and_then(|()| std::fs::rename(&temporary, path)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}
