use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use torromail_control::clients::Environment;
use torromail_control::paths;
use torromail_tui::app::App;
use torromail_tui::{data, ui};

/// How often the files are read again while nothing is pressed. The server
/// appends to the logs at any moment; this is what keeps the screens current.
const REFRESH: Duration = Duration::from_secs(2);

fn main() -> std::io::Result<()> {
    if std::env::args().any(|argument| argument == "--version" || argument == "-V") {
        println!("torromail {}", ui::VERSION);
        return Ok(());
    }
    // No home means no place the files could be. Inventing one would show an
    // empty, healthy-looking surface over a broken setup.
    let Some(directory) = paths::default_data_directory() else {
        eprintln!("torromail: HOME is not set, so there is nowhere to look for the TorroMail data.");
        std::process::exit(1);
    };
    let home = torromail_control::paths::home_directory().unwrap_or_default();
    let xdg_config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let backend = data::Backend {
        data_directory: directory,
        environment: Environment::current(),
        secrets: torromail_control::secrets::platform_store(),
        checker: Box::new(data::check_account),
        discoverer: Box::new(torromail_discovery::discover),
        mailbox_lister: Box::new(data::list_account_mailboxes),
        rebuild_command: Box::new(data::rebuild_command),
        rebuild: std::cell::RefCell::new(None),
        tool_count: std::cell::OnceCell::new(),
        unit_directory: torromail_tui::autocheck::unit_directory(&home, xdg_config.as_deref()),
        systemctl: Box::new(torromail_tui::autocheck::run_systemctl),
        release_lookup: Box::new(|| torromail_discovery::latest_release_tag("mahype/TorroMail")),
        // Downloads when there is one — where people look for a file they
        // just asked for — otherwise the home directory.
        export_directory: Some(home.join("Downloads")).filter(|path| path.is_dir()).unwrap_or_else(|| home.clone()),
    };
    let settings = torromail_tui::settings::Settings::load(&backend.data_directory);

    // `torromail check`: one pass over the accounts, no screen. What the
    // systemd timer runs.
    if std::env::args().nth(1).as_deref() == Some("check") {
        let notify: Option<torromail_tui::check::Notify<'_>> =
            if settings.notifications { Some(&torromail_tui::check::notify_send) } else { None };
        let summary = torromail_tui::check::run(&backend, settings.lang(), notify);
        println!("checked {}, broke {}, recovered {}", summary.checked, summary.broke.len(), summary.recovered.len());
        return Ok(());
    }

    // `torromail status [--json]`: the glance, for a shell or a bar widget.
    if std::env::args().nth(1).as_deref() == Some("status") {
        let snapshot = backend.load();
        if std::env::args().any(|argument| argument == "--json") {
            println!("{}", torromail_tui::status::json(&snapshot, ui::VERSION, settings.lang()));
        } else {
            println!("{}", torromail_tui::status::plain(&snapshot));
        }
        return Ok(());
    }

    let mut app = App::new(settings.lang(), backend.load());
    app.settings = settings;

    let mut terminal = ratatui::init();
    let mut loaded = Instant::now();
    let outcome = loop {
        if let Err(error) = terminal.draw(|frame| ui::draw(frame, &app)) {
            break Err(error);
        }
        match event::poll(Duration::from_millis(250)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => app.on_key(key),
                Ok(_) => {}
                Err(error) => break Err(error),
            },
            Ok(false) => {}
            Err(error) => break Err(error),
        }
        if app.should_quit {
            break Ok(());
        }
        if let Some(request) = app.request.take() {
            // A check can take a while: say so on screen before it starts.
            if let Err(error) = terminal.draw(|frame| ui::draw(frame, &app)) {
                break Err(error);
            }
            torromail_tui::perform(&backend, &mut app, request);
            loaded = Instant::now();
        }
        torromail_tui::tick(&backend, &mut app);
        if app.wants_reload || loaded.elapsed() >= REFRESH {
            app.wants_reload = false;
            app.replace_snapshot(backend.load());
            loaded = Instant::now();
        }
    };
    ratatui::restore();
    outcome
}
