import SwiftUI
import TorroMailKit

/// Localized user-facing string. English keys are the development language;
/// translations live in `Resources/<lang>.lproj/Localizable.strings`.
private func L(_ key: String) -> String {
    NSLocalizedString(key, bundle: .module, comment: "")
}

private func label(for method: LoginMethod) -> String {
    switch method {
    case .password: L("Password")
    case .oauth: L("OAuth")
    }
}

private func label(for mode: CacheMode) -> String {
    switch mode {
    case .metadata: L("Metadata")
    case .headers: L("Headers")
    case .body: L("Bodies")
    case .fullText: L("Full text")
    }
}

extension Color {
    /// Torro Rot #D50C0C — brand accent from github.com/mahype/torro-design.
    static let torroRed = Color(red: 213 / 255, green: 12 / 255, blue: 12 / 255)
}

@main
struct TorroMailApp: App {
    @StateObject private var model = TorroMailModel.preview()
    @StateObject private var mcpSupervisor = MCPServerSupervisor()

    var body: some Scene {
        WindowGroup {
            TorroMailRootView()
                .environmentObject(model)
                .environmentObject(mcpSupervisor)
                .frame(minWidth: 1080, minHeight: 660)
                .tint(.torroRed)
                .task {
                    mcpSupervisor.start(executableName: model.generalSettings.mcpExecutable)
                }
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button(L("New Account")) {
                    model.beginAccountWizard()
                }
                .keyboardShortcut("n")
            }
        }
    }
}

private struct TorroMailRootView: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        NavigationSplitView {
            List(selection: sidebarSelection) {
                Label(L("Mail Accounts"), systemImage: "envelope")
                    .tag(TorroMailSidebarSelection.accounts)
                    .badge(model.pendingActionCount)
                Label(L("Settings"), systemImage: "gearshape")
                    .tag(TorroMailSidebarSelection.settings)
                Label(L("Log"), systemImage: "list.bullet.rectangle")
                    .tag(TorroMailSidebarSelection.log)
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 200, ideal: 220, max: 280)
            .navigationTitle("TorroMail")
        } detail: {
            detailView
        }
        .sheet(isPresented: $model.showAccountWizard) {
            AccountWizardView()
                .environmentObject(model)
        }
    }

    @ViewBuilder
    private var detailView: some View {
        switch model.selectedSidebarItem {
        case .accounts:
            NavigationStack(path: $model.accountPath) {
                AccountListView()
                    .navigationDestination(for: String.self) { accountID in
                        accountDetail(for: accountID)
                    }
            }
        case .settings:
            SettingsView()
        case .log:
            LogView()
        }
    }

    @ViewBuilder
    private func accountDetail(for accountID: String) -> some View {
        if let index = model.accounts.firstIndex(where: { $0.id == accountID }) {
            AccountDetailView(account: $model.accounts[index])
                .id(accountID)
        } else {
            Text(L("No account selected"))
                .foregroundStyle(.secondary)
        }
    }

    private var sidebarSelection: Binding<TorroMailSidebarSelection?> {
        Binding(
            get: { model.selectedSidebarItem },
            set: { selection in
                guard let selection else { return }
                model.selectedSidebarItem = selection
            }
        )
    }
}

private struct AccountListView: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 10) {
                ForEach(model.accounts) { account in
                    NavigationLink(value: account.id) {
                        AccountCard(account: account)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(20)
            .frame(maxWidth: 720, alignment: .top)
            .frame(maxWidth: .infinity)
        }
        .background(.background.secondary)
        .navigationTitle(L("Mail Accounts"))
        .toolbar {
            Button {
                model.beginAccountWizard()
            } label: {
                Label(L("Add Account"), systemImage: "plus")
            }
        }
    }
}

private struct AccountCard: View {
    var account: MailAccount
    @Environment(\.colorScheme) private var colorScheme
    @State private var isHovering = false

    private let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)

    var body: some View {
        HStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color.torroRed.gradient)
                .frame(width: 34, height: 34)
                .overlay {
                    Image(systemName: "envelope.fill")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(.white)
                }
                .shadow(color: .black.opacity(0.2), radius: 1.5, y: 1)

            VStack(alignment: .leading, spacing: 1) {
                Text(account.name)
                    .font(.headline)
                    .lineLimit(1)
                Text(account.email)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer(minLength: 12)

            if !account.pendingActions.isEmpty {
                Text(account.pendingActions.count.formatted())
                    .font(.caption)
                    .monospacedDigit()
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(.tint, in: Capsule())
                    .foregroundStyle(.white)
            }

            CredentialStatusDot(state: account.connectionState)

            Image(systemName: "chevron.right")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 11)
        .background(.background.secondary, in: shape)
        // The raised look the rest of macOS uses: a lit top edge that fades
        // towards the bottom, over a soft ambient shadow.
        .overlay {
            shape.strokeBorder(edgeGradient, lineWidth: 1)
        }
        .shadow(color: .black.opacity(colorScheme == .dark ? 0.36 : 0.12), radius: 6, y: 3)
        .shadow(color: .black.opacity(colorScheme == .dark ? 0.24 : 0.06), radius: 1, y: 1)
        .contentShape(shape)
        .onHover { isHovering = $0 }
        .animation(.easeOut(duration: 0.12), value: isHovering)
    }

    private var edgeGradient: LinearGradient {
        let top: Color
        let bottom: Color
        if colorScheme == .dark {
            top = .white.opacity(isHovering ? 0.34 : 0.20)
            bottom = .white.opacity(isHovering ? 0.10 : 0.05)
        } else {
            top = .white.opacity(0.9)
            bottom = .black.opacity(isHovering ? 0.18 : 0.10)
        }
        return LinearGradient(colors: [top, bottom], startPoint: .top, endPoint: .bottom)
    }
}

/// Green when the credentials are known good, red when they are broken,
/// orange while the connection has not been verified yet.
private struct CredentialStatusDot: View {
    var state: ConnectionState

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 9, height: 9)
            .help(helpText)
            .accessibilityLabel(helpText)
    }

    private var color: Color {
        if state.isBroken { return .red }
        return state.needsAttention ? .orange : .green
    }

    private var helpText: String {
        switch state {
        case .connected: L("Credentials verified")
        case .needsTest: L("Needs test")
        case .notConfigured: L("Not configured")
        case let .failed(message): message
        }
    }
}

private struct AccountDetailView: View {
    @EnvironmentObject private var model: TorroMailModel
    @Binding var account: MailAccount
    // Transient until Keychain storage lands; never persisted.
    @State private var password = ""
    @State private var confirmRemoval = false

    var body: some View {
        Form {
            connectionSection
            permissionsSection
            foldersSection
            cacheSection
            if !account.pendingActions.isEmpty {
                pendingSection
            }
            Section {
                Button(L("Remove Account…"), role: .destructive) {
                    confirmRemoval = true
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle(account.name)
        .navigationSubtitle(account.email)
        .confirmationDialog(
            String(format: L("Remove “%@”?"), account.name),
            isPresented: $confirmRemoval,
            titleVisibility: .visible
        ) {
            Button(L("Remove Account"), role: .destructive) {
                model.removeAccount(id: account.id)
            }
        } message: {
            Text(L("This only removes the configuration from TorroMail. Your mail stays on the server."))
        }
    }

    private var connectionSection: some View {
        Section(L("Connection")) {
            Picker(L("Provider"), selection: $account.provider) {
                ForEach(Provider.allCases) { provider in
                    Text(provider.rawValue).tag(provider)
                }
            }
            Picker(L("Login"), selection: $account.loginMethod) {
                ForEach(LoginMethod.allCases) { method in
                    Text(label(for: method)).tag(method)
                }
            }

            if account.loginMethod == .password {
                TextField(L("IMAP Server"), text: $account.imapHost, prompt: Text(verbatim: "imap.example.com"))
                TextField(L("SMTP Server"), text: $account.smtpHost, prompt: Text(verbatim: "smtp.example.com"))
                TextField(L("Username"), text: $account.username)
                SecureField(L("Password"), text: $password)
            }

            HStack {
                ConnectionStatusBadge(state: account.connectionState)
                Spacer()
                if account.loginMethod == .oauth {
                    Button(L("Sign In…")) {}
                }
                Button(L("Test Connection")) {}
            }
        }
    }

    private var permissionsSection: some View {
        Section {
            Toggle(L("Read headers"), isOn: $account.permissions.readHeaders)
            Toggle(L("Read body"), isOn: $account.permissions.readBody)
            Toggle(L("Attachments"), isOn: $account.permissions.attachments)
            Toggle(L("Drafts"), isOn: $account.permissions.drafts)
            Toggle(L("Send"), isOn: $account.permissions.send)
            Toggle(L("Mark"), isOn: $account.permissions.mark)
            Toggle(L("Move"), isOn: $account.permissions.move)
            Toggle(L("Delete"), isOn: $account.permissions.delete)
            Toggle(L("Permanent delete"), isOn: $account.permissions.permanentDelete)
        } header: {
            Text(L("Permissions"))
        } footer: {
            Text(L("What connected assistants may do with this account. Send, move, and delete always require your confirmation."))
        }
    }

    private var foldersSection: some View {
        Section {
            ForEach(["INBOX", "Archive", "Sent", "Trash"], id: \.self) { name in
                mailboxToggle(name)
            }
        } header: {
            Text(L("Folders"))
        } footer: {
            Text(L("Folders the assistant may access."))
        }
    }

    private var cacheSection: some View {
        Section {
            Toggle(L("Local cache"), isOn: $account.searchCache.localCacheEnabled)
            if account.searchCache.localCacheEnabled {
                Picker(L("Cache level"), selection: $account.searchCache.cacheMode) {
                    ForEach(CacheMode.allCases) { mode in
                        Text(label(for: mode)).tag(mode)
                    }
                }
                Toggle(L("Full text index"), isOn: $account.searchCache.indexBodies)
                Toggle(L("Attachment index"), isOn: $account.searchCache.indexAttachments)
                LabeledContent(L("Storage")) {
                    Text(account.searchCache.storage)
                    Button(L("Delete")) {}
                }
            }
        } header: {
            Text(L("Search & Cache"))
        } footer: {
            Text(L("More caching makes search faster but stores mail content on this Mac."))
        }
    }

    private var pendingSection: some View {
        Section {
            ForEach(account.pendingActions) { action in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(action.subject)
                            .fontWeight(.medium)
                        Text(verbatim: "\(action.toolCall) → \(action.recipient)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text(action.expiresIn)
                        .font(.caption)
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                    Button(L("Reject"), role: .destructive) {}
                    Button(L("Approve")) {}
                        .buttonStyle(.borderedProminent)
                }
                .padding(.vertical, 2)
            }
        } header: {
            Text(L("Pending Actions"))
        } footer: {
            Text(L("Actions an assistant prepared and is waiting for you to approve."))
        }
    }

    private func mailboxToggle(_ name: String) -> some View {
        Toggle(
            name,
            isOn: Binding(
                get: { account.selectedMailboxes.contains(name) },
                set: { enabled in
                    if enabled {
                        account.selectedMailboxes.insert(name)
                    } else {
                        account.selectedMailboxes.remove(name)
                    }
                }
            )
        )
    }
}

private struct ConnectionStatusBadge: View {
    var state: ConnectionState

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
            Text(text)
                .foregroundStyle(.secondary)
        }
    }

    private var color: Color {
        switch state {
        case .connected: .green
        case .needsTest: .orange
        case .notConfigured: .gray
        case .failed: .red
        }
    }

    private var text: String {
        switch state {
        case .connected: L("Connected")
        case .needsTest: L("Needs test")
        case .notConfigured: L("Not configured")
        case let .failed(message): message
        }
    }
}

private struct SettingsView: View {
    @EnvironmentObject private var model: TorroMailModel
    @EnvironmentObject private var mcpSupervisor: MCPServerSupervisor

    var body: some View {
        Form {
            Section {
                Toggle(L("Start at login"), isOn: $model.generalSettings.launchAtLogin)
            } footer: {
                Text(L("TorroMail and its MCP server start automatically in the background."))
            }

            Section(L("MCP Server")) {
                HStack {
                    Circle()
                        .fill(statusColor)
                        .frame(width: 10, height: 10)
                    Text(statusText)
                    Spacer()
                    if mcpSupervisor.status.isRunning {
                        Button(L("Stop")) {
                            mcpSupervisor.stop()
                        }
                    } else {
                        Button(L("Start")) {
                            mcpSupervisor.start(executableName: model.generalSettings.mcpExecutable)
                        }
                    }
                }
                if let errorDetail {
                    Text(errorDetail)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }

            Section {
                Button(L("Add to Claude Desktop")) {}
                Button(L("Copy Config Snippet")) {}
            } header: {
                Text(L("AI Clients"))
            } footer: {
                Text(L("Connect an assistant so it can use your mail through TorroMail."))
            }
        }
        .formStyle(.grouped)
        .navigationTitle(L("Settings"))
    }

    private var statusColor: Color {
        switch mcpSupervisor.status {
        case .running: .green
        case .starting: .orange
        case .stopped: .gray
        case .notFound, .failed: .red
        }
    }

    private var statusText: String {
        switch mcpSupervisor.status {
        case .running: L("Running")
        case .starting: L("Starting")
        case .stopped: L("Stopped")
        case .notFound, .failed: L("Error")
        }
    }

    private var errorDetail: String? {
        switch mcpSupervisor.status {
        case let .notFound(name):
            String(format: L("MCP executable “%@” not found."), name)
        case let .failed(message):
            message
        default:
            nil
        }
    }
}

private struct LogView: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        Table(model.audit) {
            TableColumn(L("Time"), value: \.time)
            TableColumn(L("Client"), value: \.client)
            TableColumn(L("Account"), value: \.account)
            TableColumn(L("Event"), value: \.event)
            TableColumn(L("Result"), value: \.result)
        }
        .navigationTitle(L("Log"))
        .toolbar {
            Button {
            } label: {
                Label(L("Export…"), systemImage: "square.and.arrow.up")
            }
        }
    }
}

private struct AccountWizardView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: TorroMailModel
    @State private var name = ""
    @State private var email = ""
    @State private var provider = Provider.imapSmtp
    @State private var loginMethod = LoginMethod.password

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section(L("New Account")) {
                    TextField(L("Display Name"), text: $name)
                    TextField(L("Email"), text: $email)
                    Picker(L("Provider"), selection: $provider) {
                        ForEach(Provider.allCases) { provider in
                            Text(provider.rawValue).tag(provider)
                        }
                    }
                    Picker(L("Login"), selection: $loginMethod) {
                        ForEach(LoginMethod.allCases) { method in
                            Text(label(for: method)).tag(method)
                        }
                    }
                }
            }
            .formStyle(.grouped)

            HStack {
                Spacer()
                Button(L("Cancel"), role: .cancel) {
                    dismiss()
                }
                Button(L("Add")) {
                    model.addAccount(
                        name: name,
                        email: email,
                        provider: provider,
                        loginMethod: loginMethod
                    )
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .disabled(name.isEmpty || email.isEmpty)
            }
            .padding()
        }
        .frame(width: 480, height: 340)
    }
}
