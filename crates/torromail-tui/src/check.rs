//! `torromail check`: one pass over every account, without a screen. What the
//! systemd timer runs. Each result lands in the health log as a periodic
//! check; a crossing between working and broken — and only a crossing — is
//! worth a notification. A standing problem stays red in the surface, which
//! is where a standing problem belongs.

use torromail_mcp::health::{self, HealthVerdict};

use crate::data::Backend;
use crate::i18n::Lang;

/// Raises a desktop notification: title, body.
pub type Notify<'a> = &'a dyn Fn(&str, &str);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub checked: usize,
    pub broke: Vec<String>,
    pub recovered: Vec<String>,
}

pub fn run(backend: &Backend, lang: Lang, notify: Option<Notify<'_>>) -> Summary {
    let log = backend.data_directory.join(torromail_control::paths::HEALTH_LOG);
    let mut summary = Summary::default();
    for view in backend.load().accounts {
        let account = &view.account;
        if !account.has_imap_connection() {
            continue;
        }
        let was_broken = health::derive(&health::load(&log), &account.id, health::UNREACHABLE_GRACE)
            .is_some_and(|verdict| verdict.is_broken());
        let _ = backend.check_and_record(&account.id, "periodic");
        summary.checked += 1;

        // Judged from the log, not from this one result: an unreachable
        // server only counts after three in a row, here as everywhere.
        let now = health::derive(&health::load(&log), &account.id, health::UNREACHABLE_GRACE);
        match (was_broken, now) {
            (false, Some(HealthVerdict::Failed(reason))) => {
                if let Some(notify) = notify {
                    notify(&account.name, &format!("{} {reason}", lang.t("TorroMail can no longer reach this account.")));
                }
                summary.broke.push(account.name.clone());
            }
            (true, Some(HealthVerdict::Connected)) => {
                if let Some(notify) = notify {
                    notify(&account.name, lang.t("Reachable again."));
                }
                summary.recovered.push(account.name.clone());
            }
            _ => {}
        }
    }
    summary
}

/// `notify-send`, which every desktop with a notification daemon has.
pub fn notify_send(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name", "TorroMail", "--icon", "mail-unread", title, body])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}
