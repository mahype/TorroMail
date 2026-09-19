//! The terminal control surface. Like the macOS app it is a place to set
//! things up and see what assistants did — never a place to read mail.
//!
//! `data` loads a snapshot of everything on disk, `app` holds what the user is
//! looking at and reacts to keys, `ui` draws both. Nothing in `ui` touches the
//! file system, so every screen can be rendered in a test from a snapshot.

pub mod app;
pub mod autocheck;
pub mod check;
pub mod data;
pub mod i18n;
pub mod rebuild;
pub mod settings;
pub mod status;
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
        Request::SaveAccount(account, password) => {
            // Did anything change that a login depends on? Then the save is
            // followed by a real check, so the dot tells the truth at once.
            let stored = app.snapshot.accounts.iter().find(|view| view.account.id == account.id).map(|view| &view.account);
            let relogin = password.is_some()
                || stored.is_none_or(|stored| {
                    (&stored.username, &stored.imap_host, stored.imap_port, stored.imap_security, &stored.smtp_host, stored.smtp_port, stored.smtp_security)
                        != (&account.username, &account.imap_host, account.imap_port, account.imap_security, &account.smtp_host, account.smtp_port, account.smtp_security)
                });
            match backend.save_account(account, password.as_ref().map(|password| password.0.as_str())) {
                Ok(()) if relogin => {
                    let checked = backend.test_connection(&account.id);
                    app.replace_snapshot(backend.load());
                    end_editing(app);
                    match checked {
                        Ok(()) => app.succeeded("Saved. The connection works."),
                        Err(reason) => app.failed("Saved, but the connection does not work.", reason),
                    }
                    return;
                }
                Ok(()) => Ok("Saved."),
                Err(error) => Err(error),
            }
        }
        Request::RebuildCache(id) => {
            match backend.start_rebuild(id) {
                Ok(()) => app.rebuild = Some((id.clone(), None)),
                Err(reason) => app.failed("Could not rebuild the cache.", reason),
            }
            return;
        }
        Request::LoadMailboxes(id) => {
            match backend.list_mailboxes(id) {
                Ok(folders) => app.mailboxes_loaded(folders),
                Err(reason) => app.failed("Could not load folders. Check the connection and try again.", reason),
            }
            return;
        }
        Request::Connect(id) => backend.connect(id).map(|()| "Connected. Restart the assistant to load it."),
        Request::Disconnect(id) => backend.disconnect(id).map(|()| "Disconnected. Its access key no longer works."),
        Request::SetAccess(id, access) => backend.set_account_access(id, access.clone()).map(|()| "Saved."),
        Request::CopySnippet(id) => snippet(backend, app, id).and_then(|text| data::copy_to_clipboard(&text)).map(|()| "Copied to the clipboard."),
        Request::SaveSettings(settings) => {
            return match settings.save(&backend.data_directory) {
                Ok(()) => {
                    app.settings = **settings;
                    app.lang = settings.lang();
                }
                Err(error) => app.failed("Could not save.", error.to_string()),
            };
        }
        Request::CheckForUpdates => {
            return match (backend.release_lookup)() {
                Ok(tag) => {
                    app.update = Some((tag, data::now()));
                    app.message = None;
                }
                Err(reason) => app.failed("Could not check for updates.", reason),
            };
        }
        Request::ExportLog => {
            return match backend.export_log() {
                Ok(path) => {
                    app.succeeded("Exported.");
                    if let Some(message) = &mut app.message {
                        // File name first: the directory can be long, and a
                        // clipped line must still say what to look for.
                        let file = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                        let directory = path.parent().map(|parent| app.snapshot.tilde(parent)).unwrap_or_default();
                        message.detail = format!("{file} → {directory}");
                    }
                }
                Err(reason) => app.failed("Could not export the log.", reason),
            };
        }
        Request::SetAutocheck(on) => {
            let outcome = backend.set_autocheck(*on);
            app.replace_snapshot(backend.load());
            return match outcome {
                Ok(()) if *on => app.succeeded("Accounts are now checked every 15 minutes, also while this window is closed."),
                Ok(()) => app.succeeded("The background check is off."),
                Err(reason) => app.failed("Could not change the background check.", reason),
            };
        }
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
            if let Request::Connect(id) = &request {
                app.key_changed.insert(id.clone(), data::now());
            }
            if let Request::Disconnect(id) = &request {
                app.key_changed.remove(id);
            }
            if matches!(request, Request::SetAccess(..) | Request::SaveAccount(..)) {
                end_editing(app);
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

fn end_editing(app: &mut App) {
    app.access_draft = None;
    app.draft = None;
    app.extras = app::EditExtras::default();
    app.focus = app::Focus::List;
}

/// Looks in on a running rebuild; called by the event loop between frames.
pub fn tick(backend: &Backend, app: &mut App) {
    let Some(progress) = backend.poll_rebuild() else { return };
    match progress {
        rebuild::Progress::Working { .. } => {
            if let Some((_, shown)) = &mut app.rebuild {
                *shown = Some(progress);
            }
        }
        rebuild::Progress::Finished { .. } => {
            app.rebuild = None;
            app.replace_snapshot(backend.load());
            app.succeeded("The cache was rebuilt.");
        }
        rebuild::Progress::Failed(reason) => {
            app.rebuild = None;
            app.failed("Could not rebuild the cache.", reason);
        }
    }
}
