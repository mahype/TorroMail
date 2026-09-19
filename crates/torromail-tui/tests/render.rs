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
use torromail_control::enroll::CheckOutcome;
use torromail_control::logs::{AuditEntry, ClientConnection};
use torromail_tui::app::{App, Focus, Request, Section};
use torromail_tui::data::{AccountHealth, AccountView, Backend, ClientView, Snapshot};
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
                cache_bytes: 3_400_000,
            },
            AccountView {
                account: account("home", "Privat", "privat@gmx.de", json!({})),
                health: AccountHealth::Failed("[AUTHENTICATIONFAILED] Authentication failed.".to_owned()),
                last_checked: Some(NOW - 300),
                cache_bytes: 0,
            },
        ],
        clients: CATALOG
            .iter()
            .map(|descriptor| ClientView {
                descriptor: *descriptor,
                installed: matches!(descriptor.id, "claude-code" | "cursor" | "pi"),
                configured: descriptor.id == "claude-code",
                config_path: (descriptor.id == "claude-code").then(|| PathBuf::from("/home/sven/.claude.json")),
                connection: (descriptor.id == "claude-code").then(|| ClientConnection {
                    client_id: "claude-code".to_owned(),
                    last_connected: (NOW - 120) as f64,
                    reported_name: "claude-code".to_owned(),
                    reported_version: "2.1.4".to_owned(),
                }),
                access: (descriptor.id == "claude-code").then_some(torromail_control::policy::ClientAccountAccess::All),
                has_current_key: descriptor.id == "claude-code",
                requirement_met: (descriptor.id == "pi").then_some(false),
            })
            .collect(),
        audit: vec![
            entry(60, "Claude Code", "work", "mail_search", "angebot · INBOX", "ok"),
            entry(90, "Codex", "home", "mail_search", "rechnung", "error"),
            entry(200, "Codex", "", "mail_list_accounts", "", "ok"),
        ],
        taken_at: NOW,
        pairings_problem: None,
        home: Some(PathBuf::from("/home/sven")),
        server_tools: Some(15),
        autocheck: false,
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
    assert_shows(
        &render(&app),
        &["Getting started", "Add a mail account", " n ", "Decide what is allowed", "Connect an assistant", "torromail-mcp was not found"],
    );
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
    // Taller than the default: the hint and the snippet sit below the status.
    let screen = render_at(&app, 112, 50);
    assert_shows(
        &screen,
        &[
            "In Claude Code prüfen", "/mcp",
            "Claude Code hat sich mit dem Server verbunden", "Version 2.1.4", "~/.claude.json", "TorroMail-Server antwortet — 15 Werkzeuge",
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

// MARK: editing permissions

fn ctrl(app: &mut App, character: char) {
    app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL));
}

fn editing() -> App {
    let mut app = App::new(Lang::De, snapshot());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    app
}

#[test]
fn a_preset_is_applied_to_a_draft_and_nothing_is_saved_until_asked() {
    let mut app = editing();
    assert_shows(&render(&app), &["(•) Aufräumen", "leertaste", "strg+s"]);
    for _ in 0..3 {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Char(' '));
    assert_shows(&render(&app), &["(•) Voller Zugriff", "[✓] Senden", "E-Mail und Anhänge", "Ungespeicherte Änderungen"]);
    assert!(app.request.is_none(), "a change is a draft until ctrl+s");
    assert!(!app.snapshot.accounts[0].account.permissions.send, "the stored account is untouched");

    ctrl(&mut app, 's');
    let Some(Request::SaveAccount(requested, None)) = app.request.take() else { panic!("ctrl+s asks the event loop to save") };
    assert!(requested.permissions.send);
}

#[test]
fn leaving_with_unsaved_changes_asks_first() {
    let mut app = editing();
    press(&mut app, KeyCode::Char(' '));
    assert!(app.is_dirty());

    press(&mut app, KeyCode::Char('1'));
    assert_eq!(app.section, Section::Accounts, "a menu key does not throw an edit away");
    press(&mut app, KeyCode::Esc);
    assert_shows(&render(&app), &["Ungespeicherte Änderungen verwerfen? (j/n)"]);
    press(&mut app, KeyCode::Char('n'));
    assert!(app.is_dirty(), "n keeps editing");

    ctrl(&mut app, 'c');
    assert!(!app.should_quit, "quitting over unsaved changes asks too");
    press(&mut app, KeyCode::Char('j'));
    assert!(app.draft.is_none());
    assert_shows(&render(&app), &["(•) Aufräumen"]);

    // Nothing changed: esc just leaves.
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Esc);
    assert!(app.draft.is_none() && !app.confirm_discard);
}

#[test]
fn permanent_delete_cannot_outlive_the_delete_right() {
    let mut app = editing();
    press(&mut app, KeyCode::End);
    for _ in 0..10 {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Char(' '));
    assert!(app.draft.as_ref().expect("editing").permissions.write.permanent_delete, "trash is on, so it may be set");

    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Char(' '));
    let write = app.draft.as_ref().expect("editing").permissions.write;
    assert!(!write.trash && !write.permanent_delete, "taking Delete away takes its escalation with it");

    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    assert!(!app.draft.as_ref().expect("editing").permissions.write.permanent_delete);
    assert_shows(&render(&app), &["braucht zuerst das"]);
}

#[test]
fn the_read_depth_steps_and_stops_at_its_ends() {
    let mut app = editing();
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
    for _ in 0..5 {
        press(&mut app, KeyCode::Right);
    }
    assert_shows(&render(&app), &["‹ E-Mail und Anhänge ›"]);
    for _ in 0..5 {
        press(&mut app, KeyCode::Left);
    }
    assert_shows(&render(&app), &["‹ Kein Lesezugriff ›", "Voreinstellung · angepasst"]);
}

#[test]
fn a_saved_change_reaches_the_policy_document_the_server_reads() {
    use torromail_control::{AppState, JsonStateStore, StateStore, paths};
    let directory = std::env::temp_dir().join(format!("torromail-tui-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let state = AppState { accounts: vec![account("work", "Torro", "sven@torro.dev", json!({}))], ..AppState::default() };
    JsonStateStore::new(directory.join(paths::STATE_FILE)).save(&state).expect("seeds");

    let backend = Backend {
        data_directory: directory.clone(),
        environment: None,
        secrets: Box::new(torromail_control::secrets::MemoryStore::default()),
        checker: Box::new(|_, _, _, _| CheckOutcome::Ok),
        discoverer: Box::new(|_| None),
        mailbox_lister: Box::new(|_, _, _, _| Ok(Vec::new())),
        rebuild_command: Box::new(|_, _, _, _| std::process::Command::new("true")),
        rebuild: std::cell::RefCell::new(None),
        tool_count: std::cell::OnceCell::new(),
        unit_directory: directory.join("units"),
        systemctl: Box::new(|_| Ok(())),
        release_lookup: Box::new(|| Ok("v0.0.1".to_owned())),
        export_directory: directory.join("exports"),
    };
    let mut app = App::new(Lang::De, backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    for _ in 0..6 {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Char(' '));
    ctrl(&mut app, 's');
    let request = app.request.take().expect("a save was requested");
    torromail_tui::perform(&backend, &mut app, request);

    assert!(!app.is_dirty(), "what is stored now equals the draft");
    assert_shows(&render(&app), &["[✓] Senden", "Gespeichert."]);
    let policy: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(directory.join(paths::POLICY_FILE)).expect("published")).expect("JSON");
    assert_eq!(policy["accounts"][0]["send"], true);
    std::fs::remove_dir_all(&directory).ok();
}

// MARK: connecting an assistant

struct Scene {
    root: PathBuf,
    backend: Backend,
}

/// A machine with Cursor installed, the server on the PATH, and two accounts.
fn scene(name: &str) -> Scene {
    use torromail_control::clients::{Environment, Platform};
    use torromail_control::{AppState, JsonStateStore, StateStore, paths};
    let root = std::env::temp_dir().join(format!("torromail-tui-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("home/.cursor")).expect("cursor is installed");
    std::fs::create_dir_all(root.join("bin")).expect("a bin directory");
    std::fs::write(root.join("bin/torromail-mcp"), "").expect("the server is installed");
    let state = AppState {
        accounts: vec![
            account("work", "Torro", "sven@torro.dev", json!({})),
            account("home", "Privat", "privat@gmx.de", json!({})),
        ],
        ..AppState::default()
    };
    JsonStateStore::new(root.join("data").join(paths::STATE_FILE)).save(&state).expect("seeds");
    Scene {
        backend: Backend {
            data_directory: root.join("data"),
            environment: Some(Environment {
                platform: Platform::Linux,
                home: root.join("home"),
                executable_directories: vec![root.join("bin")],
            }),
            secrets: Box::new(torromail_control::secrets::MemoryStore::default()),
            checker: Box::new(|_, _, _, _| CheckOutcome::Ok),
            discoverer: Box::new(|_| None),
            mailbox_lister: Box::new(|_, _, _, _| Ok(vec!["INBOX".to_owned(), "Entw&APw-rfe".to_owned(), "Privat".to_owned()])),
            rebuild_command: Box::new(|_, _, _, _| std::process::Command::new("true")),
            rebuild: std::cell::RefCell::new(None),
            tool_count: std::cell::OnceCell::new(),
            unit_directory: root.join("units"),
            systemctl: Box::new(|_| Ok(())),
            release_lookup: Box::new(|| Ok("v0.0.1".to_owned())),
            export_directory: root.join("exports"),
        },
        root,
    }
}

impl Scene {
    fn app_on_cursor(&self) -> App {
        let mut app = App::new(Lang::De, self.backend.load());
        press(&mut app, KeyCode::Char('3'));
        // By id, not by position: the catalog grows.
        while app.snapshot.clients[app.client_index].descriptor.id != "cursor" {
            press(&mut app, KeyCode::Down);
        }
        app
    }

    fn act(&self, app: &mut App) {
        let request = app.request.take().expect("the key asked for something");
        torromail_tui::perform(&self.backend, app, request);
    }

    fn policy(&self) -> serde_json::Value {
        let path = self.root.join("data").join(torromail_control::paths::POLICY_FILE);
        serde_json::from_str(&std::fs::read_to_string(path).expect("published")).expect("JSON")
    }

    fn cursor_config(&self) -> String {
        std::fs::read_to_string(self.root.join("home/.cursor/mcp.json")).unwrap_or_default()
    }
}

#[test]
fn c_connects_an_assistant_and_the_key_stays_off_the_screen() {
    let scene = scene("connect");
    let mut app = scene.app_on_cursor();
    assert_shows(&render(&app), &["Nicht verbunden", " c ", "verbinden"]);

    press(&mut app, KeyCode::Char('c'));
    scene.act(&mut app);

    let key = scene.backend.token("cursor").expect("readable").expect("a key was stored");
    let screen = render_at(&app, 112, 48);
    assert_shows(
        &screen,
        &["Verbunden. Starte den Assistenten neu", "Eingerichtet", "Der Client hat seinen Zugangsschlüssel", "~/.cursor/mcp.json", "Kontozugriff", "(•) Alle Konten", "torro_cursor_••••••••••••"],
    );
    assert!(!screen.contains(&key[..24]), "the key is masked until asked for");
    assert!(scene.cursor_config().contains(&key), "Cursor's own config carries it");
    assert!(scene.cursor_config().contains("bin/torromail-mcp"), "and names the server by absolute path");
    assert_eq!(scene.policy()["clients"][0]["id"], "cursor");
}

#[test]
fn v_shows_the_key_only_for_the_selected_client_and_only_until_it_moves() {
    let scene = scene("reveal");
    let mut app = scene.app_on_cursor();
    press(&mut app, KeyCode::Char('c'));
    scene.act(&mut app);
    let key = scene.backend.token("cursor").expect("readable").expect("stored");

    press(&mut app, KeyCode::Char('v'));
    scene.act(&mut app);
    assert!(render_at(&app, 160, 40).contains(&key), "v reveals");
    press(&mut app, KeyCode::Up);
    assert!(app.revealed.is_none(), "moving away hides it again");
    press(&mut app, KeyCode::Down);
    assert!(!render_at(&app, 160, 40).contains(&key));
}

#[test]
fn account_access_is_narrowed_in_a_draft_and_saved_on_request() {
    let scene = scene("access");
    let mut app = scene.app_on_cursor();
    press(&mut app, KeyCode::Char('c'));
    scene.act(&mut app);

    press(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::Detail);
    // Third row is the first account; unticking it under "all" leaves the rest.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    assert_shows(&render(&app), &["(•) Ausgewählte Konten", "[ ] Torro", "[✓] Privat", "Ungespeicherte Änderungen"]);
    assert_eq!(scene.policy()["clients"][0]["account_access"]["mode"], "all", "nothing is saved yet");

    ctrl(&mut app, 's');
    scene.act(&mut app);
    assert_eq!(scene.policy()["clients"][0]["account_access"], json!({ "mode": "selected", "account_ids": ["home"] }));
    assert_eq!(app.focus, Focus::List);
    assert_shows(&render(&app), &["Gespeichert.", "[ ] Torro"]);
}

#[test]
fn disconnecting_asks_first_then_revokes_and_cleans_up() {
    let scene = scene("disconnect");
    let mut app = scene.app_on_cursor();
    press(&mut app, KeyCode::Char('c'));
    scene.act(&mut app);

    press(&mut app, KeyCode::Char('T'));
    assert_shows(&render(&app), &["Trennen: Cursor? (j/n)"]);
    press(&mut app, KeyCode::Char('n'));
    assert!(app.request.is_none(), "n changes nothing");

    press(&mut app, KeyCode::Char('T'));
    press(&mut app, KeyCode::Char('j'));
    scene.act(&mut app);
    assert_shows(&render(&app), &["Getrennt.", "Nicht verbunden"]);
    assert_eq!(scene.policy()["clients"], json!([]));
    assert_eq!(scene.backend.token("cursor"), Ok(None));
    assert!(!scene.cursor_config().contains("torromail"));
}

#[test]
fn an_assistant_that_is_not_installed_offers_no_connect() {
    let scene = scene("absent");
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('3'));
    press(&mut app, KeyCode::Char('c'));
    assert!(app.request.is_none(), "Claude Desktop is not on this machine");
    assert!(!render(&app).contains(" c "), "and the key is not advertised");
}


// MARK: adding an account

fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, KeyCode::Char(character));
    }
}

#[test]
fn a_known_provider_needs_only_an_address_and_a_password() {
    let scene = scene("wizard");
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.section, Section::Accounts, "n on the overview goes where accounts live");
    assert_shows(&render(&app), &["Neues Konto", "● E-Mail", "○ Anmeldung", "Absendername"]);

    // q, 1, j are letters now, not commands.
    type_text(&mut app, "jq1@mailbox.org");
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "Sven");
    press(&mut app, KeyCode::Enter);
    assert!(!app.should_quit);
    assert_shows(&render(&app), &["✓ E-Mail", "Einstellungen gefunden · mailbox.org", "imap.mailbox.org:993", "smtp.mailbox.org:587", "Passwort"]);

    type_text(&mut app, "hunter2-secret");
    let screen = render(&app);
    assert!(!screen.contains("hunter2"), "a password is never drawn");
    assert_shows(&screen, &["••••••••••••••"]);

    press(&mut app, KeyCode::Enter);
    assert_shows(&render(&app), &["Was dürfen verbundene Assistenten", "(•) Lesen + Entwürfe"]);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_shows(&render(&app), &["Verbindung wird geprüft"]);
    assert!(!format!("{app:?}").contains("hunter2"), "nor does it reach a debug print");

    scene.act(&mut app);
    assert_shows(&render(&app), &["Verbunden als Sven", "✓ Berechtigungen"]);
    press(&mut app, KeyCode::Enter);
    assert!(app.wizard.is_none());

    let created = &app.snapshot.accounts[app.account_index];
    assert_eq!((created.account.email.as_str(), created.account.imap_host.as_str()), ("jq1@mailbox.org", "imap.mailbox.org"));
    assert_eq!(created.account.permissions.matching_preset(), Some(torromail_control::PermissionPreset::TidyUp));
    assert_eq!(created.health, AccountHealth::Connected);
    let stored_password = scene.backend.secrets.get("TorroMail", &created.account.id).expect("readable");
    assert_eq!(stored_password.as_deref(), Some("hunter2-secret"));
    assert_eq!(scene.policy()["accounts"].as_array().expect("accounts").len(), 3);
}

#[test]
fn a_refused_login_returns_to_the_password_with_the_servers_words() {
    let mut scene = scene("wizard-refused");
    scene.backend.checker = Box::new(|_, _, _, _| CheckOutcome::Rejected("[AUTHENTICATIONFAILED] Invalid credentials".to_owned()));
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('n'));
    type_text(&mut app, "sven@gmx.de");
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "wrong");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    scene.act(&mut app);

    let wizard = app.wizard.as_ref().expect("still open");
    assert_eq!(wizard.step, torromail_tui::wizard::Step::SignIn);
    assert_eq!(wizard.email, "sven@gmx.de", "nothing typed is lost");
    assert_shows(&render(&app), &["[AUTHENTICATIONFAILED] Invalid credentials", "● Anmeldung"]);
    assert_eq!(app.snapshot.accounts.len(), 2, "no account came to exist");
}

#[test]
fn an_unknown_domain_asks_for_the_servers_and_gmail_explains_the_app_password() {
    let scene = scene("wizard-unknown");
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('n'));
    type_text(&mut app, "hallo@lindenhof-design.de");
    press(&mut app, KeyCode::Enter);
    assert_shows(&render(&app), &["Suche die Einstellungen für hallo@lindenhof-design.de"]);
    scene.act(&mut app);
    assert_shows(&render(&app), &["Für diese Domain war nichts zu finden", "IMAP-Server", "SMTP-Port", "587"]);
    press(&mut app, KeyCode::Enter);
    assert_shows(&render(&app), &["Server von Hand eintragen."]);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert!(app.wizard.is_none(), "esc walks back out");

    press(&mut app, KeyCode::Char('n'));
    assert_shows(&render(&app), &["Neues Konto", "E-Mail"]);
    press(&mut app, KeyCode::Enter);
    assert_shows(&render(&app), &["TorroMail braucht eine Adresse"]);
    type_text(&mut app, "sven@gmail.com");
    press(&mut app, KeyCode::Enter);
    assert_shows(&render_at(&app, 112, 44), &["Gmail", "Ein-Klick-Anmeldung", "https://myaccount.google.com/apppasswords", "App-Passwort"]);
}


// MARK: testing and removing an account

#[test]
fn t_logs_in_for_real_and_the_result_lands_in_the_health_log() {
    let mut scene = scene("test-connection");
    scene.backend.checker = Box::new(|_, policy, _, _| {
        assert!(policy.ends_with("policy.json"), "a test runs against the live document, not a trial");
        CheckOutcome::Rejected("[AUTHENTICATIONFAILED] Invalid credentials".to_owned())
    });
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('t'));
    assert_shows(&render(&app), &["Verbindung wird geprüft"]);
    scene.act(&mut app);

    assert_shows(&render(&app), &["TorroMail erreicht dieses Konto nicht mehr.", "[AUTHENTICATIONFAILED] Invalid credentials", "▲ Nicht erreichbar"]);
    let log = std::fs::read_to_string(scene.root.join("data/health.jsonl")).expect("a record was written");
    assert!(log.contains(r#""source":"manual""#) && log.contains(r#""outcome":"rejected""#), "got: {log}");

    scene.backend.checker = Box::new(|_, _, _, _| CheckOutcome::Ok);
    press(&mut app, KeyCode::Char('t'));
    scene.act(&mut app);
    assert_shows(&render(&app), &["● Verbunden"]);
}

#[test]
fn removing_an_account_says_what_goes_and_asks_first() {
    let scene = scene("remove");
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('D'));
    assert_shows(&render(&app), &["Entfernen: „Torro“? (j/n)", "lokalen Cache"]);
    press(&mut app, KeyCode::Char('n'));
    assert!(app.request.is_none());

    press(&mut app, KeyCode::Char('D'));
    press(&mut app, KeyCode::Char('j'));
    scene.act(&mut app);
    assert_eq!(app.snapshot.accounts.len(), 1);
    assert_shows(&render(&app), &["Konto entfernt.", "Privat"]);
    assert_eq!(scene.policy()["accounts"].as_array().expect("accounts").len(), 1);
}

#[test]
fn page_down_scrolls_a_detail_that_is_taller_than_the_terminal() {
    let scene = scene("scroll");
    let mut app = scene.app_on_cursor();
    press(&mut app, KeyCode::Char('c'));
    scene.act(&mut app);
    assert!(!render(&app).contains("behandle ihn wie"), "the end of the detail is below the fold at 34 rows");
    press(&mut app, KeyCode::PageDown);
    press(&mut app, KeyCode::PageDown);
    assert_shows(&render(&app), &["behandle ihn wie"]);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.detail_scroll, 0, "another client starts at the top");
}


#[test]
fn a_company_domain_is_looked_up_and_a_workspace_mailbox_is_not_offered_a_password_that_cannot_work() {
    let mut scene = scene("wizard-discover");
    scene.backend.discoverer = Box::new(|email| {
        assert_eq!(email, "sven@firma.example");
        torromail_control::providers::from_mx_host("firma-example.mail.protection.outlook.com")
    });
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('n'));
    type_text(&mut app, "sven@firma.example");
    press(&mut app, KeyCode::Enter);
    scene.act(&mut app);

    assert_shows(
        &render_at(&app, 112, 44),
        &["Einstellungen gefunden · Microsoft 365", "outlook.office365.com:993", "Ein-Klick-Anmeldung", "Administration"],
    );
}


// MARK: editing the other account tabs

#[test]
fn the_connection_tab_takes_text_checks_ports_and_rechecks_the_login_on_save() {
    let mut scene = scene("edit-connection");
    scene.backend.checker = Box::new(|_, _, _, _| CheckOutcome::Ok);
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Enter);
    assert_shows(&render(&app), &["Absendername", "Neues Passwort", "IMAP-Verschlüsselung", "‹ SSL/TLS ›"]);

    // j and k are letters here.
    type_text(&mut app, "jk");
    assert_eq!(app.draft.as_ref().expect("editing").name, "Torrojk");
    for _ in 0..3 {
        press(&mut app, KeyCode::Down);
    }
    for _ in 0..3 {
        press(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "x");
    ctrl(&mut app, 's');
    assert!(app.request.is_none());
    assert_shows(&render(&app), &["Ein Port ist eine Zahl"]);

    press(&mut app, KeyCode::Backspace);
    type_text(&mut app, "143");
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
    type_text(&mut app, "new-secret");
    assert!(!render(&app).contains("new-secret"));
    ctrl(&mut app, 's');
    scene.act(&mut app);

    assert_shows(&render(&app), &["Gespeichert. Die Verbindung funktioniert.", "imap.example.org:143 · STARTTLS"]);
    assert_eq!(app.focus, Focus::List);
    let id = app.snapshot.accounts[0].account.id.clone();
    assert_eq!(scene.backend.secrets.get("TorroMail", &id).expect("readable").as_deref(), Some("new-secret"));
    assert_eq!(scene.policy()["accounts"][0]["imap"]["port"], 143);
    assert_eq!(scene.policy()["accounts"][0]["imap"]["security"], "starttls");
}

#[test]
fn folder_rules_and_special_folders_are_edited_against_the_servers_own_folders() {
    let scene = scene("edit-folders");
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert_shows(&render(&app), &["Ordner werden geladen"]);
    scene.act(&mut app);
    assert_eq!(app.focus, Focus::Detail);
    assert_shows(&render(&app), &["INBOX", "Entwürfe", "Privat", "kein Zugriff", "‹ Entwürfe ›"]);

    // INBOX: standard → read only.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    // "Privat" was "no access": one more step returns it to standard.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    // Sent: automatic → the first folder the server has.
    for _ in 0..2 {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Right);
    assert_shows(&render(&app), &["angepasst · nur lesen", "‹ INBOX ›"]);

    ctrl(&mut app, 's');
    scene.act(&mut app);
    let policy = scene.policy();
    assert_eq!(policy["accounts"][0]["folder_rules"], json!({ "INBOX": { "read": true, "write": false } }));
    assert_eq!(policy["accounts"][0]["mailbox_overrides"], json!({ "drafts": "Entw&APw-rfe", "sent": "INBOX" }));
    assert_eq!(app.snapshot.accounts[0].account.known_mailboxes, ["INBOX", "Entw&APw-rfe", "Privat"]);
}

#[test]
fn folders_that_cannot_be_listed_leave_the_tab_as_it_was() {
    let mut scene = scene("edit-folders-fail");
    scene.backend.mailbox_lister = Box::new(|_, _, _, _| Err("[AUTHENTICATIONFAILED] no".to_owned()));
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    scene.act(&mut app);
    assert_eq!(app.focus, Focus::List);
    assert_shows(&render(&app), &["Ordner konnten nicht geladen werden", "[AUTHENTICATIONFAILED] no"]);
}

#[test]
fn the_cache_level_steps_between_its_ends_and_is_published() {
    let scene = scene("edit-cache");
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Enter);
    for _ in 0..5 {
        press(&mut app, KeyCode::Left);
    }
    assert_shows(&render(&app), &["‹ Aus ›", "Ungespeicherte Änderungen"]);
    press(&mut app, KeyCode::Right);
    ctrl(&mut app, 's');
    scene.act(&mut app);
    assert_shows(&render(&app), &["Gespeichert.", "Betreff & Absender"]);
    assert_eq!(scene.policy()["accounts"][0]["cache"]["level"], "headers");
}


// MARK: the cache

#[test]
fn the_cache_tab_shows_the_real_footprint() {
    let mut app = App::new(Lang::De, snapshot());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::BackTab);
    assert_shows(&render(&app), &["Speicher", "3.4 MB", " R ", "Neu aufbauen"]);
}

#[test]
fn a_rebuild_runs_beside_the_surface_and_reports_as_it_goes() {
    use torromail_tui::rebuild::{self, Progress};
    assert_eq!(
        rebuild::parse_line(r#"{"phase":"headers","mailbox":"INBOX","done":120,"total":480}"#),
        Some(Progress::Working { phase: "headers".to_owned(), mailbox: "INBOX".to_owned(), done: 120, total: 480 })
    );
    assert_eq!(
        rebuild::parse_line(r#"{"phase":"done","messages":480,"size_bytes":1234}"#),
        Some(Progress::Finished { messages: 480, size_bytes: 1234 })
    );
    assert_eq!(rebuild::parse_line("not a progress line"), None);

    // A stand-in for the server: two progress lines with a pause between, so
    // the surface is seen mid-rebuild, then the closing line.
    let mut scene = scene("rebuild");
    scene.backend.rebuild_command = Box::new(|_, _, _, account| {
        assert!(!account.is_empty());
        let mut command = std::process::Command::new("sh");
        command.args(["-c", r#"echo '{"phase":"headers","mailbox":"INBOX","done":120,"total":480}'; sleep 0.4; echo '{"phase":"done","messages":480,"size_bytes":1234}'"#]);
        command
    });
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Char('R'));
    scene.act(&mut app);
    assert!(app.rebuild.is_some());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut saw_progress = false;
    while app.rebuild.is_some() && std::time::Instant::now() < deadline {
        torromail_tui::tick(&scene.backend, &mut app);
        if render(&app).contains("INBOX 120/480") {
            saw_progress = true;
            // A second rebuild cannot be started on top of a running one.
            press(&mut app, KeyCode::Char('R'));
            assert!(app.request.is_none());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(saw_progress, "the progress line was shown while it ran");
    assert!(app.rebuild.is_none(), "and the rebuild ended");
    assert_shows(&render(&app), &["Der Cache wurde neu aufgebaut."]);
}

#[test]
fn a_rebuild_that_fails_says_why() {
    let mut scene = scene("rebuild-fails");
    scene.backend.rebuild_command = Box::new(|_, _, _, _| {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "echo '[AUTHENTICATIONFAILED] no' >&2; exit 1"]);
        command
    });
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Char('R'));
    scene.act(&mut app);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.rebuild.is_some() && std::time::Instant::now() < deadline {
        torromail_tui::tick(&scene.backend, &mut app);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_shows(&render(&app), &["Der Cache konnte nicht neu aufgebaut werden.", "[AUTHENTICATIONFAILED] no"]);
}


#[test]
fn pi_says_which_extension_it_needs_and_how_to_get_it() {
    let mut app = App::new(Lang::De, snapshot());
    press(&mut app, KeyCode::Char('3'));
    while app.snapshot.clients[app.client_index].descriptor.id != "pi" {
        press(&mut app, KeyCode::Down);
    }
    assert_shows(&render(&app), &["Pi", "Braucht die Erweiterung pi-mcp-adapter", "pi install npm:pi-mcp-adapter"]);
}


// MARK: settings

#[test]
fn the_settings_screen_has_settings() {
    let app = {
        let mut app = App::new(Lang::De, snapshot());
        press(&mut app, KeyCode::Char('4'));
        app
    };
    assert_shows(
        &render(&app),
        &["Sprache", "‹ System ›", "[ ] Konten alle 15 Minuten prüfen", "[✓] Benachrichtigen, wenn ein Konto ausfällt", "Assistenten einrichten", "Updates öffnen"],
    );
}

#[test]
fn the_language_changes_at_once_and_is_remembered() {
    let scene = scene("settings-language");
    let mut app = App::new(Lang::En, scene.backend.load());
    press(&mut app, KeyCode::Char('4'));
    assert_shows(&render(&app), &["Language", "Settings"]);

    press(&mut app, KeyCode::Char(' '));
    scene.act(&mut app);
    assert_shows(&render(&app), &["Sprache", "‹ Deutsch ›", "Einstellungen"]);
    let remembered = torromail_tui::settings::Settings::load(&scene.backend.data_directory);
    assert_eq!(remembered.language, Some(Lang::De));
    assert_eq!(remembered.lang(), Lang::De);

    press(&mut app, KeyCode::Right);
    scene.act(&mut app);
    assert_shows(&render(&app), &["‹ English ›", "Settings"]);
    press(&mut app, KeyCode::Right);
    scene.act(&mut app);
    assert_eq!(torromail_tui::settings::Settings::load(&scene.backend.data_directory).language, None, "and back to the system's");
}

#[test]
fn the_background_check_installs_a_timer_and_removes_it_again() {
    use std::sync::{Arc, Mutex};
    let mut scene = scene("settings-autocheck");
    let calls: Arc<Mutex<Vec<String>>> = Arc::default();
    let recorded = Arc::clone(&calls);
    scene.backend.systemctl = Box::new(move |arguments| {
        recorded.lock().expect("not poisoned").push(arguments.join(" "));
        Ok(())
    });
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('4'));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    scene.act(&mut app);

    let units = scene.root.join("units");
    let service = std::fs::read_to_string(units.join("torromail-check.service")).expect("a service unit");
    assert!(service.contains("\" check\n"), "it runs this program's `check`: {service}");
    assert!(service.contains("ExecStart=\"/"), "by absolute path — a unit has no PATH: {service}");
    let timer = std::fs::read_to_string(units.join("torromail-check.timer")).expect("a timer unit");
    assert!(timer.contains("OnUnitActiveSec=15min") && timer.contains("WantedBy=timers.target"));
    assert_eq!(*calls.lock().expect("not poisoned"), ["daemon-reload", "enable --now torromail-check.timer"]);
    assert_shows(&render(&app), &["[✓] Konten alle 15 Minuten prüfen", "alle 15 Minuten geprüft"]);

    press(&mut app, KeyCode::Char(' '));
    scene.act(&mut app);
    assert!(!units.join("torromail-check.timer").exists() && !units.join("torromail-check.service").exists());
    assert_shows(&render(&app), &["[ ] Konten alle 15 Minuten prüfen", "Die Prüfung im Hintergrund ist aus."]);
}

#[test]
fn a_timer_that_would_not_start_does_not_look_enabled() {
    let mut scene = scene("settings-autocheck-fails");
    scene.backend.systemctl = Box::new(|arguments| {
        if arguments.first() == Some(&"enable") { Err("Failed to connect to bus".to_owned()) } else { Ok(()) }
    });
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('4'));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    scene.act(&mut app);
    assert_shows(&render(&app), &["[ ] Konten alle 15 Minuten prüfen", "ließ sich nicht ändern", "Failed to connect to bus"]);
    assert!(!scene.root.join("units/torromail-check.timer").exists());
}

#[test]
fn notifications_are_a_remembered_switch_and_the_jumps_jump() {
    let scene = scene("settings-rest");
    let mut app = App::new(Lang::De, scene.backend.load());
    // German by choice, not by whatever locale the tests happen to run under.
    app.settings.language = Some(Lang::De);
    press(&mut app, KeyCode::Char('4'));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    scene.act(&mut app);
    assert_shows(&render(&app), &["[ ] Benachrichtigen, wenn ein Konto ausfällt"]);
    assert!(!torromail_tui::settings::Settings::load(&scene.backend.data_directory).notifications);

    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.section, Section::Clients);
    press(&mut app, KeyCode::Char('4'));
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.section, Section::Updates);
}

#[test]
fn the_background_pass_notifies_on_a_crossing_and_only_on_a_crossing() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use torromail_tui::check;
    let mut scene = scene("check");
    let notes: RefCell<Vec<String>> = RefCell::default();
    let notify = |title: &str, body: &str| notes.borrow_mut().push(format!("{title}: {body}"));
    let verdict = Rc::new(RefCell::new(CheckOutcome::Ok));
    let answer = Rc::clone(&verdict);
    scene.backend.checker = Box::new(move |_, _, _, account| {
        // Only "work" ever fails; the other account stays fine throughout.
        if account == "work" { answer.borrow().clone() } else { CheckOutcome::Ok }
    });

    let first = check::run(&scene.backend, Lang::De, Some(&notify));
    assert_eq!((first.checked, first.broke.len(), first.recovered.len()), (2, 0, 0));
    assert!(notes.borrow().is_empty(), "all is well: nothing to say");

    *verdict.borrow_mut() = CheckOutcome::Rejected("[AUTHENTICATIONFAILED] no".to_owned());
    let broke = check::run(&scene.backend, Lang::De, Some(&notify));
    assert_eq!(broke.broke, ["Torro"]);
    assert_eq!(*notes.borrow(), ["Torro: TorroMail erreicht dieses Konto nicht mehr. [AUTHENTICATIONFAILED] no"]);

    check::run(&scene.backend, Lang::De, Some(&notify));
    assert_eq!(notes.borrow().len(), 1, "a standing problem is not news");

    *verdict.borrow_mut() = CheckOutcome::Ok;
    let recovered = check::run(&scene.backend, Lang::De, Some(&notify));
    assert_eq!(recovered.recovered, ["Torro"]);
    assert_eq!(notes.borrow().last().map(String::as_str), Some("Torro: Wieder erreichbar."));

    // Switched off, the pass still checks and records — it just stays quiet.
    *verdict.borrow_mut() = CheckOutcome::Rejected("no".to_owned());
    let quiet = check::run(&scene.backend, Lang::De, None);
    assert_eq!(quiet.broke, ["Torro"]);
    assert_eq!(notes.borrow().len(), 2);
    let log = std::fs::read_to_string(scene.root.join("data/health.jsonl")).expect("records were written");
    assert!(log.contains(r#""source":"periodic""#));
}


// MARK: what a connected assistant still has to do

#[test]
fn a_fresh_key_asks_for_a_restart_until_the_assistant_connects_again() {
    let scene = scene("restart-pending");
    let mut app = scene.app_on_cursor();
    assert_shows(&render_at(&app, 112, 50), &["In Cursor prüfen", "Einstellungen → MCP"]);
    assert!(!render(&app).contains("Starte Cursor jetzt neu"));

    press(&mut app, KeyCode::Char('c'));
    scene.act(&mut app);
    assert_shows(&render(&app), &["Starte Cursor jetzt neu", "verwendet weiterhin den alten"]);

    // The server saw Cursor connect after the key was written: proof enough.
    let later = torromail_tui::data::now() + 5;
    std::fs::write(
        scene.root.join("data/connections.jsonl"),
        format!(r#"{{"ts":{later},"client_id":"cursor","client_name":"cursor","client_version":"1.0"}}"#) + "\n",
    )
    .expect("writable");
    app.replace_snapshot(scene.backend.load());
    assert!(!render(&app).contains("Starte Cursor jetzt neu"), "the warning clears by itself");
}


// MARK: updates and the log export

#[test]
fn u_asks_for_the_newest_release_and_says_which_way_it_compares() {
    let mut scene = scene("updates");
    let mut app = App::new(Lang::De, scene.backend.load());
    app.settings.language = Some(Lang::De);
    press(&mut app, KeyCode::Char('5'));
    assert_shows(&render(&app), &["Installierte Version", " u ", "Jetzt nach Updates suchen"]);

    press(&mut app, KeyCode::Char('u'));
    scene.act(&mut app);
    assert_shows(&render(&app), &["Du bist auf dem neuesten Stand. (v0.0.1)", "Zuletzt geprüft: gerade eben"]);

    scene.backend.release_lookup = Box::new(|| Ok("v99.0.0".to_owned()));
    press(&mut app, KeyCode::Char('u'));
    scene.act(&mut app);
    assert_shows(&render(&app), &["Eine neuere Version ist verfügbar: v99.0.0", "scripts/install-linux.sh"]);

    scene.backend.release_lookup = Box::new(|| Err("no route to host".to_owned()));
    press(&mut app, KeyCode::Char('u'));
    scene.act(&mut app);
    assert_shows(&render(&app), &["Die Suche nach Updates ist fehlgeschlagen.", "no route to host"]);
}

#[test]
fn e_writes_the_log_as_csv_and_says_where() {
    let scene = scene("export");
    std::fs::write(
        scene.root.join("data/audit.jsonl"),
        concat!(
            r#"{"ts":100,"client":"Codex","account":"work","tool":"mail_search","detail":"=cmd","result":"ok"}"#, "\n",
            r#"{"ts":101,"client":"Codex","account":"","tool":"mail_list_accounts","detail":"","result":"ok"}"#, "\n",
        ),
    )
    .expect("writable");
    let mut app = App::new(Lang::De, scene.backend.load());
    press(&mut app, KeyCode::Char('6'));
    assert_shows(&render(&app), &[" e ", "exportieren"]);
    press(&mut app, KeyCode::Char('e'));
    scene.act(&mut app);

    assert_shows(&render(&app), &["Exportiert:", "torromail-log-"]);
    let exported = std::fs::read_dir(scene.root.join("exports")).expect("the directory was made").flatten().next().expect("one file");
    let csv = std::fs::read_to_string(exported.path()).expect("readable");
    assert_eq!(csv.lines().count(), 3, "a header and both entries");
    assert!(csv.contains(r#""'=cmd""#), "a formula is defused: {csv}");
}
