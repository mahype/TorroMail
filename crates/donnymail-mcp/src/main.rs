use std::io::{self, BufRead, Write};

use donnymail_mcp::{LineMcpServer, ToolCatalog};

fn main() -> io::Result<()> {
    if std::env::args().any(|argument| argument == "--list-tools") {
        println!("{}", ToolCatalog::default().to_mcp_tools_json());
        return Ok(());
    }

    let server = LineMcpServer::default();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let response = server.handle_line(&line?);
        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }

    Ok(())
}
