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
    // A headless server must never block on a keychain consent dialog. Disable
    // the interaction UI for the whole process, so a secret it cannot read
    // silently fails with an error the client can see, rather than a dialog
    // nobody is there to answer. The app scopes every stored item to the shared
    // Team ID, so in practice reads just succeed; this only removes the last way
    // a prompt could ever reach the user through the server. macOS only; the
    // lock re-enables on drop, so it is held for the entire run.
    #[cfg(target_os = "macos")]
    let _keychain_ui =
        security_framework::os::macos::keychain::SecKeychain::disable_user_interaction();

    let arguments: Vec<String> = std::env::args().collect();

    if arguments.iter().any(|argument| argument == "--list-tools") {
        println!("{}", ToolCatalog::default().to_mcp_tools_json());
        return Ok(());
    }

    // The policy document for a request file, on stdout. A pure
    // transformation — it reads no secret and touches no mailbox — so it sits
    // in front of the pairing gate: the app has to be able to publish the
    // very document that gate is read from.
    if let Some(position) = arguments.iter().position(|argument| argument == "--policy-document") {
        let Some(request_path) = arguments.get(position + 1) else {
            eprintln!("--policy-document needs a request file");
            std::process::exit(2);
        };
        let outcome = std::fs::read_to_string(request_path)
            .map_err(|error| format!("the request cannot be read: {error}"))
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).map_err(|error| format!("the request is not JSON: {error}")))
            .and_then(|request| torromail_control::policy::document_for_request(&request).map_err(|error| error.to_string()));
        match outcome {
            Ok(document) => println!("{document}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // Where every known MCP client stands on this machine. Read-only, and it
    // reports the hash of whatever key a config holds, never the key.
    if arguments.iter().any(|argument| argument == "--client-status") {
        let Some(environment) = torromail_control::clients::Environment::current() else {
            eprintln!("HOME is not set");
            std::process::exit(1);
        };
        println!("{}", torromail_control::clients::status_json(&environment));
        return Ok(());
    }

    // Writes TorroMail into, or out of, one client's configuration.
    if let Some(position) = arguments.iter().position(|argument| argument == "--client-setup") {
        let Some(request_path) = arguments.get(position + 1) else {
            eprintln!("--client-setup needs a request file");
            std::process::exit(2);
        };
        let outcome = std::fs::read_to_string(request_path)
            .map_err(|error| format!("the request cannot be read: {error}"))
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).map_err(|error| format!("the request is not JSON: {error}")))
            .and_then(|request| {
                let environment = torromail_control::clients::Environment::current().ok_or("HOME is not set")?;
                torromail_control::clients::apply_request(&request, &environment, &torromail_control::clients::run_tool)
            });
        if let Err(message) = outcome {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // The access key the spawning client carries in its MCP config. Read
    // here, hashed inside the server, and enforced per call against the
    // policy document's `clients` allowlist. Scrubbing it from the
    // environment afterwards would need `env::remove_var` (unsafe, and this
    // workspace forbids unsafe code); the process spawns no children, so
    // nothing inherits it.
    let presented_token = std::env::var("TORROMAIL_TOKEN").ok();

    if let Some(position) = arguments.iter().position(|argument| argument == "--list-account-mailboxes") {
        let Some(account_id) = arguments.get(position + 1) else {
            eprintln!("--list-account-mailboxes needs an account id");
            std::process::exit(2);
        };
        match torromail_mcp::list_account_mailboxes(account_id, policy_path(), presented_token.as_deref()) {
            Ok(mailboxes) => println!("{}", serde_json::to_string(&mailboxes).expect("mailbox names are JSON strings")),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
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
        let (outcome, message) =
            torromail_mcp::check_account(account_id, policy_path(), presented_token.as_deref());
        if outcome == torromail_mcp::health::HealthOutcome::Ok {
            println!("{message}");
            return Ok(());
        }
        // Two-part stderr: the outcome word on its own first line, the reason
        // after it. The app reads the first line to tell a wrong password from
        // a dead network without parsing prose, and matches it exactly — so
        // nothing may pad or precede it. The exit status keeps its older
        // meaning either way, for callers that only ever read that.
        eprintln!("{}", outcome.as_str());
        eprintln!("{message}");
        std::process::exit(1);
    }

    // The cache rebuild behind the app's "Rebuild" button: wipe, prefetch,
    // one JSON progress line per batch on stdout. The exit code carries the
    // verdict; the reason goes to stderr.
    if let Some(position) = arguments
        .iter()
        .position(|argument| argument == "--rebuild-cache")
    {
        let Some(account_id) = arguments.get(position + 1) else {
            eprintln!("--rebuild-cache needs an account id");
            std::process::exit(2);
        };
        let mut stdout = io::stdout().lock();
        let outcome = torromail_mcp::rebuild_cache(
            account_id,
            policy_path(),
            presented_token.as_deref(),
            &mut |line| {
                let _ = writeln!(stdout, "{line}");
                let _ = stdout.flush();
            },
        );
        if let Err(message) = outcome {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // The trim behind a lowered level or read permission: local housekeeping,
    // no login, quiet on success.
    if let Some(position) = arguments
        .iter()
        .position(|argument| argument == "--trim-cache")
    {
        let Some(account_id) = arguments.get(position + 1) else {
            eprintln!("--trim-cache needs an account id");
            std::process::exit(2);
        };
        if let Err(message) = torromail_mcp::trim_cache(account_id, policy_path()) {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // Prove the accounts before the first tool call needs them, so the app's
    // dots are current even when a client — not the app — started us.
    //
    // On a thread of its own, because the client that spawned this process is
    // already waiting for its `initialize` answer and the sweep is a TLS
    // handshake and a login per account with no timeout beneath either: an
    // unroutable host costs the whole TCP connect timeout, and a handful of
    // accounts would hold the handshake past the point where a client gives up
    // — a health feature that broke mail access.
    //
    // Nothing joins it. A process whose client has gone must go with it rather
    // than linger to finish logging in, so the thread simply dies with the
    // stdio loop; a sweep cut short leaves those accounts due, and the next
    // server to start picks them up. Every write it does is a whole short line
    // appended under `O_APPEND`, so an exit mid-sweep costs at most the record
    // that was not written yet, never a torn one.
    let sweep_policy_path = policy_path();
    let sweep_token = presented_token.clone();
    std::thread::spawn(move || {
        // Attachment housekeeping first — it is pure local filesystem work,
        // so it never waits behind a slow TLS handshake.
        torromail_mcp::sweep_attachment_files(sweep_policy_path.clone());
        torromail_mcp::sweep_account_health(sweep_policy_path, sweep_token.as_deref());
    });

    let server = match policy_path() {
        Some(path) => LineMcpServer::with_policy_path(path),
        None => LineMcpServer::fixture(),
    }
    .with_presented_token(presented_token.as_deref());
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
