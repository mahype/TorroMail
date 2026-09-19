//! What a mail address implies about where to connect and how to get in.
//!
//! Everything here is either a fact about someone else's servers — kept in
//! one table rather than scattered across a wizard — or pure inference from a
//! record someone else fetched (an MX host, an SPF line, a Mozilla autoconfig
//! document). No network: the chain that asks the network belongs to the
//! platform, and hands what it found to the functions below.

use serde_json::{Value, json};

use crate::account::{ConnectionSecurity, OAuthIssuer};

/// How a mailbox lets us in. The provider decides this, not the user — which
/// is why a wizard asks for an address and never for a login method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthPath {
    /// The provider hands out tokens through its own web login.
    OAuth(OAuthIssuer),
    /// Plain IMAP: the password the user already has.
    Password,
    /// A generated app password; `setup_url` is where to create one.
    AppPassword { setup_url: String },
}

/// What one mailbox needs: where to connect and how to get in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredConfig {
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: ConnectionSecurity,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: ConnectionSecurity,
    pub auth: AuthPath,
    /// What to call the provider in the UI ("Google Workspace", "mailbox.org").
    pub provider_label: String,
    /// The provider as `state.json` stores it: `IMAP/SMTP`, `Gmail`,
    /// `Microsoft 365`.
    pub provider: &'static str,
    /// How this was found — for the log, not for the user.
    pub source: String,
}

pub const PROVIDER_IMAP_SMTP: &str = "IMAP/SMTP";
pub const PROVIDER_GMAIL: &str = "Gmail";
pub const PROVIDER_MICROSOFT: &str = "Microsoft 365";

const GMAIL_APP_PASSWORD_URL: &str = "https://myaccount.google.com/apppasswords";

impl DiscoveredConfig {
    /// A catalog entry on the standard ports; the encryption is what the
    /// ports imply, so an entry only states it where the provider is unusual.
    fn standard(imap_host: &str, smtp_host: &str, smtp_port: u16, auth: AuthPath, label: &str, provider: &'static str) -> Self {
        Self {
            imap_host: imap_host.to_owned(),
            imap_port: 993,
            imap_security: ConnectionSecurity::implied_by_imap_port(993),
            smtp_host: smtp_host.to_owned(),
            smtp_port,
            smtp_security: ConnectionSecurity::implied_by_smtp_port(smtp_port),
            auth,
            provider_label: label.to_owned(),
            provider,
            source: "catalog".to_owned(),
        }
    }

    fn relabelled(mut self, label: &str, source: &str) -> Self {
        self.provider_label = label.to_owned();
        self.source = source.to_owned();
        self
    }

    /// The shape the shared cases and any other surface read.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "imap_host": self.imap_host,
            "imap_port": self.imap_port,
            "imap_security": self.imap_security.as_str(),
            "smtp_host": self.smtp_host,
            "smtp_port": self.smtp_port,
            "smtp_security": self.smtp_security.as_str(),
            "auth": match &self.auth {
                AuthPath::OAuth(issuer) => json!({ "kind": "oauth", "issuer": issuer.as_str() }),
                AuthPath::Password => json!({ "kind": "password" }),
                AuthPath::AppPassword { setup_url } => json!({ "kind": "app_password", "setup_url": setup_url }),
            },
            "provider_label": self.provider_label,
            "provider": self.provider,
            "source": self.source,
        })
    }
}

fn gmail() -> DiscoveredConfig {
    DiscoveredConfig::standard("imap.gmail.com", "smtp.gmail.com", 465, AuthPath::OAuth(OAuthIssuer::Google), "Gmail", PROVIDER_GMAIL)
}

fn microsoft() -> DiscoveredConfig {
    DiscoveredConfig::standard(
        "outlook.office365.com",
        "smtp.office365.com",
        587,
        AuthPath::OAuth(OAuthIssuer::Microsoft),
        "Microsoft",
        PROVIDER_MICROSOFT,
    )
}

/// Domain → settings, for providers common enough that a network round trip
/// would only slow the wizard down.
#[must_use]
pub fn lookup(domain: &str) -> Option<DiscoveredConfig> {
    let app_password = |url: &str| AuthPath::AppPassword { setup_url: url.to_owned() };
    let plain = |imap: &str, smtp: &str, auth: AuthPath, label: &str| {
        DiscoveredConfig::standard(imap, smtp, 587, auth, label, PROVIDER_IMAP_SMTP)
    };
    Some(match domain.to_lowercase().as_str() {
        "gmail.com" | "googlemail.com" => gmail(),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" | "outlook.de" | "hotmail.de" => microsoft(),
        "icloud.com" | "me.com" | "mac.com" => plain(
            "imap.mail.me.com",
            "smtp.mail.me.com",
            app_password("https://account.apple.com/account/manage"),
            "iCloud",
        ),
        "gmx.de" | "gmx.net" | "gmx.at" | "gmx.ch" => plain("imap.gmx.net", "mail.gmx.net", AuthPath::Password, "GMX"),
        "web.de" => plain("imap.web.de", "smtp.web.de", AuthPath::Password, "WEB.DE"),
        "mailbox.org" => plain("imap.mailbox.org", "smtp.mailbox.org", AuthPath::Password, "mailbox.org"),
        "posteo.de" => plain("posteo.de", "posteo.de", AuthPath::Password, "Posteo"),
        "fastmail.com" | "fastmail.fm" => plain(
            "imap.fastmail.com",
            "smtp.fastmail.com",
            app_password("https://app.fastmail.com/settings/security/apps"),
            "Fastmail",
        ),
        "t-online.de" | "magenta.de" => {
            plain("secureimap.t-online.de", "securesmtp.t-online.de", AuthPath::Password, "Telekom")
        }
        "yahoo.com" | "yahoo.de" | "ymail.com" => plain(
            "imap.mail.yahoo.com",
            "smtp.mail.yahoo.com",
            app_password("https://login.yahoo.com/account/security"),
            "Yahoo",
        ),
        _ => return None,
    })
}

/// MX hostname → the provider hosting it. This is what makes a company domain
/// work: `firma.de` with MX at Google is a Workspace mailbox and must be
/// offered OAuth, never a password field.
#[must_use]
pub fn from_mx_host(mx_host: &str) -> Option<DiscoveredConfig> {
    let host = mx_host.to_lowercase();
    if host.ends_with("google.com") || host.ends_with("googlemail.com") {
        return Some(gmail().relabelled("Google Workspace", "mx"));
    }
    if host.ends_with("outlook.com") || host.ends_with("office365.com") {
        return Some(microsoft().relabelled("Microsoft 365", "mx"));
    }
    None
}

/// SPF → the provider hosting the domain. A weaker claim than the MX, but the
/// only one left when inbound mail runs through a filtering gateway. Only the
/// two hyperscalers are read out of it: anything else on the line is
/// somebody's newsletter sender, which says nothing about where the mailbox is.
#[must_use]
pub fn from_spf(record: &str) -> Option<DiscoveredConfig> {
    let line = record.to_lowercase();
    if !line.starts_with("v=spf1") {
        return None;
    }
    if line.contains("include:spf.protection.outlook.com") {
        return Some(microsoft().relabelled("Microsoft 365", "spf"));
    }
    if line.contains("include:_spf.google.com") {
        return Some(gmail().relabelled("Google Workspace", "spf"));
    }
    None
}

/// Google is the only provider where a personal account can still sidestep
/// OAuth, and only with 2FA on — a fallback, never the first path.
#[must_use]
pub fn app_password_fallback(config: &DiscoveredConfig) -> Option<AuthPath> {
    (config.provider == PROVIDER_GMAIL).then(|| AuthPath::AppPassword { setup_url: GMAIL_APP_PASSWORD_URL.to_owned() })
}

/// The domain half of an address, or `None` when it has none worth asking
/// about.
#[must_use]
pub fn domain_of(email: &str) -> Option<String> {
    let parts: Vec<&str> = email.split('@').filter(|part| !part.is_empty()).collect();
    let [_, domain] = parts.as_slice() else {
        return None;
    };
    let domain = domain.trim_matches([' ', '\t']).to_lowercase();
    domain.contains('.').then_some(domain)
}

/// Only the registrable-ish tail: `aspmx.l.google.com` → `google.com`. Crude
/// on purpose — it feeds a table lookup that either hits or does not.
#[must_use]
pub fn base_domain(host: &str) -> String {
    let host = host.to_lowercase();
    let parts: Vec<&str> = host.split('.').filter(|part| !part.is_empty()).collect();
    if parts.len() < 2 { host } else { parts[parts.len() - 2..].join(".") }
}

/// Where a domain publishes its own Mozilla autoconfig, in the order to ask.
#[must_use]
pub fn autoconfig_urls(domain: &str, email: &str) -> Vec<String> {
    let escaped: String = email
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => (byte as char).to_string(),
            b'-' | b'.' | b'_' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
            | b':' | b'@' | b'/' | b'?' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect();
    vec![
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={escaped}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={escaped}"),
    ]
}

/// Reads Mozilla's client-config XML — the format both a domain's own
/// autoconfig and the ISPDB speak. `None` when there is no IMAP server in it.
#[must_use]
pub fn parse_autoconfig(xml: &str, source: &str) -> Option<DiscoveredConfig> {
    let document = roxmltree::Document::parse(xml).ok()?;
    let elements = || document.descendants().filter(roxmltree::Node::is_element);
    let text_of = |server: roxmltree::Node<'_, '_>, name: &str| {
        server
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == name)
            .filter_map(|node| node.text())
            .map(|text| text.trim().to_owned())
            .last()
    };

    // POP is listed alongside IMAP; TorroMail only speaks IMAP.
    let incoming = elements().find(|node| {
        node.tag_name().name() == "incomingServer"
            && node.attribute("type").is_some_and(|kind| kind.eq_ignore_ascii_case("imap"))
    })?;
    let imap_host = text_of(incoming, "hostname").filter(|host| !host.is_empty())?;
    let outgoing = elements().find(|node| node.tag_name().name() == "outgoingServer");

    // Several mechanisms are listed in preference order; OAuth2 anywhere in
    // the list means the server offers it. But a token is worthless without a
    // registered client, so the hint only counts for issuers we can talk to —
    // anything else degrades to a password field, which at least has a chance.
    let offers_oauth = incoming
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "authentication")
        .filter_map(|node| node.text())
        .any(|mechanism| mechanism.to_lowercase().contains("oauth2"));
    let issuer = issuer_for_host(&imap_host);
    let auth = match issuer {
        Some(issuer) if offers_oauth => AuthPath::OAuth(issuer),
        _ => AuthPath::Password,
    };

    let port = |server: roxmltree::Node<'_, '_>| text_of(server, "port").and_then(|port| port.parse::<u16>().ok());
    let imap_port = port(incoming).unwrap_or(993);
    let smtp_port = outgoing.and_then(port).unwrap_or(587);
    let display_name = elements()
        .find(|node| node.tag_name().name() == "displayName")
        .and_then(|node| node.text())
        .map(|text| text.trim().to_owned());

    Some(DiscoveredConfig {
        imap_security: text_of(incoming, "socketType")
            .and_then(|kind| stated_security(&kind))
            .unwrap_or_else(|| ConnectionSecurity::implied_by_imap_port(imap_port)),
        imap_port,
        smtp_host: outgoing.and_then(|server| text_of(server, "hostname")).unwrap_or_default(),
        smtp_security: outgoing
            .and_then(|server| text_of(server, "socketType"))
            .and_then(|kind| stated_security(&kind))
            .unwrap_or_else(|| ConnectionSecurity::implied_by_smtp_port(smtp_port)),
        smtp_port,
        auth,
        provider_label: display_name.unwrap_or_else(|| imap_host.clone()),
        provider: match issuer {
            Some(OAuthIssuer::Google) => PROVIDER_GMAIL,
            Some(OAuthIssuer::Microsoft) => PROVIDER_MICROSOFT,
            None => PROVIDER_IMAP_SMTP,
        },
        source: source.to_owned(),
        imap_host,
    })
}

/// Mozilla's `socketType`, the one place the encryption is stated outright.
/// `plain` is deliberately not honoured — credentials never go in the clear —
/// and anything unrecognised falls back to what the port implies.
fn stated_security(socket_type: &str) -> Option<ConnectionSecurity> {
    match socket_type.to_uppercase().as_str() {
        "SSL" | "TLS" => Some(ConnectionSecurity::Tls),
        "STARTTLS" => Some(ConnectionSecurity::StartTls),
        _ => None,
    }
}

fn issuer_for_host(host: &str) -> Option<OAuthIssuer> {
    let host = host.to_lowercase();
    if host.ends_with("gmail.com") || host.ends_with("googlemail.com") {
        return Some(OAuthIssuer::Google);
    }
    if host.ends_with("office365.com") || host.ends_with("outlook.com") {
        return Some(OAuthIssuer::Microsoft);
    }
    None
}
