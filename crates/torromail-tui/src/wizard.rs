//! Adding an account, step by step: who you are, how to get in, what
//! assistants may do — and only then the check that decides whether the
//! account comes to exist. A failed check returns to the sign-in step with
//! everything typed still there.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use torromail_control::providers::{self, AuthPath, DiscoveredConfig};
use torromail_control::{ConnectionSecurity, MailAccount, PermissionPreset, enroll};

use crate::input;

/// Typed text that must never show up in a debug print or a panic message.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Secret(••••)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Identity,
    SignIn,
    Rights,
    Done,
}

impl Step {
    pub const ALL: [Self; 4] = [Self::Identity, Self::SignIn, Self::Rights, Self::Done];

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Identity => "Email",
            Self::SignIn => "Login",
            Self::Rights => "Permissions",
            Self::Done => "Done",
        }
    }
}

/// What the address told us about how this mailbox lets people in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    None,
    /// The provider wants an app password rather than the normal one.
    AppPassword,
    /// The provider would use its own web login, which this surface cannot
    /// do yet; an app password is the way in for now.
    OAuthNotYet,
    /// Nothing is known about this domain; the servers have to be typed.
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wizard {
    pub step: Step,
    pub email: String,
    pub name: String,
    pub imap_host: String,
    pub imap_port: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub password: Secret,
    /// Which field has the cursor within the current step.
    pub field: usize,
    /// Where the caret sits in that field, in characters from its end.
    pub caret_back: usize,
    /// Whether the server fields are open for typing.
    pub manual: bool,
    pub provider_label: String,
    provider: &'static str,
    pub setup_url: Option<String>,
    pub hint: Hint,
    pub preset: usize,
    pub error: Option<String>,
    /// Set while the check runs, so the screen can say so.
    pub checking: bool,
    /// Set while the network is being asked about the domain.
    pub looking_up: bool,
    /// The account that came to exist.
    pub created: Option<String>,
}

/// What the wizard wants from the event loop.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Stay,
    Close,
    Enroll(Box<MailAccount>, Secret),
    /// Ask the network what this address implies.
    Discover(String),
}

impl Default for Wizard {
    fn default() -> Self {
        Self {
            step: Step::Identity,
            email: String::new(),
            name: String::new(),
            imap_host: String::new(),
            imap_port: "993".to_owned(),
            smtp_host: String::new(),
            smtp_port: "587".to_owned(),
            password: Secret::default(),
            field: 0,
            caret_back: 0,
            manual: false,
            provider_label: String::new(),
            provider: providers::PROVIDER_IMAP_SMTP,
            setup_url: None,
            hint: Hint::None,
            // Read + drafts: the everyday default.
            preset: 1,
            error: None,
            checking: false,
            looking_up: false,
            created: None,
        }
    }
}

impl Wizard {
    fn field_count(&self) -> usize {
        match self.step {
            Step::Identity => 2,
            Step::SignIn if self.manual => 5,
            Step::SignIn => 1,
            Step::Rights => PermissionPreset::ALL.len(),
            Step::Done => 0,
        }
    }

    /// The text field under the cursor, if the cursor is on one.
    fn text(&mut self) -> Option<&mut String> {
        match (self.step, self.manual, self.field) {
            (Step::Identity, _, 0) => Some(&mut self.email),
            (Step::Identity, _, _) => Some(&mut self.name),
            (Step::SignIn, false, _) | (Step::SignIn, true, 4) => Some(&mut self.password.0),
            (Step::SignIn, true, 0) => Some(&mut self.imap_host),
            (Step::SignIn, true, 1) => Some(&mut self.imap_port),
            (Step::SignIn, true, 2) => Some(&mut self.smtp_host),
            (Step::SignIn, true, _) => Some(&mut self.smtp_port),
            _ => None,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        let place = (self.step, self.manual, self.field);
        let outcome = self.handle(key);
        // Another field takes the caret to its end.
        if (self.step, self.manual, self.field) != place {
            self.caret_back = 0;
        }
        outcome
    }

    fn handle(&mut self, key: KeyEvent) -> Outcome {
        if input::is_command(&key) {
            if key.code == KeyCode::Char('d') && self.step == Step::SignIn {
                self.manual = !self.manual;
                // The cursor lands on the first server field, or back on the
                // password.
                self.field = 0;
            }
            return Outcome::Stay;
        }
        match key.code {
            KeyCode::Esc => return self.back(),
            KeyCode::Enter => return self.forward(),
            KeyCode::Tab | KeyCode::Down => self.field = (self.field + 1) % self.field_count().max(1),
            KeyCode::BackTab | KeyCode::Up => {
                let count = self.field_count().max(1);
                self.field = (self.field + count - 1) % count;
            }
            KeyCode::Char(' ') if self.step == Step::Rights => self.preset = self.field,
            code => {
                let mut caret_back = self.caret_back;
                if let Some(text) = self.text()
                    && input::edit(text, &mut caret_back, code)
                    && matches!(code, KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete)
                {
                    self.error = None;
                }
                self.caret_back = caret_back;
            }
        }
        if self.step == Step::Rights {
            self.preset = self.field;
        }
        Outcome::Stay
    }

    fn back(&mut self) -> Outcome {
        self.error = None;
        match self.step {
            Step::Identity | Step::Done => return Outcome::Close,
            Step::SignIn => self.step = Step::Identity,
            Step::Rights => self.step = Step::SignIn,
        }
        self.field = 0;
        Outcome::Stay
    }

    fn forward(&mut self) -> Outcome {
        self.error = None;
        match self.step {
            Step::Identity => {
                let Some(domain) = providers::domain_of(&self.email) else {
                    self.error = Some("TorroMail needs an address to work with.".to_owned());
                    self.field = 0;
                    return Outcome::Stay;
                };
                self.email = self.email.trim().to_owned();
                if self.name.trim().is_empty() {
                    self.name = self.email.clone();
                }
                // The table answers at once; anything else is worth a few
                // seconds of asking the network before asking the user.
                match providers::lookup(&domain) {
                    Some(known) => self.discovered(Some(known)),
                    None => {
                        self.looking_up = true;
                        return Outcome::Discover(self.email.clone());
                    }
                }
            }
            Step::SignIn => {
                let ports_ok = self.imap_port.parse::<u16>().is_ok() && self.smtp_port.parse::<u16>().is_ok();
                if self.imap_host.trim().is_empty() || !ports_ok {
                    self.manual = true;
                    self.field = 0;
                    self.error = Some("Enter the server by hand.".to_owned());
                } else if self.password.0.is_empty() {
                    self.field = if self.manual { 4 } else { 0 };
                } else {
                    self.step = Step::Rights;
                    self.field = self.preset;
                }
            }
            Step::Rights => return self.enroll_request(),
            Step::Done => return Outcome::Close,
        }
        Outcome::Stay
    }

    /// What the table or the network found, or that nothing was: either way
    /// the sign-in step is next.
    pub fn discovered(&mut self, found: Option<DiscoveredConfig>) {
        self.looking_up = false;
        self.step = Step::SignIn;
        self.field = 0;
        self.caret_back = 0;
        let Some(config) = found else {
            self.hint = Hint::Unknown;
            self.manual = true;
            self.provider_label.clear();
            return;
        };
        self.imap_host = config.imap_host.clone();
        self.imap_port = config.imap_port.to_string();
        self.smtp_host = config.smtp_host.clone();
        self.smtp_port = config.smtp_port.to_string();
        self.provider_label = config.provider_label.clone();
        self.provider = config.provider;
        self.manual = false;
        (self.hint, self.setup_url) = match &config.auth {
            AuthPath::Password => (Hint::None, None),
            AuthPath::AppPassword { setup_url } => (Hint::AppPassword, Some(setup_url.clone())),
            AuthPath::OAuth(_) => match providers::app_password_fallback(&config) {
                Some(AuthPath::AppPassword { setup_url }) => (Hint::OAuthNotYet, Some(setup_url)),
                _ => (Hint::OAuthNotYet, None),
            },
        };
    }

    fn enroll_request(&mut self) -> Outcome {
        let (Ok(imap_port), Ok(smtp_port)) = (self.imap_port.parse::<u16>(), self.smtp_port.parse::<u16>()) else {
            self.step = Step::SignIn;
            return Outcome::Stay;
        };
        let id = match enroll::new_account_id() {
            Ok(id) => id,
            Err(error) => {
                self.error = Some(error);
                return Outcome::Stay;
            }
        };
        let config = DiscoveredConfig {
            imap_host: self.imap_host.trim().to_owned(),
            imap_port,
            imap_security: ConnectionSecurity::implied_by_imap_port(imap_port),
            smtp_host: self.smtp_host.trim().to_owned(),
            smtp_port,
            smtp_security: ConnectionSecurity::implied_by_smtp_port(smtp_port),
            auth: AuthPath::Password,
            provider_label: self.provider_label.clone(),
            provider: self.provider,
            source: "wizard".to_owned(),
        };
        let mut account = enroll::password_account(id, self.name.trim(), &self.email, &config);
        let preset = PermissionPreset::ALL[self.preset.min(PermissionPreset::ALL.len() - 1)];
        account.permissions.read = preset.read();
        account.permissions.write = preset.write();
        account.permissions.send = preset.send();
        self.checking = true;
        Outcome::Enroll(Box::new(account), self.password.clone())
    }

    /// The check refused: back to the sign-in step, everything typed intact.
    pub fn check_failed(&mut self, reason: String) {
        self.checking = false;
        self.step = Step::SignIn;
        self.field = if self.manual { 4 } else { 0 };
        self.error = Some(reason);
    }

    pub fn check_passed(&mut self, account_name: String) {
        self.checking = false;
        self.password = Secret::default();
        self.created = Some(account_name);
        self.step = Step::Done;
    }
}
