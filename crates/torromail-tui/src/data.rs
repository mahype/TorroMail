//! Everything the screens show, read from disk in one go. A snapshot is plain
//! data: the screens never read a file, so they can be drawn from a fixture.

use std::path::{Path, PathBuf};

use torromail_control::clients::{self, ClientDescriptor, ClientKind, Environment};
use torromail_control::connect::Pairing;
use torromail_control::policy::ClientAccountAccess;
use torromail_control::secrets::SecretStore;
use torromail_control::logs::{self, AuditEntry, ClientConnection};
use torromail_control::{JsonStateStore, MailAccount, StateStore, paths};
use torromail_mcp::health::{self, HealthVerdict};

/// What an account's dot says. The health log decides; an account it has never
/// seen falls back to what the state file remembers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountHealth {
    Connected,
    Failed(String),
    NeedsTest,
    NotConfigured,
}

impl AccountHealth {
    #[must_use]
    pub fn is_broken(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountView {
    pub account: MailAccount,
    pub health: AccountHealth,
    /// Unix seconds of the last check the log knows of.
    pub last_checked: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientView {
    pub descriptor: ClientDescriptor,
    pub installed: bool,
    pub configured: bool,
    pub config_path: Option<PathBuf>,
    pub connection: Option<ClientConnection>,
    /// What the client may see, when it is on the allowlist at all.
    pub access: Option<ClientAccountAccess>,
    /// Whether its config carries the key the allowlist knows. A client that
    /// points at TorroMail without it will be refused — the state the list
    /// has to call out for reconnecting. Always false for a manual client:
    /// nobody here can read a config it did not write.
    pub has_current_key: bool,
}

impl ClientView {
    #[must_use]
    pub fn is_paired(&self) -> bool {
        self.access.is_some()
    }

    /// Whether `connect` can do anything for it on this machine.
    #[must_use]
    pub fn can_connect(&self) -> bool {
        self.installed || self.descriptor.kind == ClientKind::Manual
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub data_directory: PathBuf,
    /// Where `torromail-mcp` was found, if anywhere.
    pub server_binary: Option<PathBuf>,
    pub accounts: Vec<AccountView>,
    pub clients: Vec<ClientView>,
    /// Newest first.
    pub audit: Vec<AuditEntry>,
    /// Unix seconds when this was taken; relative times count from here.
    pub taken_at: u64,
    /// Set when the pairing file cannot be read — nothing can be saved until
    /// that is repaired, and the surface should say so rather than fail late.
    pub pairings_problem: Option<String>,
    /// For showing paths the way people write them.
    pub home: Option<PathBuf>,
}

impl Snapshot {
    /// `~/.cursor/mcp.json` rather than the whole home path.
    #[must_use]
    pub fn tilde(&self, path: &Path) -> String {
        match self.home.as_ref().and_then(|home| path.strip_prefix(home).ok()) {
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }

    #[must_use]
    pub fn account_name(&self, id: &str) -> String {
        self.accounts
            .iter()
            .find(|view| view.account.id == id)
            .map_or_else(|| id.to_owned(), |view| view.account.name.clone())
    }

    /// The clients that both point at TorroMail and have proven it by
    /// connecting at least once.
    #[must_use]
    pub fn connected_clients(&self) -> Vec<&ClientView> {
        self.clients
            .iter()
            .filter(|client| client.is_paired() && client.connection.is_some())
            .filter(|client| client.has_current_key || client.descriptor.kind == ClientKind::Manual)
            .collect()
    }
}

#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// Reads the shared directory. Never fails: a first start has nothing to
/// show, and that is a state the screens know how to draw.
#[must_use]
pub fn load(data_directory: &Path, environment: Option<&Environment>) -> Snapshot {
    let state = JsonStateStore::new(data_directory.join(paths::STATE_FILE)).load();
    let records = health::load(&data_directory.join(paths::HEALTH_LOG));
    let last_checked = health::last_checked(&records);
    let accounts = state
        .accounts
        .into_iter()
        .map(|account| {
            let health = match health::derive(&records, &account.id, health::UNREACHABLE_GRACE) {
                Some(HealthVerdict::Connected) => AccountHealth::Connected,
                Some(HealthVerdict::Failed(reason)) => AccountHealth::Failed(reason),
                None if account.is_verified => AccountHealth::Connected,
                None if account.has_imap_connection() => AccountHealth::NeedsTest,
                None => AccountHealth::NotConfigured,
            };
            AccountView { last_checked: last_checked.get(&account.id).copied(), health, account }
        })
        .collect();

    let connections = logs::latest_connections(&data_directory.join(paths::CONNECTIONS_LOG));
    let installed = environment.map(clients::installed).unwrap_or_default();
    let (pairings, pairings_problem) = match torromail_control::save::load_pairings(data_directory) {
        Ok(pairings) => (pairings, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let clients = clients::CATALOG
        .iter()
        .map(|descriptor| {
            let found = installed.iter().find(|client| client.id == descriptor.id);
            let pairing = pairings.iter().find(|pairing| pairing.id == descriptor.id);
            let config_hash = found.and_then(|client| clients::configured_token(&client.setup)).map(|token| clients::sha256_hex(&token));
            ClientView {
                descriptor: *descriptor,
                installed: found.is_some(),
                configured: found.is_some_and(|client| clients::is_configured(&client.setup)),
                config_path: found.map(|client| client.setup.config().to_path_buf()),
                connection: connections.get(descriptor.id).cloned(),
                access: pairing.map(|pairing| pairing.account_access.clone()),
                has_current_key: pairing.is_some_and(|pairing| config_hash.as_deref() == Some(pairing.token_sha256.as_str())),
            }
        })
        .collect();

    let mut audit = logs::load_audit(&data_directory.join(paths::AUDIT_LOG), 500);
    audit.reverse();

    Snapshot {
        data_directory: data_directory.to_path_buf(),
        server_binary: environment.and_then(server_binary),
        accounts,
        clients,
        audit,
        taken_at: now(),
        pairings_problem,
        home: environment.map(|environment| environment.home.clone()),
    }
}

/// Beside this program first — that is how a package installs the pair — then
/// on the PATH.
fn server_binary(environment: &Environment) -> Option<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("torromail-mcp")));
    beside
        .into_iter()
        .chain(environment.executable_directories.iter().map(|directory| directory.join("torromail-mcp")))
        .find(|candidate| candidate.is_file())
}

/// What the surface does to the world, as opposed to what it shows. The event
/// loop owns one; tests build one over scratch directories and an in-memory
/// secret store.
pub struct Backend {
    pub data_directory: PathBuf,
    pub environment: Option<Environment>,
    pub secrets: Box<dyn SecretStore>,
    /// How a candidate account is checked: server binary, policy document,
    /// key, account id. The real one logs into the mailbox; tests put their
    /// own verdict here.
    pub checker: Box<CheckAccount>,
}

pub type CheckAccount = dyn Fn(&Path, &Path, &str, &str) -> torromail_control::enroll::CheckOutcome;

impl Backend {
    #[must_use]
    pub fn load(&self) -> Snapshot {
        load(&self.data_directory, self.environment.as_ref())
    }

    /// Saves one edited account: state and policy document, together.
    pub fn save_account(&self, account: &MailAccount) -> Result<(), String> {
        torromail_control::save::save_account(&self.data_directory, account, &torromail_control::save::default_context())
            .map_err(|error| error.to_string())
    }

    /// Checks the candidate against the real server and stores it if it
    /// passes. Returns the account's name, or the reason in words.
    pub fn enroll(&self, account: &MailAccount, password: &str) -> Result<String, String> {
        let binary = self.environment.as_ref().and_then(server_binary).ok_or("torromail-mcp was not found")?;
        let check = |policy: &Path, token: &str, account_id: &str| (self.checker)(&binary, policy, token, account_id);
        torromail_control::enroll::enroll(
            &self.data_directory,
            self.secrets.as_ref(),
            &torromail_control::save::default_context(),
            account.clone(),
            password,
            &check,
        )
        .map(|account| account.name)
        .map_err(|error| error.to_string())
    }

    /// The "Test Connection" button: a real login, and — because the user
    /// asked for it — a record in the health log like any other check.
    pub fn test_connection(&self, account_id: &str) -> Result<(), String> {
        use torromail_control::enroll::CheckOutcome;
        use torromail_mcp::health::{self, HealthOutcome};
        let binary = self.environment.as_ref().and_then(server_binary).ok_or("torromail-mcp was not found")?;
        let token = torromail_control::enroll::app_token(self.secrets.as_ref())?;
        let policy = self.data_directory.join(paths::POLICY_FILE);
        let outcome = (self.checker)(&binary, &policy, &token, account_id);
        let (word, detail) = match &outcome {
            CheckOutcome::Ok => (HealthOutcome::Ok, ""),
            CheckOutcome::Rejected(reason) => (HealthOutcome::Rejected, reason.as_str()),
            CheckOutcome::Unreachable(reason) => (HealthOutcome::Unreachable, reason.as_str()),
        };
        health::append(&self.data_directory.join(paths::HEALTH_LOG), account_id, word, "manual", detail);
        match outcome {
            CheckOutcome::Ok => Ok(()),
            CheckOutcome::Rejected(reason) | CheckOutcome::Unreachable(reason) => Err(reason),
        }
    }

    /// Removes an account and what this machine kept for it. Returns what
    /// could not be cleaned up, in words.
    pub fn remove_account(&self, account_id: &str) -> Result<Vec<String>, String> {
        torromail_control::remove::remove_account(
            &self.data_directory,
            self.secrets.as_ref(),
            &torromail_control::save::default_context(),
            account_id,
        )
        .map_err(|error| error.to_string())
    }

    fn pairing<T>(&self, act: impl FnOnce(&Pairing<'_>) -> Result<T, torromail_control::connect::ConnectError>) -> Result<T, String> {
        let environment = self.environment.as_ref().ok_or("HOME is not set")?;
        let context = torromail_control::save::default_context();
        act(&Pairing {
            data_directory: &self.data_directory,
            environment,
            secrets: self.secrets.as_ref(),
            context: &context,
            run: &clients::run_tool,
        })
        .map_err(|error| error.to_string())
    }

    /// The config of a client has to name the server by absolute path: the
    /// assistant spawns it without this terminal's PATH.
    pub fn connect(&self, client_id: &str) -> Result<(), String> {
        let binary = self.environment.as_ref().and_then(server_binary).ok_or("torromail-mcp was not found")?;
        let command = binary.to_string_lossy().into_owned();
        self.pairing(|pairing| pairing.connect(client_id, &command))
    }

    pub fn disconnect(&self, client_id: &str) -> Result<(), String> {
        self.pairing(|pairing| pairing.disconnect(client_id))
    }

    pub fn set_account_access(&self, client_id: &str, access: ClientAccountAccess) -> Result<(), String> {
        self.pairing(|pairing| pairing.set_account_access(client_id, access))
    }

    pub fn token(&self, client_id: &str) -> Result<Option<String>, String> {
        self.pairing(|pairing| pairing.token(client_id))
    }
}

/// Hands text to the desktop clipboard through whichever helper is there. The
/// text travels over stdin — it may carry a key.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use std::io::Write;
    for (program, arguments) in [("wl-copy", &[][..]), ("xclip", &["-selection", "clipboard"][..])] {
        let Ok(mut child) = std::process::Command::new(program)
            .args(arguments)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        if written && child.wait().is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    Err("wl-copy / xclip".to_owned())
}

/// `torromail-mcp --check-account`: exit 0 is a login that worked; otherwise
/// stderr carries the outcome word alone on its first line and the reason
/// after it. Never waits longer than half a minute — a hung TLS handshake must
/// not freeze the surface for good.
pub fn check_account(binary: &Path, policy: &Path, token: &str, account_id: &str) -> torromail_control::enroll::CheckOutcome {
    use torromail_control::enroll::CheckOutcome;
    let spawned = std::process::Command::new(binary)
        .args(["--check-account", account_id])
        .env("TORROMAIL_POLICY_PATH", policy)
        .env("TORROMAIL_TOKEN", token)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return CheckOutcome::Unreachable(format!("torromail-mcp could not be started: {error}")),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return CheckOutcome::Unreachable("the connection check timed out".to_owned());
            }
        }
    }
    let Ok(output) = child.wait_with_output() else {
        return CheckOutcome::Unreachable("the connection check could not be read".to_owned());
    };
    if output.status.success() {
        return CheckOutcome::Ok;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut lines = stderr.lines();
    let word = lines.next().unwrap_or_default();
    let reason = lines.collect::<Vec<_>>().join(" ").trim().to_owned();
    if word == "rejected" { CheckOutcome::Rejected(reason) } else { CheckOutcome::Unreachable(reason) }
}
