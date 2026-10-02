//! The recovery command is dispatched before account files or keychain access.
//! A separate Sparkle process updates the installed app, using its feed/key.
use std::ffi::OsString;
#[cfg(any(test, target_os = "macos"))]
use std::path::Path;
use std::path::PathBuf;

pub const USAGE: &str = "Usage: torromail update [--check] [--interactive] [--allow-major-upgrades] [--app PATH]\n\
Updates the macOS app without opening its window. A running app is restarted.\n\
  --check                 Only check; do not download or install.\n\
  --interactive           Allow macOS to request installation authorization.\n\
  --allow-major-upgrades  Allow Sparkle major upgrades.\n\
  --app PATH              Target a TorroMail.app in another location.\n\
By default, use the app containing this CLI, or /Applications/TorroMail.app.\n\
Exit codes: 0 update found/installed; 4 already current; 1 failed; 2 invalid\n\
arguments/major upgrade blocked; 3 authorization needed; 5 authorization\n\
cancelled; 6 update permission needed; 8 cannot replace app.\n\
Linux/Windows: update through your package manager or GitHub releases.";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    check: bool,
    interactive: bool,
    allow_major: bool,
    app: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut result = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--check") => result.check = true,
                Some("--interactive") => result.interactive = true,
                Some("--allow-major-upgrades") => result.allow_major = true,
                Some("--app") => {
                    if result.app.is_some() {
                        return Err("Specify --app only once.".into());
                    }
                    let value = args.next().ok_or("--app needs a path.")?;
                    if value.is_empty() || value.to_string_lossy().starts_with("--") {
                        return Err("--app needs a path.".into());
                    }
                    result.app = Some(value.into());
                }
                _ => {
                    return Err(format!(
                        "Unknown update argument: {}",
                        arg.to_string_lossy()
                    ));
                }
            }
        }
        if result.check && result.interactive {
            return Err("--check cannot be combined with --interactive.".into());
        }
        Ok(result)
    }

    #[cfg(any(test, target_os = "macos"))]
    fn target(&self, executable: &Path) -> PathBuf {
        self.app
            .clone()
            .or_else(|| bundled_app(executable))
            .unwrap_or_else(|| PathBuf::from("/Applications/TorroMail.app"))
    }

    #[cfg(any(test, target_os = "macos"))]
    fn helper_args(&self, app: &Path) -> Vec<OsString> {
        let mut args = vec![app.as_os_str().to_owned()];
        if self.check {
            args.push("--check".into());
        }
        if self.interactive {
            args.push("--interactive".into());
        }
        if self.allow_major {
            args.push("--allow-major-upgrades".into());
        }
        args
    }
}

#[cfg(any(test, target_os = "macos"))]
fn bundled_app(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.to_owned())
}

pub fn run(args: Vec<OsString>) -> i32 {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("torromail update: {message}\n{USAGE}");
            return 2;
        }
    };
    #[cfg(target_os = "macos")]
    {
        let executable = match std::env::current_exe().and_then(|path| path.canonicalize()) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("torromail update: {error}");
                return 1;
            }
        };
        let target = options.target(&executable);
        let app = match target.canonicalize() {
            Ok(path) if path.is_dir() => path,
            _ => {
                eprintln!(
                    "torromail update: App not found at {}. Use --app PATH.",
                    target.display()
                );
                return 1;
            }
        };
        let helper =
            app.join("Contents/Helpers/TorroMailUpdater.app/Contents/MacOS/TorroMailUpdater");
        if !helper.is_file() {
            eprintln!(
                "torromail update: This app has no CLI updater. Install a newer official DMG once, then retry."
            );
            return 1;
        }
        match std::process::Command::new(helper)
            .args(options.helper_args(&app))
            .status()
        {
            Ok(status) => status.code().unwrap_or(1),
            Err(error) => {
                eprintln!("torromail update: Unable to start Sparkle: {error}");
                1
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = options;
        eprintln!(
            "torromail update: App updates are available on macOS. Use your package manager or GitHub releases on this platform."
        );
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(OsString::from))
    }
    #[test]
    fn rejects_invalid_options_before_starting_any_updater() {
        for args in [
            &["--feed-url", "https://example.com"][..],
            &["--app"],
            &["--app", "--check"],
            &["--app", ""],
            &["--app", "a", "--app", "b"],
            &["--check", "--interactive"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }
    #[test]
    fn relocated_bundle_wins_over_applications_default() {
        let options = Options::default();
        assert_eq!(
            options.target(Path::new(
                "/Users/me/My Apps/TorroMail.app/Contents/MacOS/torromail"
            )),
            PathBuf::from("/Users/me/My Apps/TorroMail.app")
        );
        assert_eq!(
            options.target(Path::new("/usr/local/bin/torromail")),
            PathBuf::from("/Applications/TorroMail.app")
        );
    }
    #[test]
    fn explicit_app_and_paths_with_spaces_are_passed_as_single_arguments() {
        let options = parse(&[
            "--app",
            "/tmp/Other App.app",
            "--interactive",
            "--allow-major-upgrades",
        ])
        .expect("valid options");
        let target = options.target(Path::new("/tmp/TorroMail.app/Contents/MacOS/torromail"));
        assert_eq!(target, PathBuf::from("/tmp/Other App.app"));
        assert_eq!(
            options.helper_args(&target),
            vec![
                OsString::from("/tmp/Other App.app"),
                OsString::from("--interactive"),
                OsString::from("--allow-major-upgrades")
            ]
        );
    }
    #[test]
    fn check_only_passes_no_installation_or_permission_flags() {
        let options = parse(&["--check"]).expect("check only");
        assert_eq!(
            options.helper_args(Path::new("/tmp/App.app")),
            vec![OsString::from("/tmp/App.app"), OsString::from("--check")]
        );
    }
}
