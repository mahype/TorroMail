//! Private metadata, separate from the GUI-owned policy. An OS file lock
//! serializes operations for an account across MCP processes. A crash releases
//! the lock but leaves the attempt checkpoint, so an ambiguous APPEND is never
//! repeated. No credentials, recipient addresses or MIME are stored here.
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde_json::{Value, json};

use crate::ToolFailure;

pub(crate) struct DraftState {
    directory: PathBuf,
    account_key: String,
    _lock: File,
}

impl DraftState {
    pub(crate) fn open(policy: &Path, account: &str) -> Result<Self, ToolFailure> {
        let policy = fs::canonicalize(policy).map_err(|_| failure())?;
        let directory = policy
            .parent()
            .ok_or_else(|| failure())?
            .join("draft-state")
            .join(crate::sha256_hex(&policy.to_string_lossy()));
        fs::create_dir_all(&directory).map_err(|_| failure())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| failure())?;
        }
        let account_key = crate::sha256_hex(account);
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(directory.join(format!("{account_key}.lock")))
            .map_err(|_| failure())?;
        lock.lock_exclusive().map_err(|_| failure())?;
        Ok(Self {
            directory,
            account_key,
            _lock: lock,
        })
    }

    fn path(&self, key: &str) -> PathBuf {
        self.directory
            .join(format!("{}-{key}.json", self.account_key))
    }

    pub(crate) fn mailbox(&self) -> Result<Option<String>, ToolFailure> {
        Ok(read(&self.path("mailbox"))?
            .and_then(|value| value["mailbox"].as_str().map(str::to_owned)))
    }

    pub(crate) fn save_mailbox(&self, mailbox: &str) -> Result<(), ToolFailure> {
        write(&self.path("mailbox"), &json!({"mailbox": mailbox}))
    }

    pub(crate) fn clear_refused_attempt(&self, operation: &str) -> Result<(), ToolFailure> {
        fs::remove_file(self.path(operation)).map_err(|_| failure())
    }

    /// true only before the first attempt. Persist before the wire mutation;
    /// even a process dying between APPEND and its reply cannot append twice.
    pub(crate) fn begin(
        &self,
        operation: &str,
        fingerprint: &str,
        mailbox: &str,
    ) -> Result<bool, ToolFailure> {
        let path = self.path(operation);
        if let Some(value) = read(&path)? {
            if value["fingerprint"].as_str() != Some(fingerprint) {
                return Err(ToolFailure::InvalidParams("idempotency_key was already used for different draft contents; use the original contents or a new key".into()));
            }
            if value["mailbox"].as_str() != Some(mailbox) {
                return Err(ToolFailure::Core(torromail_core::CoreError::DraftFailure(
                    torromail_core::DraftFailureKind::StorageUncertain,
                )));
            }
            return Ok(false);
        }
        write(
            &path,
            &json!({"fingerprint": fingerprint, "mailbox": mailbox}),
        )?;
        Ok(true)
    }
}

fn failure() -> ToolFailure {
    ToolFailure::Io("cannot safely persist draft state; check TorroMail's local storage permissions and free space".into())
}

fn read(path: &Path) -> Result<Option<Value>, ToolFailure> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| failure()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(failure()),
    }
}

fn write(path: &Path, value: &Value) -> Result<(), ToolFailure> {
    // Account lock also covers this temporary path. Sync before atomic rename.
    let temporary = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| failure())?;
    use std::io::Write;
    file.write_all(value.to_string().as_bytes())
        .map_err(|_| failure())?;
    file.sync_all().map_err(|_| failure())?;
    fs::rename(&temporary, path).map_err(|_| failure())?;
    #[cfg(unix)]
    File::open(path.parent().ok_or_else(failure)?)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| failure())?;
    Ok(())
}
