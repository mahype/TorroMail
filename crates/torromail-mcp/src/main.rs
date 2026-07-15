use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use torromail_mcp::{LineMcpServer, ToolCatalog};

/// Where the app publishes account permissions. The env override exists for
/// development and the app supervisor; MCP clients that spawn the server
/// themselves land on the same Application Support path.
fn policy_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TORROMAIL_POLICY_PATH") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/TorroMail/policy.json"))
}

fn main() -> io::Result<()> {
    let arguments: Vec<String> = std::env::args().collect();

    if arguments.iter().any(|argument| argument == "--list-tools") {
        println!("{}", ToolCatalog::default().to_mcp_tools_json());
        return Ok(());
    }

    // The connection check behind the app's "Test Connection" button:
    // resolve the secret, log in over TLS, report — exit code carries the
    // verdict.
    if let Some(position) = arguments
        .iter()
        .position(|argument| argument == "--check-account")
    {
        let Some(account_id) = arguments.get(position + 1) else {
            eprintln!("--check-account needs an account id");
            std::process::exit(2);
        };
        match torromail_mcp::check_account(account_id, policy_path()) {
            Ok(summary) => {
                println!("{summary}");
                return Ok(());
            }
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
    }

    let server = match policy_path() {
        Some(path) => LineMcpServer::with_policy_path(path),
        None => LineMcpServer::default(),
    };
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        // Notifications get no answer at all — that is the protocol.
        if let Some(response) = server.handle_line(&line?) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }

    Ok(())
}
