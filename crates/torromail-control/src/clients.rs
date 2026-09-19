//! Wiring TorroMail into MCP clients: which assistants exist, where each
//! keeps its servers, and how the `torromail` entry gets in and out.
//!
//! Servers the user configured elsewhere always survive — a JSON config is
//! merged, never replaced, and a config owned by a CLI (Claude Code, Codex) is
//! handed to that CLI rather than edited behind its back. A configuration
//! that cannot be read is an error and is never overwritten.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// The name TorroMail registers itself under in every client.
pub const SERVER_NAME: &str = "torromail";
/// The variable a client config carries the access key in.
pub const TOKEN_VARIABLE: &str = "TORROMAIL_TOKEN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Linux,
}

impl Platform {
    #[must_use]
    pub fn current() -> Self {
        if cfg!(target_os = "macos") { Self::MacOs } else { Self::Linux }
    }
}

/// Everything detection depends on, stated rather than read from the process,
/// so it can be pointed at a scratch directory.
#[derive(Debug, Clone)]
pub struct Environment {
    pub platform: Platform,
    pub home: PathBuf,
    /// Where command-line clients are looked for, in order. A GUI app inherits
    /// no shell `PATH`, so macOS passes the well-known install locations; a
    /// terminal surface passes its real `PATH`.
    pub executable_directories: Vec<PathBuf>,
}

impl Environment {
    /// The running process's own view: its home and, split, its `PATH`.
    #[must_use]
    pub fn current() -> Option<Self> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let executable_directories = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        Some(Self { platform: Platform::current(), home, executable_directories })
    }
}

/// How TorroMail can set a client up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    /// TorroMail knows where the config lives and writes itself in.
    Automatic,
    /// TorroMail only offers the snippet to paste.
    Manual,
}

/// The shape the snippet has to take — clients do not all read the same file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnippetFormat {
    /// Top-level `mcpServers` JSON object: most clients, and the default.
    McpServersJson,
    /// VS Code's `mcp.json`: the entry under `servers`, tagged `"type": "stdio"`.
    ServersJson,
    /// Hermes' `~/.hermes/config.yaml`: YAML under `mcp_servers`.
    HermesYaml,
    /// OpenClaw's `~/.openclaw/openclaw.json`: JSON nested under `mcp.servers`.
    OpenClawJson,
}

impl SnippetFormat {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::McpServersJson => "mcp_servers_json",
            Self::ServersJson => "servers_json",
            Self::HermesYaml => "hermes_yaml",
            Self::OpenClawJson => "openclaw_json",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "mcp_servers_json" => Some(Self::McpServersJson),
            "servers_json" => Some(Self::ServersJson),
            "hermes_yaml" => Some(Self::HermesYaml),
            "openclaw_json" => Some(Self::OpenClawJson),
            _ => None,
        }
    }
}

/// A client TorroMail knows about, whether or not it is on this machine. The
/// catalog drives the client list: every known assistant shows up, so the
/// user sees the whole field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub kind: ClientKind,
    pub snippet_format: SnippetFormat,
    /// For a manual client whose config file is known: where the snippet goes.
    pub manual_config_path: Option<&'static str>,
}

const fn automatic(id: &'static str, display_name: &'static str, snippet_format: SnippetFormat) -> ClientDescriptor {
    ClientDescriptor { id, display_name, kind: ClientKind::Automatic, snippet_format, manual_config_path: None }
}

/// Ids are the ones access keys are stored under, so they never change.
pub const CATALOG: &[ClientDescriptor] = &[
    automatic("claude-desktop", "Claude Desktop", SnippetFormat::McpServersJson),
    automatic("claude-code", "Claude Code", SnippetFormat::McpServersJson),
    automatic("chatgpt", "ChatGPT", SnippetFormat::McpServersJson),
    automatic("gemini-cli", "Gemini CLI", SnippetFormat::McpServersJson),
    automatic("cursor", "Cursor", SnippetFormat::McpServersJson),
    automatic("lm-studio", "LM Studio", SnippetFormat::McpServersJson),
    automatic("vscode", "VS Code", SnippetFormat::ServersJson),
    automatic("windsurf", "Windsurf", SnippetFormat::McpServersJson),
    ClientDescriptor {
        id: "clawbot",
        display_name: "Clawbot",
        kind: ClientKind::Manual,
        snippet_format: SnippetFormat::OpenClawJson,
        manual_config_path: Some("~/.openclaw/openclaw.json"),
    },
    ClientDescriptor {
        id: "hermes",
        display_name: "Hermes",
        kind: ClientKind::Manual,
        snippet_format: SnippetFormat::HermesYaml,
        manual_config_path: Some("~/.hermes/config.yaml"),
    },
    ClientDescriptor {
        id: "other",
        display_name: "Other client",
        kind: ClientKind::Manual,
        snippet_format: SnippetFormat::McpServersJson,
        manual_config_path: None,
    },
];

#[must_use]
pub fn descriptor(id: &str) -> Option<&'static ClientDescriptor> {
    CATALOG.iter().find(|descriptor| descriptor.id == id)
}

/// How one installed client is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientSetup {
    /// A JSON file with a top-level `mcpServers` object we merge into.
    McpServersJson { config: PathBuf },
    /// The same merge under VS Code's `servers` key.
    ServersJson { config: PathBuf },
    /// Codex owns its TOML; its CLI does the edit, the file is only read.
    CodexCli { executable: PathBuf, config: PathBuf },
    /// Claude Code rewrites `~/.claude.json` constantly; editing it by hand
    /// would race it, so its CLI registers us and the file is only read.
    ClaudeCodeCli { executable: PathBuf, config: PathBuf },
}

impl ClientSetup {
    #[must_use]
    pub fn config(&self) -> &Path {
        match self {
            Self::McpServersJson { config }
            | Self::ServersJson { config }
            | Self::CodexCli { config, .. }
            | Self::ClaudeCodeCli { config, .. } => config,
        }
    }

    /// For the JSON configs: the top-level key their servers live under.
    fn json_root_key(&self) -> &'static str {
        match self {
            Self::ServersJson { .. } => "servers",
            _ => "mcpServers",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledClient {
    pub id: &'static str,
    pub setup: ClientSetup,
}

/// The catalog clients present on this machine, path-resolved. Only what is
/// installed is listed: TorroMail does not offer to set up an assistant that
/// is not here.
#[must_use]
pub fn installed(environment: &Environment) -> Vec<InstalledClient> {
    let home = &environment.home;
    let mac = environment.platform == Platform::MacOs;
    let mut clients = Vec::new();
    let mut json_client = |id: &'static str, marker: PathBuf, file: &str, servers_key: bool| {
        if marker.exists() {
            let config = marker.join(file);
            clients.push(InstalledClient {
                id,
                setup: if servers_key {
                    ClientSetup::ServersJson { config }
                } else {
                    ClientSetup::McpServersJson { config }
                },
            });
        }
    };

    let application_support = if mac { home.join("Library/Application Support") } else { home.join(".config") };
    json_client("claude-desktop", application_support.join("Claude"), "claude_desktop_config.json", false);
    json_client("gemini-cli", home.join(".gemini"), "settings.json", false);
    json_client("cursor", home.join(".cursor"), "mcp.json", false);
    json_client("lm-studio", home.join(".lmstudio"), "mcp.json", false);
    json_client("windsurf", home.join(".codeium/windsurf"), "mcp_config.json", false);
    // The `User` directory is VS Code's marker; its servers sit under
    // `servers`, not `mcpServers`.
    json_client("vscode", application_support.join("Code/User"), "mcp.json", true);

    // On macOS the ChatGPT app bundles the codex CLI; elsewhere it is a
    // command on the PATH.
    let mut codex_candidates = Vec::new();
    if mac {
        codex_candidates.push(PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"));
    } else {
        codex_candidates.extend(environment.executable_directories.iter().map(|directory| directory.join("codex")));
    }
    if let Some(executable) = first_executable(&codex_candidates) {
        clients.push(InstalledClient {
            id: "chatgpt",
            setup: ClientSetup::CodexCli { executable, config: home.join(".codex/config.toml") },
        });
    }

    let mut claude_candidates = vec![home.join(".claude/local/claude")];
    if mac {
        claude_candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
        claude_candidates.push(PathBuf::from("/usr/local/bin/claude"));
        claude_candidates.push(home.join(".local/bin/claude"));
    } else {
        claude_candidates.extend(environment.executable_directories.iter().map(|directory| directory.join("claude")));
    }
    if let Some(executable) = first_executable(&claude_candidates) {
        clients.push(InstalledClient {
            id: "claude-code",
            setup: ClientSetup::ClaudeCodeCli { executable, config: home.join(".claude.json") },
        });
    }

    // Catalog order, so every surface lists the clients the same way.
    clients.sort_by_key(|client| CATALOG.iter().position(|descriptor| descriptor.id == client.id));
    clients
}

#[must_use]
pub fn installed_client(environment: &Environment, id: &str) -> Option<InstalledClient> {
    installed(environment).into_iter().find(|client| client.id == id)
}

fn first_executable(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| is_executable(path)).cloned()
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

// MARK: reading a client's configuration

fn json_server_entry(setup: &ClientSetup) -> Option<Value> {
    let root: Value = serde_json::from_str(&std::fs::read_to_string(setup.config()).ok()?).ok()?;
    root.get(setup.json_root_key())?.get(SERVER_NAME).cloned()
}

/// Whether a client already points at TorroMail. Read from its own
/// configuration — cheaper than launching a CLI on every refresh.
#[must_use]
pub fn is_configured(setup: &ClientSetup) -> bool {
    match setup {
        ClientSetup::CodexCli { config, .. } => std::fs::read_to_string(config)
            .map(|text| codex_section(&text).is_some())
            .unwrap_or(false),
        _ => json_server_entry(setup).is_some(),
    }
}

/// The TOML from our table header on. A table header is unambiguous enough to
/// scan for — but only ours: `[mcp_servers.torromail]` or a sub-table of it,
/// not another server whose name merely starts the same way.
fn codex_section(text: &str) -> Option<&str> {
    [format!("[mcp_servers.{SERVER_NAME}]"), format!("[mcp_servers.{SERVER_NAME}.")]
        .iter()
        .filter_map(|header| text.find(header.as_str()))
        .min()
        .map(|start| &text[start..])
}

/// The access key the client's config carries, if any.
#[must_use]
pub fn configured_token(setup: &ClientSetup) -> Option<String> {
    match setup {
        ClientSetup::CodexCli { config, .. } => {
            // Codex writes either an `env` table or an inline one; in both the
            // key follows the variable name as the next quoted string.
            let text = std::fs::read_to_string(config).ok()?;
            let section = codex_section(&text)?;
            let after = &section[section.find(TOKEN_VARIABLE)? + TOKEN_VARIABLE.len()..];
            let quoted = &after[after.find('=')? + 1..];
            let open = quoted.find('"')? + 1;
            let close = open + quoted[open..].find('"')?;
            Some(quoted[open..close].to_owned())
        }
        _ => json_server_entry(setup)?
            .get("env")?
            .get(TOKEN_VARIABLE)?
            .as_str()
            .map(str::to_owned),
    }
}

/// Whether the config carries exactly this key — the difference between
/// "points at TorroMail" and "will get in". False for a config written before
/// keys existed, and after a renewal the config missed.
#[must_use]
pub fn has_key(setup: &ClientSetup, token: &str) -> bool {
    configured_token(setup).as_deref() == Some(token)
}

// MARK: writing a client's configuration

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupError {
    /// The file is there and is not what we expected. Never overwritten.
    UnreadableConfig,
    WriteFailed(String),
    ToolNotStarted,
    ToolFailed,
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnreadableConfig => formatter.write_str("The existing configuration could not be read."),
            Self::WriteFailed(detail) => write!(formatter, "Could not update the configuration. ({detail})"),
            Self::ToolNotStarted => formatter.write_str("The assistant's setup tool could not be started."),
            Self::ToolFailed => formatter.write_str("The assistant's setup tool reported an error."),
        }
    }
}

impl std::error::Error for SetupError {}

/// Runs a client's own CLI. Injected so the merge rules can be tested without
/// an assistant installed.
pub type ToolRunner<'a> = &'a dyn Fn(&Path, &[String]) -> Result<(), SetupError>;

pub fn run_tool(executable: &Path, arguments: &[String]) -> Result<(), SetupError> {
    let status = std::process::Command::new(executable)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| SetupError::ToolNotStarted)?;
    if status.success() { Ok(()) } else { Err(SetupError::ToolFailed) }
}

/// Registers the server with a client, access key included.
pub fn add(setup: &ClientSetup, command_path: &str, token: &str, run: ToolRunner<'_>) -> Result<(), SetupError> {
    match setup {
        ClientSetup::McpServersJson { config } | ClientSetup::ServersJson { config } => {
            let root_key = setup.json_root_key();
            let mut root = read_json_config(config)?;
            let mut entry = json!({ "command": command_path, "env": { TOKEN_VARIABLE: token } });
            // VS Code's schema tags the transport; the `mcpServers` clients
            // infer stdio from `command` and reject an unknown key here.
            if root_key == "servers" {
                entry["type"] = json!("stdio");
            }
            let servers = root.entry(root_key).or_insert_with(|| Value::Object(Map::new()));
            if !servers.is_object() {
                *servers = Value::Object(Map::new());
            }
            servers[SERVER_NAME] = entry;
            write_json_config(config, &root)
        }
        ClientSetup::CodexCli { executable, .. } => add_via_tool(
            executable,
            &[],
            &["--env".to_owned(), format!("{TOKEN_VARIABLE}={token}")],
            command_path,
            run,
        ),
        // `-s user` registers globally; the default scope is the current
        // project, which for a background surface would be nowhere useful.
        ClientSetup::ClaudeCodeCli { executable, .. } => add_via_tool(
            executable,
            &["-s".to_owned(), "user".to_owned()],
            &["-e".to_owned(), format!("{TOKEN_VARIABLE}={token}")],
            command_path,
            run,
        ),
    }
}

/// Removes TorroMail from a client's configuration — the counterpart to `add`.
pub fn remove(setup: &ClientSetup, run: ToolRunner<'_>) -> Result<(), SetupError> {
    match setup {
        ClientSetup::McpServersJson { config } | ClientSetup::ServersJson { config } => {
            if std::fs::read(config).map(|bytes| bytes.is_empty()).unwrap_or(true) {
                return Ok(());
            }
            let mut root = read_json_config(config)?;
            let Some(servers) = root.get_mut(setup.json_root_key()).and_then(Value::as_object_mut) else {
                return Ok(());
            };
            servers.remove(SERVER_NAME);
            write_json_config(config, &root)
        }
        ClientSetup::CodexCli { executable, .. } => {
            run(executable, &["mcp".to_owned(), "remove".to_owned(), SERVER_NAME.to_owned()])
        }
        ClientSetup::ClaudeCodeCli { executable, .. } => run(
            executable,
            &["mcp".to_owned(), "remove".to_owned(), SERVER_NAME.to_owned(), "-s".to_owned(), "user".to_owned()],
        ),
    }
}

/// `<cli> mcp add <name> [scope] [env] -- <command>`. An existing entry is
/// removed first: the CLIs refuse a name that already exists, and reconnecting
/// or renewing a key *is* replacing the entry. The remove may fail freely — a
/// missing entry is exactly the state it aims for.
fn add_via_tool(
    executable: &Path,
    scope: &[String],
    environment: &[String],
    command_path: &str,
    run: ToolRunner<'_>,
) -> Result<(), SetupError> {
    let name = SERVER_NAME.to_owned();
    let _ = run(executable, &[vec!["mcp".to_owned(), "remove".to_owned(), name.clone()], scope.to_vec()].concat());
    run(
        executable,
        &[
            vec!["mcp".to_owned(), "add".to_owned(), name],
            scope.to_vec(),
            environment.to_vec(),
            vec!["--".to_owned(), command_path.to_owned()],
        ]
        .concat(),
    )
}

fn read_json_config(config: &Path) -> Result<Map<String, Value>, SetupError> {
    match std::fs::read(config) {
        // A client may ship an empty placeholder file (Cursor does); that is
        // a fresh start, not a corrupt config. So is no file at all.
        Err(_) => Ok(Map::new()),
        Ok(bytes) if bytes.is_empty() => Ok(Map::new()),
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(Value::Object(root)) => Ok(root),
            _ => Err(SetupError::UnreadableConfig),
        },
    }
}

fn write_json_config(config: &Path, root: &Map<String, Value>) -> Result<(), SetupError> {
    let text = serde_json::to_string_pretty(root).map_err(|error| SetupError::WriteFailed(error.to_string()))?;
    // A config kept in a dotfiles repository is a symlink; renaming over the
    // link would replace it with a plain file and silently detach it.
    let target = std::fs::canonicalize(config).unwrap_or_else(|_| config.to_path_buf());
    crate::state::write_atomically(&target, (text + "\n").as_bytes())
        .map_err(|error| SetupError::WriteFailed(error.to_string()))
}

// MARK: the snippet to paste

/// The snippet in the shape the target client expects. `token` is the key to
/// embed — the masked stand-in for display, the real key for the clipboard, so
/// what is on screen and what leaves it are the same text with one word
/// swapped.
#[must_use]
pub fn config_snippet(command_path: &str, format: SnippetFormat, token: &str) -> String {
    let name = SERVER_NAME;
    match format {
        SnippetFormat::McpServersJson => format!(
            r#"{{
  "mcpServers": {{
    "{name}": {{
      "command": "{command_path}",
      "env": {{
        "{TOKEN_VARIABLE}": "{token}"
      }}
    }}
  }}
}}"#
        ),
        SnippetFormat::ServersJson => format!(
            r#"{{
  "servers": {{
    "{name}": {{
      "type": "stdio",
      "command": "{command_path}",
      "env": {{
        "{TOKEN_VARIABLE}": "{token}"
      }}
    }}
  }}
}}"#
        ),
        SnippetFormat::HermesYaml => format!(
            "mcp_servers:\n  {name}:\n    command: \"{command_path}\"\n    env:\n      {TOKEN_VARIABLE}: \"{token}\""
        ),
        SnippetFormat::OpenClawJson => format!(
            r#"{{
  "mcp": {{
    "servers": {{
      "{name}": {{
        "command": "{command_path}",
        "env": {{
          "{TOKEN_VARIABLE}": "{token}"
        }}
      }}
    }}
  }}
}}"#
        ),
    }
}

// MARK: access keys

/// `torro_<client>_<32 bytes of system randomness>` — the prefix names the
/// client for humans reading a config; the entropy does the work.
pub fn mint_token(client_id: &str) -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("system randomness unavailable: {error}"))?;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("torro_{client_id}_{hex}"))
}

/// Keeps the recognizable prefix and none of the secret.
#[must_use]
pub fn masked_token(client_id: &str) -> String {
    format!("torro_{client_id}_••••••••••••")
}

/// Plain SHA-256, hex-encoded — what the server computes over the key a
/// client presents, and the only form of a key that is ever written down.
#[must_use]
pub fn sha256_hex(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where every catalog client stands on this machine, as the JSON
/// `--client-status` prints. Carries the *hash* of whatever key a config
/// holds, never the key: the caller compares it against the hash of the key
/// it stored, and no secret crosses the process boundary.
#[must_use]
pub fn status_json(environment: &Environment) -> Value {
    let present = installed(environment);
    let clients: Vec<Value> = CATALOG
        .iter()
        .map(|descriptor| {
            let client = present.iter().find(|client| client.id == descriptor.id);
            json!({
                "id": descriptor.id,
                "display_name": descriptor.display_name,
                "kind": match descriptor.kind { ClientKind::Automatic => "automatic", ClientKind::Manual => "manual" },
                "snippet_format": descriptor.snippet_format.as_str(),
                "manual_config_path": descriptor.manual_config_path,
                "installed": client.is_some(),
                "config_path": client.map(|client| client.setup.config().to_string_lossy().into_owned()),
                "configured": client.is_some_and(|client| is_configured(&client.setup)),
                "token_sha256": client
                    .and_then(|client| configured_token(&client.setup))
                    .map(|token| sha256_hex(&token)),
            })
        })
        .collect();
    json!({ "clients": clients })
}

/// What `torromail-mcp --client-setup` does for a request file:
/// `{"action": "add" | "remove", "client_id": …, "command_path": …, "token": …}`.
/// The key arrives in the file, never on a command line where any process
/// could read it.
pub fn apply_request(request: &Value, environment: &Environment, run: ToolRunner<'_>) -> Result<(), String> {
    let text = |key: &str| {
        request.get(key).and_then(Value::as_str).ok_or_else(|| format!("request: {key} is missing or not text"))
    };
    let client_id = text("client_id")?;
    let client = installed_client(environment, client_id)
        .ok_or_else(|| format!("{client_id} is not installed on this machine"))?;
    match text("action")? {
        "add" => add(&client.setup, text("command_path")?, text("token")?, run),
        "remove" => remove(&client.setup, run),
        other => return Err(format!("request: unknown action `{other}`")),
    }
    .map_err(|error| error.to_string())
}
