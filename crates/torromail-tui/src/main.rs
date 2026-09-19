use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use torromail_control::clients::Environment;
use torromail_control::paths;
use torromail_tui::app::App;
use torromail_tui::i18n::Lang;
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
    let environment = Environment::current();
    let mut app = App::new(Lang::from_environment(), data::load(&directory, environment.as_ref()));

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
        if let Some(account) = app.save_request.take() {
            match data::save_account(&directory, &account) {
                Ok(()) => {
                    app.replace_snapshot(data::load(&directory, environment.as_ref()));
                    app.saved();
                }
                Err(detail) => app.save_failed(detail),
            }
            loaded = Instant::now();
        }
        if app.wants_reload || loaded.elapsed() >= REFRESH {
            app.wants_reload = false;
            app.replace_snapshot(data::load(&directory, environment.as_ref()));
            loaded = Instant::now();
        }
    };
    ratatui::restore();
    outcome
}
