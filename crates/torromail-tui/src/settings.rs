//! What this surface remembers about itself, in `tui.json` beside the state.
//! Deliberately not in `state.json`: that file's `settings` belong to the
//! macOS app, whose decoder expects exactly its own keys.

use std::path::Path;

use crate::i18n::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// `None` follows the system (`LC_ALL`, `LC_MESSAGES`, `LANG`).
    pub language: Option<Lang>,
    /// Whether the background check may raise a desktop notification when an
    /// account stops working, and when it works again.
    pub notifications: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { language: None, notifications: true }
    }
}

pub const FILE: &str = "tui.json";

impl Settings {
    /// Never fails: a missing or unreadable file is the defaults.
    #[must_use]
    pub fn load(data_directory: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(data_directory.join(FILE)) else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Self::default();
        };
        Self {
            language: match value["language"].as_str() {
                Some("de") => Some(Lang::De),
                Some("en") => Some(Lang::En),
                _ => None,
            },
            notifications: value["notifications"].as_bool().unwrap_or(true),
        }
    }

    pub fn save(&self, data_directory: &Path) -> std::io::Result<()> {
        let language = match self.language {
            Some(Lang::De) => "de",
            Some(Lang::En) => "en",
            None => "system",
        };
        let document = serde_json::json!({ "language": language, "notifications": self.notifications });
        std::fs::create_dir_all(data_directory)?;
        std::fs::write(data_directory.join(FILE), serde_json::to_string_pretty(&document).map_err(std::io::Error::other)?)
    }

    /// The language to draw in.
    #[must_use]
    pub fn lang(&self) -> Lang {
        self.language.unwrap_or_else(Lang::from_environment)
    }

    /// System → Deutsch → English → System.
    #[must_use]
    pub fn next_language(&self) -> Option<Lang> {
        match self.language {
            None => Some(Lang::De),
            Some(Lang::De) => Some(Lang::En),
            Some(Lang::En) => None,
        }
    }
}
