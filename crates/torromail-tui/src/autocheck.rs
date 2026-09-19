//! Checking the accounts while nobody is looking. The macOS app does this
//! from its own background process; here a systemd user timer runs
//! `torromail check` every quarter of an hour — no daemon of ours to keep
//! alive, and nothing at all when the user turned it off.

use std::path::{Path, PathBuf};

pub const SERVICE: &str = "torromail-check.service";
pub const TIMER: &str = "torromail-check.timer";

/// Runs `systemctl --user <arguments>`. Injected so the unit files can be
/// tested without a systemd to talk to.
pub type Systemctl<'a> = &'a dyn Fn(&[&str]) -> Result<(), String>;

/// `$XDG_CONFIG_HOME/systemd/user`, falling back to `~/.config/systemd/user`.
#[must_use]
pub fn unit_directory(home: &Path, xdg_config_home: Option<&Path>) -> PathBuf {
    xdg_config_home
        .filter(|path| path.is_absolute())
        .map_or_else(|| home.join(".config"), Path::to_path_buf)
        .join("systemd/user")
}

#[must_use]
pub fn is_enabled(unit_directory: &Path) -> bool {
    unit_directory.join(TIMER).exists()
}

fn service_unit(program: &Path) -> String {
    format!(
        "[Unit]\nDescription=TorroMail: check that every mail account still works\n\n\
         [Service]\nType=oneshot\nExecStart=\"{}\" check\n",
        program.display()
    )
}

/// Two minutes after login, then every fifteen — the macOS app's cadence.
fn timer_unit() -> &'static str {
    "[Unit]\nDescription=TorroMail: check the mail accounts regularly\n\n\
     [Timer]\nOnStartupSec=2min\nOnUnitActiveSec=15min\n\n\
     [Install]\nWantedBy=timers.target\n"
}

/// Writes the two units and starts the timer. `program` is this very binary,
/// by absolute path: a unit has no shell PATH to find it on.
pub fn enable(unit_directory: &Path, program: &Path, systemctl: Systemctl<'_>) -> Result<(), String> {
    std::fs::create_dir_all(unit_directory).map_err(|error| error.to_string())?;
    std::fs::write(unit_directory.join(SERVICE), service_unit(program)).map_err(|error| error.to_string())?;
    std::fs::write(unit_directory.join(TIMER), timer_unit()).map_err(|error| error.to_string())?;
    let started = systemctl(&["daemon-reload"]).and_then(|()| systemctl(&["enable", "--now", TIMER]));
    if started.is_err() {
        // A timer that did not start must not look enabled next time.
        let _ = std::fs::remove_file(unit_directory.join(TIMER));
        let _ = std::fs::remove_file(unit_directory.join(SERVICE));
    }
    started
}

pub fn disable(unit_directory: &Path, systemctl: Systemctl<'_>) -> Result<(), String> {
    // Stopping may fail when it was never started; the files going is what
    // makes it off.
    let _ = systemctl(&["disable", "--now", TIMER]);
    for unit in [TIMER, SERVICE] {
        let path = unit_directory.join(unit);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
    }
    let _ = systemctl(&["daemon-reload"]);
    Ok(())
}

pub fn run_systemctl(arguments: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("systemctl")
        .arg("--user")
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("systemctl could not be started: {error}"))?;
    if output.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()) }
}
