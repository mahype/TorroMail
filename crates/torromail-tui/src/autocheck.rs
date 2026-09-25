//! Checking the accounts while nobody is looking. The system's own scheduler
//! runs `torromail check` every quarter of an hour — a systemd user timer on
//! Linux, a launchd agent on macOS — so there is no daemon of ours to keep
//! alive, and nothing at all when the user turned it off. (The macOS app has a
//! background monitor of its own; both append to the same health log, so
//! running the two side by side only checks more often.)

use std::path::{Path, PathBuf};

pub const SERVICE: &str = "torromail-check.service";
pub const TIMER: &str = "torromail-check.timer";

/// The launchd job on macOS, and the file it is described in.
pub const LAUNCH_AGENT: &str = "com.torromail.check";
pub const LAUNCH_AGENT_FILE: &str = "com.torromail.check.plist";

/// Runs the scheduler's command — `systemctl --user <arguments>` on Linux,
/// `launchctl <arguments>` on macOS. Injected so the job files can be tested
/// without a scheduler to talk to.
pub type Systemctl<'a> = &'a dyn Fn(&[&str]) -> Result<(), String>;

/// Where the job files live: `~/Library/LaunchAgents` on macOS, elsewhere
/// `$XDG_CONFIG_HOME/systemd/user`, falling back to `~/.config/systemd/user`.
#[must_use]
pub fn unit_directory(home: &Path, xdg_config_home: Option<&Path>) -> PathBuf {
    if cfg!(target_os = "macos") {
        return home.join("Library/LaunchAgents");
    }
    xdg_config_home
        .filter(|path| path.is_absolute())
        .map_or_else(|| home.join(".config"), Path::to_path_buf)
        .join("systemd/user")
}

#[must_use]
pub fn is_enabled(unit_directory: &Path) -> bool {
    unit_directory.join(if cfg!(target_os = "macos") { LAUNCH_AGENT_FILE } else { TIMER }).exists()
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

/// The launchd agent: `torromail check` at login, then every fifteen minutes
/// — the same cadence as the timer. `program` goes in by absolute path.
fn launch_agent(program: &Path) -> String {
    let program = program.display().to_string().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{LAUNCH_AGENT}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{program}</string>\n\t\t<string>check</string>\n\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>StartInterval</key>\n\t<integer>900</integer>\n\
         \t<key>ProcessType</key>\n\t<string>Background</string>\n\
         </dict>\n</plist>\n"
    )
}

/// The launchd domain of whoever owns the agent directory — the user, since
/// it is in their home. Read from the file system because the process has no
/// safe way to ask for its own uid.
#[cfg(unix)]
fn gui_domain(launch_agents: &Path) -> Result<String, String> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata(launch_agents).map_err(|error| error.to_string())?.uid();
    Ok(format!("gui/{uid}"))
}

#[cfg(not(unix))]
fn gui_domain(_launch_agents: &Path) -> Result<String, String> {
    Err("launchd exists only on macOS".to_owned())
}

/// Writes the launchd agent and loads it. A stale one from an earlier
/// install is unloaded first, so the new path is the one that runs.
pub fn enable_launch_agent(launch_agents: &Path, program: &Path, launchctl: Systemctl<'_>) -> Result<(), String> {
    std::fs::create_dir_all(launch_agents).map_err(|error| error.to_string())?;
    let file = launch_agents.join(LAUNCH_AGENT_FILE);
    std::fs::write(&file, launch_agent(program)).map_err(|error| error.to_string())?;
    let domain = gui_domain(launch_agents)?;
    let _ = launchctl(&["bootout", &format!("{domain}/{LAUNCH_AGENT}")]);
    let started = launchctl(&["bootstrap", &domain, &file.to_string_lossy()]);
    if started.is_err() {
        // An agent that did not load must not look enabled next time.
        let _ = std::fs::remove_file(&file);
    }
    started
}

pub fn disable_launch_agent(launch_agents: &Path, launchctl: Systemctl<'_>) -> Result<(), String> {
    // Unloading fails when it was never loaded; the file going is what makes
    // it off.
    if let Ok(domain) = gui_domain(launch_agents) {
        let _ = launchctl(&["bootout", &format!("{domain}/{LAUNCH_AGENT}")]);
    }
    let file = launch_agents.join(LAUNCH_AGENT_FILE);
    if file.exists() {
        std::fs::remove_file(&file).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn run_launchctl(arguments: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("launchctl")
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("launchctl could not be started: {error}"))?;
    if output.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()) }
}

/// The scheduler this platform has.
pub fn run_scheduler(arguments: &[&str]) -> Result<(), String> {
    if cfg!(target_os = "macos") { run_launchctl(arguments) } else { run_systemctl(arguments) }
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
