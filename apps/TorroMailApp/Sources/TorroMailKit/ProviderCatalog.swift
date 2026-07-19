import Foundation

/// How a mailbox lets us in. The provider decides this, not the user — which
/// is why the wizard asks for an address and never for a login method.
public enum AuthPath: Hashable, Sendable {
    /// The provider hands out tokens through its own web login.
    case oauth(OAuthIssuer)
    /// Plain IMAP: the password the user already has.
    case password
    /// Gmail without OAuth: a generated app password, 2FA required.
    case appPassword(setupURL: URL)
}

/// The two token issuers TorroMail speaks to. Both are public clients under
/// RFC 8252 — no client secret, PKCE instead.
public enum OAuthIssuer: String, Hashable, Sendable, Codable {
    case google
    case microsoft

    public var displayName: String {
        switch self {
        case .google: "Google"
        case .microsoft: "Microsoft"
        }
    }

    public var authorizationEndpoint: URL {
        switch self {
        case .google:
            URL(string: "https://accounts.google.com/o/oauth2/v2/auth")!
        case .microsoft:
            URL(string: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize")!
        }
    }

    public var tokenEndpoint: URL {
        switch self {
        case .google:
            URL(string: "https://oauth2.googleapis.com/token")!
        case .microsoft:
            URL(string: "https://login.microsoftonline.com/common/oauth2/v2.0/token")!
        }
    }

    /// `openid email profile` rides along everywhere: it is what lets the
    /// wizard fill in the sender name and the canonical address after login
    /// instead of trusting what was typed.
    public var scopes: [String] {
        switch self {
        case .google:
            // Full IMAP access is a Google "restricted scope" — it is what
            // forces app verification plus a CASA assessment before the app
            // can leave testing mode.
            ["https://mail.google.com/", "openid", "email", "profile"]
        case .microsoft:
            [
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "https://outlook.office.com/SMTP.Send",
                "offline_access",
                "openid",
                "email",
                "profile"
            ]
        }
    }

    /// Where to ask who the token belongs to. Lets the wizard correct the
    /// address and fill in the sender name from the provider instead of
    /// trusting what was typed into the first step.
    public var userInfoEndpoint: URL {
        switch self {
        case .google:
            URL(string: "https://openidconnect.googleapis.com/v1/userinfo")!
        case .microsoft:
            URL(string: "https://graph.microsoft.com/oidc/userinfo")!
        }
    }

    /// No client secret anywhere, by design: both clients are registered as
    /// public clients (RFC 8252) and PKCE is what protects the exchange. A
    /// secret shipped inside a desktop app would not be one.
    public var clientID: String {
        switch self {
        case .google:
            ProviderCatalog.override(for: "TorroMailGoogleClientID")
                ?? ProviderCatalog.placeholderGoogleClientID
        case .microsoft:
            ProviderCatalog.override(for: "TorroMailMicrosoftClientID")
                ?? ProviderCatalog.placeholderMicrosoftClientID
        }
    }

    /// The custom scheme the provider redirects back to. Google demands the
    /// reverse of the client ID; Microsoft accepts `msauth.<bundle-id>`.
    public var redirectScheme: String {
        switch self {
        case .google:
            clientID.split(separator: ".").reversed().joined(separator: ".")
        case .microsoft:
            "msauth.\(ProviderCatalog.bundleIdentifier)"
        }
    }

    public var redirectURI: String {
        switch self {
        case .google: "\(redirectScheme):/oauth2redirect"
        case .microsoft: "\(redirectScheme)://auth"
        }
    }

    /// Google keeps unverified apps in a testing pen: a scary interstitial and
    /// a hard cap of 100 registered testers. The wizard says this out loud
    /// before opening the browser, so the warning reads as expected rather
    /// than as a failure.
    public var warnsAboutUnverifiedApp: Bool {
        self == .google
    }
}

/// What one mailbox needs: where to connect and how to get in.
public struct DiscoveredConfig: Hashable, Sendable {
    public var imapHost: String
    public var imapPort: Int
    public var imapSecurity: ConnectionSecurity
    public var smtpHost: String
    public var smtpPort: Int
    public var smtpSecurity: ConnectionSecurity
    public var auth: AuthPath
    /// What to call the provider in the UI ("Google Workspace", "Mailbox.org").
    public var providerLabel: String
    public var provider: Provider
    /// How this was found — for the log, not for the user.
    public var source: String

    /// The encryption defaults to what the port implies, so a catalog entry
    /// only states it where the provider is unusual.
    public init(
        imapHost: String,
        imapPort: Int = 993,
        imapSecurity: ConnectionSecurity? = nil,
        smtpHost: String,
        smtpPort: Int = 587,
        smtpSecurity: ConnectionSecurity? = nil,
        auth: AuthPath,
        providerLabel: String,
        provider: Provider = .imapSmtp,
        source: String
    ) {
        self.imapHost = imapHost
        self.imapPort = imapPort
        self.imapSecurity = imapSecurity ?? .impliedByIMAPPort(imapPort)
        self.smtpHost = smtpHost
        self.smtpPort = smtpPort
        self.smtpSecurity = smtpSecurity ?? .impliedBySMTPPort(smtpPort)
        self.auth = auth
        self.providerLabel = providerLabel
        self.provider = provider
        self.source = source
    }
}

/// The providers worth knowing without asking the network. Everything here is
/// a fact about someone else's servers, so it belongs in one table rather than
/// scattered across the wizard.
public enum ProviderCatalog {
    /// Placeholders until the app registrations exist. A build that still
    /// carries these cannot do OAuth, and `isConfigured` is what the wizard
    /// checks before offering it.
    static let placeholderGoogleClientID = "UNCONFIGURED.apps.googleusercontent.com"
    static let placeholderMicrosoftClientID = "UNCONFIGURED-microsoft-client-id"

    public static var bundleIdentifier: String {
        Bundle.main.bundleIdentifier ?? "de.torromail.app"
    }

    /// Client IDs are not secrets — every open-source mail client ships them
    /// in the clear, and PKCE is what actually protects the exchange. They
    /// still come from Info.plist when present, so a fork can drop in its own
    /// registration without touching this file.
    static func override(for key: String) -> String? {
        guard let value = Bundle.main.object(forInfoDictionaryKey: key) as? String,
              !value.isEmpty else { return nil }
        return value
    }

    public static func isConfigured(_ issuer: OAuthIssuer) -> Bool {
        switch issuer {
        case .google: issuer.clientID != placeholderGoogleClientID
        case .microsoft: issuer.clientID != placeholderMicrosoftClientID
        }
    }

    static let gmailAppPasswordURL = URL(string: "https://myaccount.google.com/apppasswords")!

    /// Google's own domains: a personal Gmail can still fall back to an app
    /// password, so `appPasswordFallback` stays open here.
    static let gmail = DiscoveredConfig(
        imapHost: "imap.gmail.com",
        imapPort: 993,
        smtpHost: "smtp.gmail.com",
        smtpPort: 465,
        auth: .oauth(.google),
        providerLabel: "Gmail",
        provider: .gmail,
        source: "catalog"
    )

    static let microsoft = DiscoveredConfig(
        imapHost: "outlook.office365.com",
        imapPort: 993,
        smtpHost: "smtp.office365.com",
        smtpPort: 587,
        auth: .oauth(.microsoft),
        providerLabel: "Microsoft",
        provider: .microsoft,
        source: "catalog"
    )

    /// Domain → settings, for providers common enough that a network round
    /// trip would only slow the wizard down.
    static let byDomain: [String: DiscoveredConfig] = {
        var table: [String: DiscoveredConfig] = [:]

        for domain in ["gmail.com", "googlemail.com"] {
            table[domain] = gmail
        }
        for domain in ["outlook.com", "hotmail.com", "live.com", "msn.com", "outlook.de", "hotmail.de"] {
            table[domain] = microsoft
        }
        for domain in ["icloud.com", "me.com", "mac.com"] {
            table[domain] = DiscoveredConfig(
                imapHost: "imap.mail.me.com",
                smtpHost: "smtp.mail.me.com",
                auth: .appPassword(
                    setupURL: URL(string: "https://account.apple.com/account/manage")!
                ),
                providerLabel: "iCloud",
                source: "catalog"
            )
        }
        for domain in ["gmx.de", "gmx.net", "gmx.at", "gmx.ch"] {
            table[domain] = DiscoveredConfig(
                imapHost: "imap.gmx.net",
                smtpHost: "mail.gmx.net",
                auth: .password,
                providerLabel: "GMX",
                source: "catalog"
            )
        }
        table["web.de"] = DiscoveredConfig(
            imapHost: "imap.web.de",
            smtpHost: "smtp.web.de",
            auth: .password,
            providerLabel: "WEB.DE",
            source: "catalog"
        )
        table["mailbox.org"] = DiscoveredConfig(
            imapHost: "imap.mailbox.org",
            smtpHost: "smtp.mailbox.org",
            auth: .password,
            providerLabel: "mailbox.org",
            source: "catalog"
        )
        table["posteo.de"] = DiscoveredConfig(
            imapHost: "posteo.de",
            smtpHost: "posteo.de",
            auth: .password,
            providerLabel: "Posteo",
            source: "catalog"
        )
        for domain in ["fastmail.com", "fastmail.fm"] {
            table[domain] = DiscoveredConfig(
                imapHost: "imap.fastmail.com",
                smtpHost: "smtp.fastmail.com",
                auth: .appPassword(
                    setupURL: URL(string: "https://app.fastmail.com/settings/security/apps")!
                ),
                providerLabel: "Fastmail",
                source: "catalog"
            )
        }
        for domain in ["t-online.de", "magenta.de"] {
            table[domain] = DiscoveredConfig(
                imapHost: "secureimap.t-online.de",
                smtpHost: "securesmtp.t-online.de",
                auth: .password,
                providerLabel: "Telekom",
                source: "catalog"
            )
        }
        for domain in ["yahoo.com", "yahoo.de", "ymail.com"] {
            table[domain] = DiscoveredConfig(
                imapHost: "imap.mail.yahoo.com",
                smtpHost: "smtp.mail.yahoo.com",
                auth: .appPassword(
                    setupURL: URL(string: "https://login.yahoo.com/account/security")!
                ),
                providerLabel: "Yahoo",
                source: "catalog"
            )
        }
        return table
    }()

    public static func lookup(domain: String) -> DiscoveredConfig? {
        byDomain[domain.lowercased()]
    }

    /// MX hostname → the provider hosting it. This is what makes a company
    /// domain work: `firma.de` with MX at Google is a Workspace mailbox and
    /// must be offered OAuth, never a password field.
    static func fromMXHost(_ mxHost: String) -> DiscoveredConfig? {
        let host = mxHost.lowercased()
        if host.hasSuffix("google.com") || host.hasSuffix("googlemail.com") {
            var config = gmail
            config.providerLabel = "Google Workspace"
            config.source = "mx"
            return config
        }
        if host.hasSuffix("outlook.com") || host.hasSuffix("protection.outlook.com")
            || host.hasSuffix("office365.com") {
            var config = microsoft
            config.providerLabel = "Microsoft 365"
            config.source = "mx"
            return config
        }
        return nil
    }

    /// Google is the only provider where a personal account can still sidestep
    /// OAuth, and only with 2FA on. Workspace lost that option in May 2025 —
    /// which is why this is offered as a fallback, never as the first path.
    public static func appPasswordFallback(for config: DiscoveredConfig) -> AuthPath? {
        guard config.provider == .gmail else { return nil }
        return .appPassword(setupURL: gmailAppPasswordURL)
    }
}
