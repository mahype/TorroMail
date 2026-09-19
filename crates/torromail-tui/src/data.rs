//! Everything the screens show, read from disk in one go. A snapshot is plain
//! data: the screens never read a file, so they can be drawn from a fixture.

use std::path::{Path, PathBuf};

use torromail_control::clients::{self, ClientDescriptor, Environment};
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
}

impl Snapshot {
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
        self.clients.iter().filter(|client| client.configured && client.connection.is_some()).collect()
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
    let clients = clients::CATALOG
        .iter()
        .map(|descriptor| {
            let found = installed.iter().find(|client| client.id == descriptor.id);
            ClientView {
                descriptor: *descriptor,
                installed: found.is_some(),
                configured: found.is_some_and(|client| clients::is_configured(&client.setup)),
                config_path: found.map(|client| client.setup.config().to_path_buf()),
                connection: connections.get(descriptor.id).cloned(),
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

/// Saves one edited account: state and policy document, together.
pub fn save_account(data_directory: &Path, account: &MailAccount) -> Result<(), String> {
    torromail_control::save::save_account(data_directory, account, &torromail_control::save::default_context())
        .map_err(|error| error.to_string())
}
