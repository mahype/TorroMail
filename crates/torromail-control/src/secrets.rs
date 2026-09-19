//! Where passwords, token sets and client keys are kept. Never in a file of
//! ours: on macOS the keychain holds them, elsewhere the desktop's Secret
//! Service (gnome-keyring, KWallet). There is no plaintext fallback — a
//! machine without a secret service cannot hold an account, and saying so is
//! better than pretending.
//!
//! The Secret Service is reached through `secret-tool`, the command libsecret
//! ships. A secret travels to it over stdin and back over stdout, never on a
//! command line; only the service and account names are arguments, and those
//! are not secrets.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// The service every TorroMail secret is filed under — the same word the
/// macOS keychain items carry, so a `keychain://TorroMail/<id>` reference
/// means the same thing on both platforms.
pub const SERVICE: &str = "TorroMail";

/// The account name a client's access key is filed under.
#[must_use]
pub fn client_key_account(client_id: &str) -> String {
    format!("client-key-{client_id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// The secret service could not be reached — no session bus, a locked
    /// keyring nobody unlocked, `secret-tool` not installed. A state of the
    /// machine, and usually a passing one: it says nothing about any account.
    Unavailable(String),
    /// The service answered and refused.
    Failed(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(detail) => write!(formatter, "the secret service is not available ({detail})"),
            Self::Failed(detail) => write!(formatter, "the secret service refused ({detail})"),
        }
    }
}

impl std::error::Error for SecretError {}

pub trait SecretStore {
    /// `Ok(None)` is an answer — nothing is stored under that name — and
    /// distinct from not having been able to ask.
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), SecretError>;
    /// Removing what is not there succeeds: gone is the state it aims for.
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError>;
}

/// The Secret Service, through `secret-tool`.
#[derive(Debug, Clone)]
pub struct SecretToolStore {
    program: PathBuf,
}

impl Default for SecretToolStore {
    fn default() -> Self {
        Self { program: PathBuf::from("secret-tool") }
    }
}

impl SecretToolStore {
    /// With an explicit program — a test double, or an absolute path.
    #[must_use]
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self { program: program.into() }
    }

    fn run(&self, arguments: &[&str], input: Option<&str>) -> Result<std::process::Output, SecretError> {
        let mut child = Command::new(&self.program)
            .args(arguments)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                SecretError::Unavailable(format!("{} could not be started: {error}", self.program.display()))
            })?;
        if let (Some(secret), Some(mut stdin)) = (input, child.stdin.take()) {
            // Dropped at the end of this block, which closes the pipe: the
            // tool reads the secret to end-of-input.
            stdin
                .write_all(secret.as_bytes())
                .map_err(|error| SecretError::Failed(format!("the secret could not be handed over: {error}")))?;
        }
        child.wait_with_output().map_err(|error| SecretError::Failed(error.to_string()))
    }

    /// A failed run with something on stderr could not reach the service; the
    /// tool is silent when it merely found nothing.
    fn failure(output: &std::process::Output) -> SecretError {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if detail.is_empty() {
            SecretError::Failed(format!("secret-tool exited with {}", output.status))
        } else {
            SecretError::Unavailable(detail)
        }
    }
}

impl SecretStore for SecretToolStore {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, SecretError> {
        let output = self.run(&["lookup", "service", service, "account", account], None)?;
        if output.status.success() {
            return String::from_utf8(output.stdout)
                .map(Some)
                .map_err(|_| SecretError::Failed("the stored secret is not text".to_owned()));
        }
        if output.stderr.is_empty() { Ok(None) } else { Err(Self::failure(&output)) }
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), SecretError> {
        let label = format!("{service}: {account}");
        let output = self.run(&["store", "--label", &label, "service", service, "account", account], Some(secret))?;
        if output.status.success() { Ok(()) } else { Err(Self::failure(&output)) }
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        let output = self.run(&["clear", "service", service, "account", account], None)?;
        if output.status.success() || output.stderr.is_empty() { Ok(()) } else { Err(Self::failure(&output)) }
    }
}

/// Secrets held in memory, for tests and for nothing else.
#[derive(Debug, Default)]
pub struct MemoryStore {
    secrets: Mutex<HashMap<(String, String), String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, SecretError> {
        let secrets = self.secrets.lock().map_err(|_| SecretError::Failed("poisoned".to_owned()))?;
        Ok(secrets.get(&(service.to_owned(), account.to_owned())).cloned())
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), SecretError> {
        let mut secrets = self.secrets.lock().map_err(|_| SecretError::Failed("poisoned".to_owned()))?;
        secrets.insert((service.to_owned(), account.to_owned()), secret.to_owned());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        let mut secrets = self.secrets.lock().map_err(|_| SecretError::Failed("poisoned".to_owned()))?;
        secrets.remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }
}
