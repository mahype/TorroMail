//! What the user is looking at, and what a key does to it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use torromail_control::policy::ClientAccountAccess;
use torromail_control::{MailAccount, PermissionPreset, ReadAccess};

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
    SaveAccount(Box<MailAccount>),
    Connect(String),
    Disconnect(String),
    SetAccess(String, ClientAccountAccess),
    /// Put the snippet, real key included, on the clipboard.
    CopySnippet(String),
    /// Fetch the key so the snippet on screen can show it.
    RevealKey(String),
}

/// The editable rows of the permissions tab, top to bottom.
pub const PERMISSION_ROWS: usize = 11;
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
            confirm_discard: false,
            message: None,
            request: None,
            access_draft: None,
            confirm_disconnect: false,
            revealed: None,
        }
    }

    /// Whether a draft differs from what is stored.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        if let (Some(draft), Some(client)) = (&self.access_draft, self.snapshot.clients.get(self.client_index)) {
            return client.access.as_ref() != Some(draft);
        }
        match (&self.draft, self.snapshot.accounts.get(self.account_index)) {
            (Some(draft), Some(stored)) => *draft != stored.account,
            _ => false,
        }
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
        let rows = if self.section == Section::Clients { 2 + self.snapshot.accounts.len() } else { PERMISSION_ROWS };
        match key.code {
            KeyCode::Esc if self.is_dirty() => self.confirm_discard = true,
            KeyCode::Esc => self.leave_detail(),
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(rows - 1),
            KeyCode::Char(' ') | KeyCode::Enter if self.section == Section::Clients => self.toggle_access_row(),
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
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                // Quitting over unsaved changes asks first, like leaving does.
                KeyCode::Char('c' | 'q') if self.is_dirty() && !self.confirm_discard => self.confirm_discard = true,
                KeyCode::Char('c' | 'q') => self.should_quit = true,
                KeyCode::Char('s') if self.is_dirty() => {
                    self.request = match (&self.access_draft, self.snapshot.clients.get(self.client_index)) {
                        (Some(access), Some(client)) => {
                            Some(Request::SetAccess(client.descriptor.id.to_owned(), access.clone()))
                        }
                        _ => self.draft.clone().map(|draft| Request::SaveAccount(Box::new(draft))),
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
        if self.confirm_disconnect {
            self.confirm_disconnect = false;
            if let (KeyCode::Char('y' | 'j'), Some(client)) = (key.code, self.snapshot.clients.get(self.client_index)) {
                self.request = Some(Request::Disconnect(client.descriptor.id.to_owned()));
            }
            return;
        }
        self.message = None;
        if self.section == Section::Clients && self.on_client_key(key.code) {
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('r') if self.section != Section::Log => self.wants_reload = true,
            KeyCode::Char(digit @ '1'..='7') => {
                self.section = Section::ALL[digit as usize - '1' as usize];
                self.revealed = None;
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
            KeyCode::Enter if self.section == Section::Accounts && self.account_tab == 1 => {
                if let Some(view) = self.snapshot.accounts.get(self.account_index) {
                    self.draft = Some(view.account.clone());
                    self.focus = Focus::Detail;
                    self.cursor = 0;
                }
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
    }
}
