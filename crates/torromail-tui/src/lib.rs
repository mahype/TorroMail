//! The terminal control surface. Like the macOS app it is a place to set
//! things up and see what assistants did — never a place to read mail.
//!
//! `data` loads a snapshot of everything on disk, `app` holds what the user is
//! looking at and reacts to keys, `ui` draws both. Nothing in `ui` touches the
//! file system, so every screen can be rendered in a test from a snapshot.

pub mod app;
pub mod data;
pub mod i18n;
pub mod theme;
pub mod ui;
pub mod wizard;

use app::{App, Request};
use data::Backend;

/// Carries out what the app asked for and tells it how that went. Lives here
/// rather than in `main` so a test can drive the same code the event loop
/// does, over scratch directories.
pub fn perform(backend: &Backend, app: &mut App, request: Request) {
    let outcome = match &request {
        Request::SaveAccount(account) => backend.save_account(account).map(|()| "Saved."),
        Request::Connect(id) => backend.connect(id).map(|()| "Connected. Restart the assistant to load it."),
        Request::Disconnect(id) => backend.disconnect(id).map(|()| "Disconnected. Its access key no longer works."),
        Request::SetAccess(id, access) => backend.set_account_access(id, access.clone()).map(|()| "Saved."),
        Request::CopySnippet(id) => snippet(backend, app, id).and_then(|text| data::copy_to_clipboard(&text)).map(|()| "Copied to the clipboard."),
        Request::Discover(email) => {
            let found = (backend.discoverer)(email);
            if let Some(wizard) = &mut app.wizard {
                wizard.discovered(found);
            }
            return;
        }
        Request::TestConnection(id) => {
            let outcome = backend.test_connection(id);
            app.replace_snapshot(backend.load());
            match outcome {
                Ok(()) => app.succeeded("Connected"),
                Err(reason) => app.failed("TorroMail can no longer reach this account.", reason),
            }
            return;
        }
        Request::RemoveAccount(id) => match backend.remove_account(id) {
            Ok(leftovers) => {
                app.replace_snapshot(backend.load());
                if leftovers.is_empty() {
                    app.succeeded("Account removed. Your mail stays on the server.");
                } else {
                    app.failed("Account removed, but not everything could be cleaned up.", leftovers.join(" · "));
                }
                return;
            }
            Err(error) => Err(error),
        },
        Request::Enroll(account, password) => {
            let enrolled = backend.enroll(account, &password.0);
            app.replace_snapshot(backend.load());
            if let Some(wizard) = &mut app.wizard {
                match enrolled {
                    Ok(name) => {
                        wizard.check_passed(name);
                        app.account_index = app.snapshot.accounts.len().saturating_sub(1);
                    }
                    Err(reason) => wizard.check_failed(reason),
                }
            }
            return;
        }
        Request::RevealKey(id) => match backend.token(id) {
            Ok(Some(token)) => {
                app.revealed = Some((id.clone(), token));
                return;
            }
            Ok(None) => Err("no key is stored for this client — reconnect it".to_owned()),
            Err(error) => Err(error),
        },
    };
    app.replace_snapshot(backend.load());
    match outcome {
        Ok(text) => {
            // What is stored now is what the draft said; editing is over.
            if matches!(request, Request::SetAccess(..)) {
                app.access_draft = None;
                app.focus = app::Focus::List;
            }
            app.succeeded(text);
        }
        Err(detail) => app.failed(
            match request {
                Request::CopySnippet(_) => "No clipboard helper found.",
                Request::Connect(_) => "Could not connect.",
                _ => "Could not save.",
            },
            detail,
        ),
    }
}

/// The snippet as it leaves the screen: the real key in place of the mask.
fn snippet(backend: &Backend, app: &App, client_id: &str) -> Result<String, String> {
    let token = backend.token(client_id)?.ok_or("no key is stored for this client — reconnect it")?;
    let format = torromail_control::clients::descriptor(client_id).ok_or("unknown client")?.snippet_format;
    let command = app.snapshot.server_binary.as_ref().map_or_else(|| "torromail-mcp".to_owned(), |path| path.display().to_string());
    Ok(torromail_control::clients::config_snippet(&command, format, &token))
}
