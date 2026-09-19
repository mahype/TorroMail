//! Every screen, drawn from a fixture into an in-memory terminal. What is
//! asserted is what a person would look for: the facts on screen, in words —
//! a state must never be readable from a colour alone.

use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use torromail_control::MailAccount;
use torromail_control::clients::CATALOG;
use torromail_control::logs::{AuditEntry, ClientConnection};
use torromail_tui::app::{App, Section};
use torromail_tui::data::{AccountHealth, AccountView, ClientView, Snapshot};
use torromail_tui::i18n::Lang;
use torromail_tui::ui;

const NOW: u64 = 1_790_000_000;

fn account(id: &str, name: &str, email: &str, extra: serde_json::Value) -> MailAccount {
    let mut stored = json!({
        "id": id, "name": name, "email": email, "provider": "IMAP/SMTP", "loginMethod": "password",
        "imapHost": "imap.example.org", "imapPort": 993, "imapSecurity": "tls",
        "smtpHost": "smtp.example.org", "smtpPort": 587, "smtpSecurity": "starttls",
        "username": email, "knownMailboxes": ["INBOX"],
        "permissions": { "read": 2, "write": { "drafts": false, "mark": true, "move": true, "trash": true, "permanentDelete": false },
                         "send": false, "perFolder": true, "folderRules": { "Privat": { "read": false, "write": false } } },
        "specialMailboxes": { "manual": true, "drafts": "Entw&APw-rfe", "sent": "", "archive": "", "junk": "", "trash": "" },
        "searchCache": { "level": "attachments" }, "isVerified": true
    });
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        stored[key] = value;
    }
    MailAccount::from_json(&stored).expect("the fixture account reads")
}

fn snapshot() -> Snapshot {
    let entry = |seconds_ago: u64, client: &str, account: &str, tool: &str, detail: &str, result: &str| AuditEntry {
        timestamp: (NOW - seconds_ago) as f64,
        client: client.to_owned(),
        account: account.to_owned(),
        tool: tool.to_owned(),
        detail: detail.to_owned(),
        result: result.to_owned(),
    };
    Snapshot {
        data_directory: PathBuf::from("/home/sven/.local/state/torromail"),
        server_binary: Some(PathBuf::from("/usr/bin/torromail-mcp")),
        accounts: vec![
            AccountView {
                account: account("work", "Torro", "sven@torro.dev", json!({})),
                health: AccountHealth::Connected,
                last_checked: Some(NOW - 120),
            },
            AccountView {
                account: account("home", "Privat", "privat@gmx.de", json!({})),
                health: AccountHealth::Failed("[AUTHENTICATIONFAILED] Authentication failed.".to_owned()),
                last_checked: Some(NOW - 300),
            },
        ],
        clients: CATALOG
            .iter()
            .map(|descriptor| ClientView {
                descriptor: *descriptor,
                installed: matches!(descriptor.id, "claude-code" | "cursor"),
                configured: descriptor.id == "claude-code",
                config_path: (descriptor.id == "claude-code").then(|| PathBuf::from("/home/sven/.claude.json")),
                connection: (descriptor.id == "claude-code").then(|| ClientConnection {
                    client_id: "claude-code".to_owned(),
                    last_connected: (NOW - 120) as f64,
                    reported_name: "claude-code".to_owned(),
                    reported_version: "2.1.4".to_owned(),
                }),
            })
            .collect(),
        audit: vec![
            entry(60, "Claude Code", "work", "mail_search", "angebot · INBOX", "ok"),
            entry(90, "Codex", "home", "mail_search", "rechnung", "error"),
            entry(200, "Codex", "", "mail_list_accounts", "", "ok"),
        ],
        taken_at: NOW,
    }
}

fn render(app: &App) -> String {
    render_at(app, 112, 34)
}

fn render_at(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("an in-memory terminal");
    terminal.draw(|frame| ui::draw(frame, app)).expect("the screen draws");
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    if std::env::var_os("TORROMAIL_SHOW").is_some() {
        println!("{text}");
    }
    text
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn assert_shows(screen: &str, expected: &[&str]) {
    for text in expected {
        assert!(screen.contains(text), "the screen does not show “{text}”:\n{screen}");
    }
}

#[test]
fn the_overview_names_what_is_wrong_in_words() {
    let screen = render(&App::new(Lang::De, snapshot()));
    assert_shows(
        &screen,
        &[
            "TORROMAIL",
            "1  Übersicht", "7  Hilfe",
            "Bereit für Assistenten",
            "Claude Code verbunden",
            "Braucht deine Aufmerksamkeit",
            "[AUTHENTICATIONFAILED] Authentication failed.",
            "Zuletzt geprüft vor 5 Min",
            "Letzte Aktivität", "E-Mails durchsucht", "Abgelehnt", "Konten aufgelistet",
        ],
    );
}

#[test]
fn a_healthy_setup_shows_no_attention_card() {
    let mut healthy = snapshot();
    healthy.accounts.truncate(1);
    let screen = render(&App::new(Lang::De, healthy));
    assert!(!screen.contains("Braucht deine Aufmerksamkeit"), "its appearing is the message:\n{screen}");
}

#[test]
fn a_first_start_explains_itself_instead_of_showing_empty_boxes() {
    let mut app = App::new(Lang::En, Snapshot::default());
    assert_shows(&render(&app), &["Not reachable for assistants", "No assistant connected", "No accounts yet.", "No activity yet."]);
    press(&mut app, KeyCode::Char('2'));
    assert_shows(&render(&app), &["No accounts yet."]);
    press(&mut app, KeyCode::Char('6'));
    assert_shows(&render(&app), &["No activity yet."]);
}

#[test]
fn enter_on_the_overview_leads_to_the_broken_account() {
    let mut app = App::new(Lang::De, snapshot());
    press(&mut app, KeyCode::Enter);
    assert_eq!((app.section, app.account_index, app.account_tab), (Section::Accounts, 1, 0));
    assert_shows(
        &render(&app),
        &["Privat · privat@gmx.de", "Nicht erreichbar", "TorroMail erreicht dieses Konto nicht mehr.", "imap.example.org:993", "SSL/TLS", "STARTTLS"],
    );
}

#[test]
fn the_account_tabs_show_the_stored_decisions() {
    let mut app = App::new(Lang::De, snapshot());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Tab);
    assert_shows(&render(&app), &["(•) Aufräumen", "( ) Voller Zugriff", "Ganze E-Mail", "[✓] Verschieben", "[ ] Senden", "nur manuell"]);
    press(&mut app, KeyCode::Tab);
    assert_shows(&render(&app), &["Rechte pro Ordner", "Privat", "kein Zugriff", "Spezialordner", "Entwürfe"]);
    press(&mut app, KeyCode::Tab);
    assert_shows(&render(&app), &["E-Mails & Anhänge", "Durch das Leserecht begrenzt auf", "„Ganze E-Mails“"]);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.account_tab, 0, "the tabs wrap around");
}

#[test]
fn the_client_list_says_where_each_assistant_stands_and_never_shows_a_key() {
    let mut app = App::new(Lang::De, snapshot());
    press(&mut app, KeyCode::Char('3'));
    assert_shows(&render(&app), &["Claude Desktop", "Auf diesem Rechner nicht gefunden", "Hat sich noch nicht verbunden"]);
    press(&mut app, KeyCode::Down);
    let screen = render(&app);
    assert_shows(
        &screen,
        &[
            "Claude Code hat sich mit dem Server verbunden", "Version 2.1.4", "/home/sven/.claude.json",
            "Nicht verbunden", "Nicht installiert", "Von Hand einrichten",
            "\"mcpServers\"", "/usr/bin/torromail-mcp", "torro_claude-code_••••••••••••",
        ],
    );
    press(&mut app, KeyCode::End);
    assert_shows(&render(&app), &["Anderer Client", "Hat sich noch nicht verbunden"]);
}

#[test]
fn the_log_sorts_by_the_chosen_column() {
    let mut app = App::new(Lang::De, snapshot());
    press(&mut app, KeyCode::Char('6'));
    // Six columns beside the menu need room; a narrower terminal clips the
    // details first, which is the column that can best afford it.
    let newest_first = render_at(&app, 140, 34);
    assert_shows(&newest_first, &["Zeit ▼", "angebot · INBOX", "rechnung"]);
    assert!(newest_first.find("angebot").expect("shown") < newest_first.find("rechnung").expect("shown"));

    press(&mut app, KeyCode::Char('r'));
    let oldest_first = render_at(&app, 140, 34);
    assert_shows(&oldest_first, &["Zeit ▲"]);
    assert!(oldest_first.find("rechnung").expect("shown") < oldest_first.find("angebot").expect("shown"));

    press(&mut app, KeyCode::Char('s'));
    assert_shows(&render(&app), &["Client ▲"]);
}

#[test]
fn selection_stops_at_the_ends_and_survives_a_reload() {
    let mut app = App::new(Lang::De, snapshot());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.account_index, 0);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.account_index, 1);

    let mut smaller = snapshot();
    smaller.accounts.truncate(1);
    app.replace_snapshot(smaller);
    assert_eq!(app.account_index, 0, "a selection past the end moves to the last account");
    render(&app);
}

#[test]
fn the_quiet_screens_and_a_narrow_terminal_draw_without_panicking() {
    let mut app = App::new(Lang::En, snapshot());
    for key in ['4', '5', '7'] {
        press(&mut app, KeyCode::Char(key));
        render(&app);
    }
    assert_shows(&render(&app), &["About TorroMail", "/home/sven/.local/state/torromail"]);
    for (width, height) in [(40, 10), (20, 5), (1, 1)] {
        for section in Section::ALL {
            app.section = section;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a tiny terminal");
            terminal.draw(|frame| ui::draw(frame, &app)).expect("still draws");
        }
    }
    press(&mut app, KeyCode::Char('q'));
    assert!(app.should_quit);
}
