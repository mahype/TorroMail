//! What the user is looking at, and what a key does to it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::data::Snapshot;
use crate::i18n::Lang;

/// The same seven places, in the same order, as the macOS app's sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Overview,
    Accounts,
    Clients,
    Settings,
    Updates,
    Log,
    Help,
}

impl Section {
    pub const ALL: [Self; 7] =
        [Self::Overview, Self::Accounts, Self::Clients, Self::Settings, Self::Updates, Self::Log, Self::Help];

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Accounts => "Mail Accounts",
            Self::Clients => "MCP Clients",
            Self::Settings => "Settings",
            Self::Updates => "Updates",
            Self::Log => "Log",
            Self::Help => "Help",
        }
    }
}

pub const ACCOUNT_TABS: [&str; 4] = ["Connection", "Permissions", "Folders", "Search & Cache"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogColumn {
    Time,
    Client,
    Account,
    Event,
    Details,
    Result,
}

impl LogColumn {
    pub const ALL: [Self; 6] = [Self::Time, Self::Client, Self::Account, Self::Event, Self::Details, Self::Result];

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::Client => "Client",
            Self::Account => "Account",
            Self::Event => "Event",
            Self::Details => "Details",
            Self::Result => "Result",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub lang: Lang,
    pub snapshot: Snapshot,
    pub section: Section,
    pub account_index: usize,
    pub account_tab: usize,
    pub client_index: usize,
    pub log_index: usize,
    pub log_sort: LogColumn,
    /// Newest first is the default, as in the app.
    pub log_descending: bool,
    pub wants_reload: bool,
    pub should_quit: bool,
}

impl App {
    #[must_use]
    pub fn new(lang: Lang, snapshot: Snapshot) -> Self {
        Self {
            lang,
            snapshot,
            section: Section::Overview,
            account_index: 0,
            account_tab: 0,
            client_index: 0,
            log_index: 0,
            log_sort: LogColumn::Time,
            log_descending: true,
            wants_reload: false,
            should_quit: false,
        }
    }

    /// A fresh snapshot keeps the selection where it was, as far as it still
    /// exists: a reload must not throw the user back to the top of a list.
    pub fn replace_snapshot(&mut self, snapshot: Snapshot) {
        self.snapshot = snapshot;
        self.account_index = self.account_index.min(self.snapshot.accounts.len().saturating_sub(1));
        self.client_index = self.client_index.min(self.snapshot.clients.len().saturating_sub(1));
        self.log_index = self.log_index.min(self.snapshot.audit.len().saturating_sub(1));
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if matches!(key.code, KeyCode::Char('c' | 'q')) {
                self.should_quit = true;
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('r') if self.section != Section::Log => self.wants_reload = true,
            KeyCode::Char(digit @ '1'..='7') => {
                self.section = Section::ALL[digit as usize - '1' as usize];
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Home => self.move_selection(isize::MIN),
            KeyCode::End => self.move_selection(isize::MAX),
            KeyCode::Tab | KeyCode::Right if self.section == Section::Accounts => {
                self.account_tab = (self.account_tab + 1) % ACCOUNT_TABS.len();
            }
            KeyCode::BackTab | KeyCode::Left if self.section == Section::Accounts => {
                self.account_tab = (self.account_tab + ACCOUNT_TABS.len() - 1) % ACCOUNT_TABS.len();
            }
            KeyCode::Enter if self.section == Section::Overview => {
                // The attention card leads to where the repair is.
                if let Some(index) = self.snapshot.accounts.iter().position(|view| view.health.is_broken()) {
                    self.account_index = index;
                    self.account_tab = 0;
                    self.section = Section::Accounts;
                }
            }
            KeyCode::Char('s') if self.section == Section::Log => {
                let position = LogColumn::ALL.iter().position(|column| *column == self.log_sort).unwrap_or(0);
                self.log_sort = LogColumn::ALL[(position + 1) % LogColumn::ALL.len()];
                self.log_index = 0;
            }
            KeyCode::Char('r') if self.section == Section::Log => {
                self.log_descending = !self.log_descending;
                self.log_index = 0;
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let (index, count) = match self.section {
            Section::Accounts => (&mut self.account_index, self.snapshot.accounts.len()),
            Section::Clients => (&mut self.client_index, self.snapshot.clients.len()),
            Section::Log => (&mut self.log_index, self.snapshot.audit.len()),
            _ => return,
        };
        let last = count.saturating_sub(1);
        *index = index.saturating_add_signed(delta).min(last);
    }
}
