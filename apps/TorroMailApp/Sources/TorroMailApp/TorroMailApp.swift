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
                Section(L("Accounts")) {
                    ForEach(model.accounts) { account in
                        AccountSidebarRow(account: account)
                            .tag(TorroMailSidebarSelection.account(account.id))
                            .badge(account.pendingActions.count)
                    }
                }

                Section {
                    Label(L("Settings"), systemImage: "gearshape")
                        .tag(TorroMailSidebarSelection.settings)
                    Label(L("Log"), systemImage: "list.bullet.rectangle")
                        .tag(TorroMailSidebarSelection.log)
                }
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 360)
            .navigationTitle("TorroMail")
            .toolbar {
                Button {
                    model.beginAccountWizard()
                } label: {
                    Label(L("Add Account"), systemImage: "plus")
                }
            }
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
        case let .account(accountID):
            if let index = model.accounts.firstIndex(where: { $0.id == accountID }) {
                AccountDetailView(account: $model.accounts[index])
                    .id(accountID)
            } else {
                Text(L("No account selected"))
                    .foregroundStyle(.secondary)
            }
        case .settings:
            SettingsView()
        case .log:
            LogView()
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

private struct AccountSidebarRow: View {
    var account: MailAccount

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(account.name)
                    .fontWeight(.medium)
                    .lineLimit(1)
                Text(account.email)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer(minLength: 8)

            // Quiet when healthy: the dot only appears when the connection
            // needs the user's attention.
            if account.connectionState.needsAttention {
                Circle()
                    .fill(.orange)
                    .frame(width: 8, height: 8)
            }
        }
        .padding(.vertical, 4)
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
