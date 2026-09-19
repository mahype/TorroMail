import AppKit
import SwiftUI
import TorroMailKit

/// Setting up a mailbox, start to finish, in one window.
///
/// The old sheet asked for four facts and left the user in the account
/// details to do the actual work. This asks for an address, works out the rest
/// itself, and does not finish until the account is proven and its rights are
/// chosen. Nothing half-configured reaches the account list: the trial runs
/// against a throwaway policy document, and cancelling leaves nothing behind.
struct AccountSetupWizard: View {
    @Environment(\.textScale) private var textScale
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: TorroMailModel

    /// Only three of these are stops the user makes decisions at — the other
    /// two are the app doing work and saying so.
    ///
    /// The rights come before the check, not after it: everything the user
    /// decides is decided first, and the one step that can fail is the last
    /// one. The terminal surface walks the same order.
    enum Step {
        case identity
        case discovering
        case login
        case permissions
        case testing
    }

    @State private var step: Step = .identity
    @State private var email = ""
    @State private var name = NSFullUserName()
    @State private var discovered: DiscoveredConfig?
    @State private var password = ""
    @State private var permissions = PermissionSet()
    @State private var failure: String?
    @State private var showManualDetails = false
    /// Set when the user leaves the OAuth step for the server fields. The
    /// escape hatch for a tenant that only filters inbound mail through
    /// Microsoft while the mailboxes sit somewhere else — the MX says
    /// Microsoft either way, so detection cannot tell the two apart and the
    /// user has to be able to.
    @State private var overrodeOAuth = false

    /// The candidate's identity, fixed up front so the keychain entry and the
    /// trial document agree on who is being set up.
    @State private var accountID = UUID().uuidString

    // Manual overrides. Seeded from discovery, editable behind "Details" —
    // and the only thing on screen when discovery came up empty.
    @State private var imapHost = ""
    @State private var imapPort = 993
    @State private var imapSecurity: ConnectionSecurity = .tls
    @State private var smtpHost = ""
    @State private var smtpPort = 587
    @State private var smtpSecurity: ConnectionSecurity = .startTLS
    @State private var username = ""

    var body: some View {
        VStack(spacing: 0) {
            header

            Divider()

            Group {
                switch step {
                case .identity: identityStep
                case .discovering: waitingStep(L("Looking up the settings for %@…"))
                case .login: loginStep
                case .permissions: permissionsStep
                case .testing: waitingStep(L("Checking the connection…"))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            Divider()

            footer
        }
        .frame(width: 520 * max(1, textScale), height: 420 * max(1, textScale))
        .onDisappear(perform: discardLeftovers)
    }

    // MARK: - Chrome

    private var header: some View {
        HStack(spacing: 10 * textScale) {
            Image(systemName: "envelope.badge.shield.half.filled")
                .scaledFont(.title2)
                .foregroundStyle(Color.torroRed)
            VStack(alignment: .leading, spacing: 1 * textScale) {
                Text(L("Add Account"))
                    .scaledFont(.headline)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .scaledFont(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
        }
        .scaledPadding(.horizontal, 20)
        .scaledPadding(.vertical, 14)
    }

    private var subtitle: String {
        switch step {
        case .identity: L("TorroMail needs an address to work with.")
        case .discovering, .testing: ""
        case .login: discovered.map { "\($0.providerLabel) · \($0.imapHost)" } ?? L("Enter the server by hand.")
        case .permissions: L("The one thing left to decide.")
        }
    }

    private var footer: some View {
        HStack {
            if let failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .textSelection(.enabled)
            }
            Spacer()
            Button(L("Cancel"), role: .cancel) { dismiss() }
                .torroButton()

            switch step {
            case .identity:
                Button(L("Continue")) { beginDiscovery() }
                    .buttonStyle(.borderedProminent)
                    .disabled(Autodiscovery.domain(of: email) == nil || name.isEmpty)
            case .login:
                // OAuth signs in here, in the browser, because that is where
                // the user expects the provider's page; a password is only
                // noted and proven at the end with everything else.
                if case .oauth = effectiveAuth {
                    Button(L("Sign In")) { leaveLogin() }
                        .buttonStyle(.borderedProminent)
                        .disabled(!canAttemptLogin)
                } else {
                    Button(L("Continue")) { leaveLogin() }
                        .buttonStyle(.borderedProminent)
                        .disabled(!canAttemptLogin)
                }
            case .permissions:
                Button(L("Back")) {
                    failure = nil
                    step = .login
                }
                .torroButton()
                Button(L("Add")) { beginTrial() }
                    .buttonStyle(.borderedProminent)
            case .discovering, .testing:
                EmptyView()
            }
        }
        .scaledPadding(.horizontal, 20)
        .scaledPadding(.vertical, 12)
    }

    // MARK: - Steps

    private var identityStep: some View {
        Form {
            Section {
                TextField(L("Email"), text: $email, prompt: Text(verbatim: "name@example.com"))
                    .textContentType(.emailAddress)
                TextField(L("Sender Name"), text: $name, prompt: Text(verbatim: "Sven Wagener"))
            } footer: {
                Text(L("The name and address anything you approve is sent as."))
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    private func waitingStep(_ format: String) -> some View {
        VStack(spacing: 14 * textScale) {
            ProgressView()
                .controlSize(.large)
            Text(String(format: format, Autodiscovery.domain(of: email) ?? email))
                .scaledFont(.callout)
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var loginStep: some View {
        switch effectiveAuth {
        case .oauth(let issuer):
            oauthStep(issuer)
        case .appPassword(let setupURL):
            appPasswordStep(setupURL)
        case .password, .none:
            passwordStep
        }
    }

    /// The path the flow actually takes, which is not always the one discovery
    /// named. An account at Google or Microsoft is meant to use OAuth — but if
    /// this build has no client registered, OAuth cannot start, and a dead end
    /// is worse than a password field. So it falls back rather than trapping
    /// the user: to an app password where the provider issues them, and to the
    /// plain server fields otherwise. Once a client is hinterlegt,
    /// `isConfigured` is true and OAuth returns as the primary path with no
    /// further change here.
    private var effectiveAuth: AuthPath? {
        guard let config = discovered else { return discovered?.auth }
        guard case .oauth(let issuer) = config.auth else { return config.auth }
        if overrodeOAuth { return .password }
        if ProviderCatalog.isConfigured(issuer) { return config.auth }
        // Microsoft issues no app passwords for a tenant with modern auth, so
        // there is nothing to fall back *to* except the server fields — which
        // discovery has already filled in.
        return ProviderCatalog.appPasswordFallback(for: config) ?? .password
    }

    /// True when this step is standing in for OAuth — so it can say why it is
    /// asking for a password, instead of looking like the provider simply
    /// never supported anything better.
    private var isOAuthFallback: Bool {
        guard !overrodeOAuth, case .oauth = discovered?.auth else { return false }
        switch effectiveAuth {
        case .appPassword, .password: return true
        case .oauth, .none: return false
        }
    }

    /// One button. The provider's own page does the rest, which is the whole
    /// point — TorroMail never sees the password.
    private func oauthStep(_ issuer: OAuthIssuer) -> some View {
        VStack(spacing: 16 * textScale) {
            Spacer()
            Image(systemName: "lock.shield")
                .scaledSymbolFont(size: 34)
                .foregroundStyle(Color.torroRed)
            Text(String(format: L("%@ handles the login."), issuer.displayName))
                .scaledFont(.headline)
            Text(String(format: L("You sign in on %@’s own page. TorroMail never sees your password."), issuer.displayName))
                .scaledFont(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .scaledPadding(.horizontal, 40)

            // Google keeps unverified apps behind a warning screen. Saying so
            // first turns a scare into an expected step; saying nothing makes
            // a working setup look broken.
            if issuer.warnsAboutUnverifiedApp {
                Label(
                    L("Google will warn that TorroMail is not verified — that is expected while it is in testing. Choose “Advanced” and continue."),
                    systemImage: "info.circle"
                )
                .scaledFont(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.leading)
                .scaledPadding(.horizontal, 40)
            }

            if !ProviderCatalog.isConfigured(issuer) {
                Label(
                    String(format: L("This build has no %@ client registered, so the OAuth login is unavailable."), issuer.displayName),
                    systemImage: "exclamationmark.triangle"
                )
                .scaledFont(.caption)
                .foregroundStyle(.secondary)
            }

            // Detection reads the domain's DNS, and a company can point its
            // MX at Microsoft purely for spam filtering while the mailboxes
            // stay on its own server. No lookup can tell that apart — only
            // the person setting the account up can, so the way out is here
            // rather than in a support thread.
            Button(L("Enter the server details instead")) {
                overrodeOAuth = true
                showManualDetails = true
            }
            .torroButton()
            Spacer()
        }
    }

    private func appPasswordStep(_ setupURL: URL) -> some View {
        VStack(alignment: .leading, spacing: 12 * textScale) {
            Text(String(format: L("%@ needs an app password."), discovered?.providerLabel ?? ""))
                .scaledFont(.headline)
            Text(L("Your normal password will not work for mail apps. Create an app password and paste it here — it is a password just for TorroMail, and you can revoke it any time."))
                .scaledFont(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            // Why an app password for a provider that has OAuth: this build
            // carries no client yet. Said plainly so the step reads as a
            // working alternative, not a downgrade.
            if isOAuthFallback {
                Label(
                    L("The one-click sign-in is not available in this version yet. An app password works just as well and needs two-factor to be on."),
                    systemImage: "info.circle"
                )
                .scaledFont(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }

            Button(L("Create an app password…")) {
                NSWorkspace.shared.open(setupURL)
            }
            .torroButton()

            SecureField(L("App password"), text: $password)
                .textFieldStyle(.roundedBorder)

            manualDetails
            Spacer()
        }
        .scaledPadding(20)
    }

    private var passwordStep: some View {
        VStack(alignment: .leading, spacing: 12 * textScale) {
            if discovered == nil {
                Label(
                    L("Nothing could be found for this domain — the server details have to come from you."),
                    systemImage: "questionmark.circle"
                )
                .scaledFont(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }

            // Why a password for a provider whose whole point is that it does
            // not want one: this build carries no client yet. Said plainly, so
            // the step reads as a stopgap rather than as the intended way in.
            if isOAuthFallback {
                Label(
                    String(
                        format: L("The one-click sign-in with %@ is not available in this version yet. Use the password for this mailbox — with %@ that only works if your administrator still allows it."),
                        discovered?.providerLabel ?? "",
                        discovered?.providerLabel ?? ""
                    ),
                    systemImage: "info.circle"
                )
                .scaledFont(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }

            SecureField(L("Password"), text: $password)
                .textFieldStyle(.roundedBorder)

            manualDetails
            Spacer()
        }
        .scaledPadding(20)
    }

    /// Collapsed when discovery worked, open when it did not — the fields are
    /// an escape hatch, not the normal way through.
    private var manualDetails: some View {
        DisclosureGroup(L("Details"), isExpanded: $showManualDetails) {
            Form {
                TextField(L("IMAP Server"), text: $imapHost, prompt: Text(verbatim: "imap.example.com"))
                PortField(title: L("IMAP Port"), port: $imapPort, security: $imapSecurity)
                TextField(L("SMTP Server"), text: $smtpHost, prompt: Text(verbatim: "smtp.example.com"))
                PortField(title: L("SMTP Port"), port: $smtpPort, security: $smtpSecurity)
                TextField(L("Username"), text: $username)
            }
            .formStyle(.columns)
            .scaledPadding(.top, 6)
        }
    }

    private var permissionsStep: some View {
        VStack(spacing: 18 * textScale) {
            Spacer()
            // Nothing is connected yet — the check follows this step — so the
            // address is named without a verdict on it.
            Text(verbatim: email)
                .scaledFont(.headline)

            VStack(spacing: 8 * textScale) {
                Text(L("What may connected assistants do with this account?"))
                    .scaledFont(.callout)
                PresetChips(permissions: $permissions)
                Text(L("Every send waits for your approval, whatever you pick here. You can change this any time."))
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .scaledPadding(.horizontal, 40)
            }
            Spacer()
        }
    }

    // MARK: - Flow

    private var canAttemptLogin: Bool {
        switch effectiveAuth {
        case .oauth(let issuer):
            ProviderCatalog.isConfigured(issuer)
        case .password, .appPassword, .none:
            !password.isEmpty && !imapHost.isEmpty
        }
    }

    private func beginDiscovery() {
        failure = nil
        step = .discovering
        let address = email

        Task {
            let found = await Autodiscovery.resolve(email: address)
            await MainActor.run {
                discovered = found
                seedFields(from: found, email: address)
                // Discovery failing is not an error the user did anything
                // about — it just means the fields are theirs to fill.
                showManualDetails = found == nil
                step = .login
            }
        }
    }

    private func seedFields(from config: DiscoveredConfig?, email: String) {
        username = email
        guard let config else { return }
        imapHost = config.imapHost
        imapPort = config.imapPort
        imapSecurity = config.imapSecurity
        smtpHost = config.smtpHost
        smtpPort = config.smtpPort
        smtpSecurity = config.smtpSecurity
    }

    /// Leaves the login step for the rights. A password needs nothing yet; an
    /// OAuth account makes its browser round trip now, so the token is in the
    /// keychain by the time the check runs.
    private func leaveLogin() {
        failure = nil
        if case .oauth(let issuer) = effectiveAuth {
            Task { await signIn(with: issuer) }
        } else {
            step = .permissions
        }
    }

    /// Saves the secret, proves the account, and only then adds it. A failure
    /// returns to the login step with everything the user typed — and chose —
    /// still there.
    private func beginTrial() {
        failure = nil
        step = .testing
        // An OAuth account already stored its token when it signed in.
        if case .oauth = effectiveAuth {
            runTrial()
            return
        }
        do {
            try KeychainStore.savePassword(password, forAccount: accountID)
        } catch {
            failure = L("Could not save the password to the keychain.")
            step = .login
            return
        }
        runTrial()
    }

    /// The browser round trip, then the same trial every other account gets —
    /// a token that authorizes is not yet proof that IMAP will accept it.
    @MainActor
    private func signIn(with issuer: OAuthIssuer) async {
        do {
            let result = try await OAuthService.signIn(
                issuer: issuer,
                loginHint: email,
                presentationAnchor: NSApplication.shared.keyWindow
            )
            // The provider is the authority on who this is; what was typed in
            // step one was only ever a guess to route the lookup.
            if let identity = result.identity {
                email = identity.email
                username = identity.email
                if let providerName = identity.name, !providerName.isEmpty {
                    name = providerName
                }
            }
            try KeychainStore.savePassword(
                result.tokens.keychainPayload(),
                forAccount: accountID
            )
            step = .permissions
        } catch OAuthService.Failure.cancelled {
            // Closing the window is a decision, not an error to explain.
            failure = nil
        } catch {
            // On the surface a sentence the user can act on; the provider's
            // own wording is a diagnosis and belongs in the log.
            NSLog("TorroMail: sign-in failed: %@", error.localizedDescription)
            failure = L("The sign-in was declined. Try again, or set the account up with an app password.")
        }
    }

    private func runTrial() {
        let candidate = draftAccount()
        let executable = model.generalSettings.mcpExecutable
        Task.detached(priority: .userInitiated) {
            let state = AccountTrial.check(account: candidate, executableName: executable)
            await MainActor.run {
                switch state {
                case .connected:
                    password = ""
                    finish()
                case .failed(let reason):
                    NSLog("TorroMail: account check failed: %@", reason)
                    failure = L("The server did not accept these details. Check the password, or open the server details below.")
                    step = .login
                case .notConfigured, .needsTest:
                    failure = L("The connection could not be checked.")
                    step = .login
                }
            }
        }
    }

    private func draftAccount() -> MailAccount {
        MailAccount(
            id: accountID,
            name: name,
            email: email,
            provider: discovered?.provider ?? .imapSmtp,
            loginMethod: oauthIssuer == nil ? .password : .oauth,
            oauthIssuer: oauthIssuer,
            imapHost: imapHost,
            // Every connection fact comes from the fields, discovered or
            // typed. Reading the SMTP port off `discovered` instead pinned it
            // to a default in exactly the case the fields exist for: a
            // discovery that came up empty.
            imapPort: imapPort,
            imapSecurity: imapSecurity,
            smtpHost: smtpHost,
            smtpPort: smtpPort,
            smtpSecurity: smtpSecurity,
            username: username.isEmpty ? email : username,
            connectionState: .connected,
            permissions: permissions
        )
    }

    /// The issuer only when OAuth is the path actually taken. When the flow
    /// fell back to an app password this is nil, so the saved account records
    /// password auth — which is what the app password really is.
    private var oauthIssuer: OAuthIssuer? {
        if case .oauth(let issuer) = effectiveAuth { return issuer }
        return nil
    }

    private func finish() {
        model.addAccount(draftAccount())
        // The account owns the keychain entry now — nothing to clean up.
        accountID = ""
        dismiss()
    }

    /// A cancelled setup must not leave a password behind under an id no
    /// account will ever claim.
    private func discardLeftovers() {
        guard !accountID.isEmpty else { return }
        KeychainStore.deletePassword(forAccount: accountID)
    }
}
