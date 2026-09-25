//! What the user is looking at, and what a key does to it.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use torromail_control::policy::ClientAccountAccess;
use torromail_control::{CacheLevel, ConnectionSecurity, FolderRule, MailAccount, PermissionPreset, ReadAccess};

use crate::data::Snapshot;
use crate::i18n::Lang;
use crate::input;
use crate::wizard::{Outcome, Secret, Wizard};

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

/// Where the keys go inside a section with a list and a detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    /// Editing: an account's permissions, or a client's account access.
    Detail,
}

/// Something the event loop has to do to the world. The app only asks; it
/// never writes a file or starts a process itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// The account, and a new password for it when one was typed.
    SaveAccount(Box<MailAccount>, Option<Secret>),
    /// Log in and list the account's folders, to edit folder rules with.
    LoadMailboxes(String),
    RebuildCache(String),
    Connect(String),
    Disconnect(String),
    SetAccess(String, ClientAccountAccess),
    /// Put the snippet, real key included, on the clipboard.
    CopySnippet(String),
    /// Fetch the key so the snippet on screen can show it.
    RevealKey(String),
    /// Check the candidate and, if it passes, store it.
    Enroll(Box<MailAccount>, Secret),
    TestConnection(String),
    RemoveAccount(String),
    /// Find the servers for an address the provider table does not know.
    Discover(String),
    SaveSettings(Box<crate::settings::Settings>),
    /// Ask GitHub whether a newer release exists.
    CheckForUpdates,
    /// Write the log to a CSV file.
    ExportLog,
    /// Install or remove the timer behind the background check.
    SetAutocheck(bool),
}

/// The rows of the settings section, top to bottom.
pub const SETTINGS_ROWS: usize = 5;

/// The editable rows of the permissions tab, top to bottom.
pub const PERMISSION_ROWS: usize = 11;
/// Connection tab: name, username, IMAP host/port/encryption, SMTP
/// host/port/encryption, new password.
pub const CONNECTION_ROWS: usize = 9;
const ROW_IMAP_SECURITY: usize = 4;
const ROW_SMTP_SECURITY: usize = 7;
pub const SPECIAL_ROLES: usize = 5;

/// What an account draft cannot hold while it is being typed: ports that are
/// not numbers yet, a password that is not stored yet, and the folders the
/// server reported for the pickers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditExtras {
    pub imap_port: String,
    pub smtp_port: String,
    pub password: Secret,
    pub folders: Vec<String>,
}
const ROW_READ_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: &'static str,
    pub detail: String,
    pub is_error: bool,
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
    pub focus: Focus,
    /// The account being edited, apart from the stored one until it is saved.
    pub draft: Option<MailAccount>,
    pub cursor: usize,
    /// Where the caret sits in the text row under the cursor, in characters
    /// from its end.
    pub caret_back: usize,
    pub confirm_discard: bool,
    pub message: Option<Message>,
    /// What the event loop should do next; it reports back through
    /// `succeeded` / `failed`.
    pub request: Option<Request>,
    /// A client's account access while it is being edited.
    pub access_draft: Option<ClientAccountAccess>,
    pub confirm_disconnect: bool,
    /// The one client whose key is currently shown in clear, with the key.
    pub revealed: Option<(String, String)>,
    /// The add-account wizard, while it is open. It takes every key.
    pub wizard: Option<Wizard>,
    pub confirm_remove: bool,
    pub extras: EditExtras,
    /// How far the detail pane is scrolled; it can be taller than the terminal.
    pub detail_scroll: u16,
    /// The account whose cache is being rebuilt, and the last progress seen.
    pub rebuild: Option<(String, Option<crate::rebuild::Progress>)>,
    pub settings: crate::settings::Settings,
    pub settings_index: usize,
    /// When a client's key was last written in this session. An assistant
    /// reads its key once, at start, so until it has connected *after* this
    /// moment it is still presenting the old one.
    pub key_changed: std::collections::HashMap<String, u64>,
    /// The newest release seen, and when it was asked for (Unix seconds).
    pub update: Option<(String, u64)>,
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
            focus: Focus::List,
            draft: None,
            cursor: 0,
            caret_back: 0,
            confirm_discard: false,
            message: None,
            request: None,
            access_draft: None,
            confirm_disconnect: false,
            revealed: None,
            wizard: None,
            confirm_remove: false,
            extras: EditExtras::default(),
            rebuild: None,
            settings: crate::settings::Settings::default(),
            settings_index: 0,
            key_changed: std::collections::HashMap::new(),
            update: None,
            detail_scroll: 0,
        }
    }

    /// Whether the client still has to be restarted before the key written
    /// for it takes effect. A connection older than the change proves nothing;
    /// one after it is the proof, which is why this clears by itself.
    #[must_use]
    pub fn restart_pending(&self, client: &crate::data::ClientView) -> bool {
        self.key_changed.get(client.descriptor.id).is_some_and(|changed| {
            client.connection.as_ref().is_none_or(|connection| (connection.last_connected as u64) < *changed)
        })
    }

    /// Whether a draft differs from what is stored.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        if let (Some(draft), Some(client)) = (&self.access_draft, self.snapshot.clients.get(self.client_index)) {
            return client.access.as_ref() != Some(draft);
        }
        match (&self.draft, self.snapshot.accounts.get(self.account_index)) {
            (Some(draft), Some(stored)) => {
                *draft != stored.account
                    || !self.extras.password.0.is_empty()
                    || self.extras.imap_port != stored.account.imap_port.to_string()
                    || self.extras.smtp_port != stored.account.smtp_port.to_string()
            }
            _ => false,
        }
    }

    fn begin_account_edit(&mut self, folders: Vec<String>) {
        let Some(view) = self.snapshot.accounts.get(self.account_index) else { return };
        self.extras = EditExtras {
            imap_port: view.account.imap_port.to_string(),
            smtp_port: view.account.smtp_port.to_string(),
            password: Secret::default(),
            folders,
        };
        self.draft = Some(view.account.clone());
        self.focus = Focus::Detail;
        self.cursor = 0;
        self.caret_back = 0;
        self.detail_scroll = 0;
    }

    /// The event loop listed the account's folders: the folders tab can be
    /// edited now.
    pub fn mailboxes_loaded(&mut self, folders: Vec<String>) {
        self.message = None;
        self.begin_account_edit(folders);
    }

    /// The account as it would be saved, or why it cannot be yet.
    fn account_to_save(&self) -> Result<MailAccount, &'static str> {
        let mut account = self.draft.clone().ok_or("Nothing to save.")?;
        account.imap_port = self.extras.imap_port.trim().parse().map_err(|_| "A port is a number between 1 and 65535.")?;
        account.smtp_port = self.extras.smtp_port.trim().parse().map_err(|_| "A port is a number between 1 and 65535.")?;
        if account.imap_port == 0 || account.smtp_port == 0 {
            return Err("A port is a number between 1 and 65535.");
        }
        if !self.extras.folders.is_empty() {
            account.known_mailboxes = self.extras.folders.clone();
        }
        Ok(account)
    }

    fn rows(&self) -> usize {
        match (self.section, self.account_tab) {
            (Section::Clients, _) => 2 + self.snapshot.accounts.len(),
            (_, 0) => CONNECTION_ROWS,
            (_, 1) => PERMISSION_ROWS,
            (_, 2) => 1 + self.extras.folders.len() + SPECIAL_ROLES,
            _ => 1,
        }
    }

    /// The text under the cursor on the connection tab, if it is on text.
    fn connection_text(&mut self) -> Option<&mut String> {
        let draft = self.draft.as_mut()?;
        match self.cursor {
            0 => Some(&mut draft.name),
            1 => Some(&mut draft.username),
            2 => Some(&mut draft.imap_host),
            3 => Some(&mut self.extras.imap_port),
            5 => Some(&mut draft.smtp_host),
            6 => Some(&mut self.extras.smtp_port),
            8 => Some(&mut self.extras.password.0),
            _ => None,
        }
    }

    fn on_connection_key(&mut self, code: KeyCode) {
        let on_security = matches!(self.cursor, ROW_IMAP_SECURITY | ROW_SMTP_SECURITY);
        match code {
            KeyCode::Up | KeyCode::BackTab => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab | KeyCode::Enter => self.cursor = (self.cursor + 1).min(CONNECTION_ROWS - 1),
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if on_security => {
                if let Some(draft) = &mut self.draft {
                    let security = if self.cursor == ROW_IMAP_SECURITY { &mut draft.imap_security } else { &mut draft.smtp_security };
                    *security = match security {
                        ConnectionSecurity::Tls => ConnectionSecurity::StartTls,
                        ConnectionSecurity::StartTls => ConnectionSecurity::Tls,
                    };
                }
            }
            code => {
                let mut caret_back = self.caret_back;
                if let Some(text) = self.connection_text() {
                    input::edit(text, &mut caret_back, code);
                }
                self.caret_back = caret_back;
                return;
            }
        }
        // Another row takes the caret to its end.
        self.caret_back = 0;
    }

    /// Row 0 is the per-folder switch, then the folders, then the five roles.
    fn on_folders_key(&mut self, code: KeyCode) {
        let folders = self.extras.folders.clone();
        let cursor = self.cursor;
        let Some(draft) = &mut self.draft else { return };
        let step: isize = match code {
            KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Right => 1,
            KeyCode::Left => -1,
            _ => return,
        };
        if cursor == 0 {
            draft.permissions.per_folder = !draft.permissions.per_folder;
        } else if let Some(folder) = folders.get(cursor - 1) {
            // Standard → read only → no access → standard. A folder can only
            // ever allow less than the account does.
            let rules = &mut draft.permissions.folder_rules;
            match rules.get(folder).copied() {
                None => {
                    rules.insert(folder.clone(), FolderRule { read: true, write: false });
                }
                Some(FolderRule { read: true, write: false }) => {
                    rules.insert(folder.clone(), FolderRule { read: false, write: false });
                }
                Some(_) => {
                    rules.remove(folder);
                }
            }
            draft.permissions.per_folder = true;
        } else {
            let special = &mut draft.special_mailboxes;
            let slot = match cursor - 1 - folders.len() {
                0 => &mut special.drafts,
                1 => &mut special.sent,
                2 => &mut special.archive,
                3 => &mut special.junk,
                _ => &mut special.trash,
            };
            // Automatic first, then every folder the server has.
            let position = folders.iter().position(|folder| folder == slot).map_or(0, |index| index + 1);
            let count = folders.len() as isize + 1;
            let next = (position as isize + step).rem_euclid(count) as usize;
            *slot = if next == 0 { String::new() } else { folders[next - 1].clone() };
            special.manual = [&special.drafts, &special.sent, &special.archive, &special.junk, &special.trash]
                .iter()
                .any(|choice| !choice.is_empty());
        }
    }

    fn step_cache_level(&mut self, delta: isize) {
        let Some(draft) = &mut self.draft else { return };
        let levels = [CacheLevel::Off, CacheLevel::Headers, CacheLevel::Bodies, CacheLevel::Attachments];
        let position = levels.iter().position(|level| *level == draft.cache_level).unwrap_or(1) as isize;
        draft.cache_level = levels[(position + delta).clamp(0, 3) as usize];
    }

    /// The event loop did what was asked.
    pub fn succeeded(&mut self, text: &'static str) {
        self.message = Some(Message { text, detail: String::new(), is_error: false });
    }

    pub fn failed(&mut self, text: &'static str, detail: String) {
        self.message = Some(Message { text, detail, is_error: true });
    }

    fn leave_detail(&mut self) {
        self.focus = Focus::List;
        self.draft = None;
        self.access_draft = None;
        self.confirm_discard = false;
    }

    fn on_detail_key(&mut self, key: KeyEvent) {
        if self.confirm_discard {
            match key.code {
                KeyCode::Char('y' | 'j') => self.leave_detail(),
                KeyCode::Char('n') | KeyCode::Esc => self.confirm_discard = false,
                _ => {}
            }
            return;
        }
        self.message = None;
        let rows = self.rows();
        match key.code {
            KeyCode::Esc if self.is_dirty() => self.confirm_discard = true,
            KeyCode::Esc => self.leave_detail(),
            // On the connection tab letters are text, so it handles its own
            // movement; j and k must not steer there.
            code if self.section == Section::Accounts && self.account_tab == 0 => self.on_connection_key(code),
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(rows - 1),
            KeyCode::Char(' ') | KeyCode::Enter if self.section == Section::Clients => self.toggle_access_row(),
            code if self.section == Section::Accounts && self.account_tab == 2 => self.on_folders_key(code),
            KeyCode::Left if self.section == Section::Accounts && self.account_tab == 3 => self.step_cache_level(-1),
            KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter if self.section == Section::Accounts && self.account_tab == 3 => {
                self.step_cache_level(1);
            }
            KeyCode::Left if self.cursor == ROW_READ_DEPTH => self.step_read_depth(-1),
            KeyCode::Right if self.cursor == ROW_READ_DEPTH => self.step_read_depth(1),
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_row(),
            _ => {}
        }
    }

    /// Row 0 is "all accounts", row 1 "selected accounts", the rest are the
    /// accounts. Ticking an account is itself the choice of "selected".
    fn toggle_access_row(&mut self) {
        let Some(draft) = &mut self.access_draft else { return };
        match self.cursor {
            0 => *draft = ClientAccountAccess::All,
            1 => {
                if *draft == ClientAccountAccess::All {
                    *draft = ClientAccountAccess::Selected(Default::default());
                }
            }
            row => {
                let Some(view) = self.snapshot.accounts.get(row - 2) else { return };
                // Under "all" every box shows ticked, so the first toggle
                // unticks one: the selection starts as everything there is.
                let mut ids = match draft {
                    ClientAccountAccess::Selected(ids) => ids.clone(),
                    ClientAccountAccess::All => {
                        self.snapshot.accounts.iter().map(|view| view.account.id.clone()).collect()
                    }
                };
                if !ids.remove(&view.account.id) {
                    ids.insert(view.account.id.clone());
                }
                *draft = ClientAccountAccess::Selected(ids);
            }
        }
    }

    fn step_read_depth(&mut self, delta: i64) {
        let Some(draft) = &mut self.draft else { return };
        let rank = (draft.permissions.read.rank() as i64 + delta).clamp(0, 3) as u64;
        if let Some(read) = ReadAccess::from_rank(rank) {
            draft.permissions.read = read;
        }
    }

    fn toggle_row(&mut self) {
        let cursor = self.cursor;
        let Some(draft) = &mut self.draft else { return };
        let permissions = &mut draft.permissions;
        match cursor {
            0..=3 => {
                let preset = PermissionPreset::ALL[cursor];
                permissions.read = preset.read();
                permissions.write = preset.write();
                permissions.send = preset.send();
            }
            ROW_READ_DEPTH => {
                let next = (permissions.read.rank() + 1) % 4;
                permissions.read = ReadAccess::from_rank(next).unwrap_or(permissions.read);
            }
            5 => permissions.write.drafts = !permissions.write.drafts,
            6 => permissions.send = !permissions.send,
            7 => permissions.write.mark = !permissions.write.mark,
            8 => permissions.write.r#move = !permissions.write.r#move,
            9 => {
                permissions.write.trash = !permissions.write.trash;
                // Permanent delete cannot outlive the right it escalates.
                if !permissions.write.trash {
                    permissions.write.permanent_delete = false;
                }
            }
            _ if permissions.write.trash => {
                permissions.write.permanent_delete = !permissions.write.permanent_delete;
            }
            _ => {
                self.message = Some(Message {
                    text: "Permanent delete needs the Delete right first.",
                    detail: String::new(),
                    is_error: true,
                });
            }
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
        if let Some(wizard) = &mut self.wizard {
            if input::is_command(&key) && key.code == KeyCode::Char('c') {
                self.should_quit = true;
                return;
            }
            match wizard.on_key(key) {
                Outcome::Stay => {}
                Outcome::Close => self.wizard = None,
                Outcome::Enroll(account, password) => self.request = Some(Request::Enroll(account, password)),
                Outcome::Discover(email) => self.request = Some(Request::Discover(email)),
            }
            return;
        }
        if input::is_command(&key) {
            match key.code {
                // Quitting over unsaved changes asks first, like leaving does.
                KeyCode::Char('c' | 'q') if self.is_dirty() && !self.confirm_discard => self.confirm_discard = true,
                KeyCode::Char('c' | 'q') => self.should_quit = true,
                KeyCode::Char('s') if self.is_dirty() => {
                    self.request = match (&self.access_draft, self.snapshot.clients.get(self.client_index)) {
                        (Some(access), Some(client)) => {
                            Some(Request::SetAccess(client.descriptor.id.to_owned(), access.clone()))
                        }
                        _ => match self.account_to_save() {
                            Ok(account) => {
                                let password = Some(self.extras.password.clone()).filter(|password| !password.0.is_empty());
                                Some(Request::SaveAccount(Box::new(account), password))
                            }
                            Err(problem) => {
                                self.message = Some(Message { text: problem, detail: String::new(), is_error: true });
                                None
                            }
                        },
                    };
                }
                _ => {}
            }
            return;
        }
        if self.focus == Focus::Detail {
            self.on_detail_key(key);
            return;
        }
        if self.confirm_remove {
            self.confirm_remove = false;
            if let (KeyCode::Char('y' | 'j'), Some(view)) = (key.code, self.snapshot.accounts.get(self.account_index)) {
                self.request = Some(Request::RemoveAccount(view.account.id.clone()));
            }
            return;
        }
        if self.confirm_disconnect {
            self.confirm_disconnect = false;
            if let (KeyCode::Char('y' | 'j'), Some(client)) = (key.code, self.snapshot.clients.get(self.client_index)) {
                self.request = Some(Request::Disconnect(client.descriptor.id.to_owned()));
            }
            return;
        }
        self.message = None;
        if self.section == Section::Settings && self.on_settings_key(key.code) {
            return;
        }
        if self.section == Section::Clients && self.on_client_key(key.code) {
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('r') if self.section != Section::Log => self.wants_reload = true,
            KeyCode::Char(digit @ '1'..='7') => {
                self.section = Section::ALL[digit as usize - '1' as usize];
                self.revealed = None;
                self.detail_scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp if self.section != Section::Log => self.detail_scroll = self.detail_scroll.saturating_sub(8),
            KeyCode::PageDown if self.section != Section::Log => self.detail_scroll = (self.detail_scroll + 8).min(200),
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
            KeyCode::Char('u') if self.section == Section::Updates => {
                self.message = Some(Message { text: "Checking for updates…", detail: String::new(), is_error: false });
                self.request = Some(Request::CheckForUpdates);
            }
            KeyCode::Char('e') if self.section == Section::Log && !self.snapshot.audit.is_empty() => {
                self.request = Some(Request::ExportLog);
            }
            KeyCode::Char('R') if self.section == Section::Accounts && self.account_tab == 3 && self.rebuild.is_none() => {
                if let Some(view) = self.snapshot.accounts.get(self.account_index) {
                    self.request = Some(Request::RebuildCache(view.account.id.clone()));
                }
            }
            KeyCode::Char('t') if self.section == Section::Accounts => {
                if let Some(view) = self.snapshot.accounts.get(self.account_index) {
                    self.message = Some(Message { text: "Checking the connection…", detail: String::new(), is_error: false });
                    self.request = Some(Request::TestConnection(view.account.id.clone()));
                }
            }
            KeyCode::Char('D') if self.section == Section::Accounts && !self.snapshot.accounts.is_empty() => {
                self.confirm_remove = true;
            }
            KeyCode::Char('n') if matches!(self.section, Section::Accounts | Section::Overview) => {
                self.section = Section::Accounts;
                self.wizard = Some(Wizard::default());
            }
            // The folders tab edits against the folders the server really
            // has, so it logs in first; the others start at once.
            KeyCode::Enter if self.section == Section::Accounts && self.account_tab == 2 => {
                if let Some(view) = self.snapshot.accounts.get(self.account_index) {
                    self.message = Some(Message { text: "Loading folders…", detail: String::new(), is_error: false });
                    self.request = Some(Request::LoadMailboxes(view.account.id.clone()));
                }
            }
            KeyCode::Enter if self.section == Section::Accounts => self.begin_account_edit(Vec::new()),
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

    /// Language, background check, notifications, then the two jumps the
    /// macOS app offers here too. Returns whether it took the key.
    fn on_settings_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.settings_index = self.settings_index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.settings_index = (self.settings_index + 1).min(SETTINGS_ROWS - 1),
            KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Left | KeyCode::Right => match self.settings_index {
                0 => {
                    let mut settings = self.settings;
                    settings.language = settings.next_language();
                    self.request = Some(Request::SaveSettings(Box::new(settings)));
                }
                1 => self.request = Some(Request::SetAutocheck(!self.snapshot.autocheck)),
                2 => {
                    let mut settings = self.settings;
                    settings.notifications = !settings.notifications;
                    self.request = Some(Request::SaveSettings(Box::new(settings)));
                }
                3 => self.section = Section::Clients,
                _ => self.section = Section::Updates,
            },
            _ => return false,
        }
        true
    }

    /// The keys that act on the selected client. Returns whether it took one.
    fn on_client_key(&mut self, code: KeyCode) -> bool {
        let Some(client) = self.snapshot.clients.get(self.client_index) else { return false };
        let id = client.descriptor.id.to_owned();
        match code {
            KeyCode::Char('c') if client.can_connect() => self.request = Some(Request::Connect(id)),
            KeyCode::Char('T') if client.is_paired() => self.confirm_disconnect = true,
            KeyCode::Char('y') if client.is_paired() => self.request = Some(Request::CopySnippet(id)),
            KeyCode::Char('v') if client.is_paired() => {
                if self.revealed.as_ref().is_some_and(|(shown, _)| *shown == id) {
                    self.revealed = None;
                } else {
                    self.request = Some(Request::RevealKey(id));
                }
            }
            KeyCode::Enter if client.is_paired() => {
                self.access_draft = client.access.clone();
                self.focus = Focus::Detail;
                self.cursor = 0;
            }
            // A key shown in clear does not follow the selection around.
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k') => {
                self.revealed = None;
                return false;
            }
            _ => return false,
        }
        true
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
        // A different item: its detail starts at the top.
        self.detail_scroll = 0;
    }
}
