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
    if std::env::args().any(|argument| argument == "--list-tools") {
        println!("{}", ToolCatalog::default().to_mcp_tools_json());
        return Ok(());
    }

    let server = match policy_path() {
        Some(path) => LineMcpServer::with_policy_path(path),
        None => LineMcpServer::default(),
    };
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let response = server.handle_line(&line?);
        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }

    Ok(())
}
