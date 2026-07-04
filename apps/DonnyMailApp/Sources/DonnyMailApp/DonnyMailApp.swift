import DonnyMailKit
import SwiftUI

@main
struct DonnyMailApp: App {
    @StateObject private var model = DonnyMailModel.preview()
    @StateObject private var mcpSupervisor = MCPServerSupervisor()

    var body: some Scene {
        WindowGroup {
            DonnyMailRootView()
                .environmentObject(model)
                .environmentObject(mcpSupervisor)
                .frame(minWidth: 1240, minHeight: 740)
                .task {
                    mcpSupervisor.startIfNeeded(settings: model.generalSettings)
                }
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("Account") {
                    model.beginAccountWizard()
                }
                .keyboardShortcut("n")
            }
        }
    }
}

private struct DonnyMailRootView: View {
    @EnvironmentObject private var model: DonnyMailModel

    var body: some View {
        NavigationSplitView {
            List(selection: sidebarSelection) {
                Section(model.sidebarGroups[0].label) {
                    ForEach(model.accounts) { account in
                        AccountSidebarRow(account: account)
                        .tag(DonnyMailSidebarSelection.account(account.id))
                    }
                }

                Section(model.sidebarGroups[1].label) {
                    ForEach(model.sidebarGroups[1].items) { item in
                        SidebarRow(item.selection, item.label, item.symbol)
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 300, ideal: 340, max: 430)
            .navigationTitle("DonnyMail")
            .toolbar {
                Button {
                    model.beginAccountWizard()
                } label: {
                    Label("Add Account", systemImage: "plus")
                }
            }
        } detail: {
            switch model.selectedSidebarItem {
            case .account:
                AccountDetailView(account: model.bindingForSelectedAccount())
            case .aiClients:
                AIClientsView()
            case .generalSettings:
                GeneralSettingsView()
            case .auditLog:
                AuditLogView()
            case .diagnostics:
                DiagnosticsView()
            }
        }
        .sheet(isPresented: $model.showAccountWizard) {
            AccountWizardView()
                .environmentObject(model)
        }
    }

    private var sidebarSelection: Binding<DonnyMailSidebarSelection?> {
        Binding(
            get: { model.selectedSidebarItem },
            set: { selection in
                guard let selection else { return }
                model.selectedSidebarItem = selection
                if case .account = selection {
                    model.selectedAccountSection = .overview
                }
            }
        )
    }
}

private struct AccountSidebarRow: View {
    var account: MailAccount

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 7)
                    .fill(.quaternary)
                Image(systemName: "envelope")
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(.secondary)
            }
            .frame(width: 30, height: 30)

            VStack(alignment: .leading, spacing: 3) {
                Text(account.name)
                    .font(.body)
                    .fontWeight(.medium)
                    .lineLimit(1)
                Text(account.email)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text("\(account.provider.rawValue) · \(account.imapStatus)")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)
        }
        .padding(.vertical, 6)
    }
}

private struct SidebarRow: View {
    var selection: DonnyMailSidebarSelection
    var title: String
    var symbol: String

    init(_ selection: DonnyMailSidebarSelection, _ title: String, _ symbol: String) {
        self.selection = selection
        self.title = title
        self.symbol = symbol
    }

    var body: some View {
        Label(title, systemImage: symbol)
            .padding(.vertical, 2)
            .tag(selection)
    }
}

private struct AccountDetailView: View {
    @EnvironmentObject private var model: DonnyMailModel
    @Binding var account: MailAccount

    var body: some View {
        VStack(spacing: 0) {
            AccountHeaderView(account: account)
                .padding([.horizontal, .top], 24)
                .padding(.bottom, 16)

            Picker("Section", selection: $model.selectedAccountSection) {
                ForEach(DonnyMailAccountSection.allCases) { section in
                    Text(section.label).tag(section)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, 24)
            .padding(.bottom, 12)

            Divider()

            ScrollView {
                Group {
                    switch model.selectedAccountSection {
                    case .overview:
                        AccountOverviewView(account: account)
                    case .connection:
                        AccountConnectionView(account: $account)
                    case .permissions:
                        AccountPermissionsView(permissions: $account.permissions)
                    case .searchCache:
                        AccountSearchCacheView(searchCache: $account.searchCache)
                    case .mailboxes:
                        AccountMailboxesView(account: $account)
                    case .pendingActions:
                        AccountPendingActionsView(account: account)
                    case .advanced:
                        AccountAdvancedView(account: $account)
                    }
                }
                .padding(24)
            }
        }
        .navigationTitle(account.name)
    }
}

private struct AccountHeaderView: View {
    var account: MailAccount

    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 6) {
                Text(account.name)
                    .font(.title2)
                    .fontWeight(.semibold)
                Text(account.email)
                    .foregroundStyle(.secondary)
                Text("\(account.provider.rawValue) · \(account.loginMethod.rawValue) · \(account.imapStatus)")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            HStack {
                Button {
                } label: {
                    Label("Test", systemImage: "checkmark.circle")
                }
                Button {
                } label: {
                    Label("Sync", systemImage: "arrow.triangle.2.circlepath")
                }
            }
        }
    }
}

private struct AccountOverviewView: View {
    var account: MailAccount

    var body: some View {
        Form {
            Section("Status") {
                LabeledContent("Connection", value: account.imapStatus)
                LabeledContent("Authentication", value: account.oauthStatus)
                LabeledContent("Mailboxes", value: account.selectedMailboxes.sorted().joined(separator: ", "))
            }

            Section("Access") {
                LabeledContent("Permissions", value: account.permissions.summary)
                LabeledContent("Search", value: account.searchCache.summary)
                LabeledContent("Pending Actions", value: account.pendingActions.count.formatted())
            }

            Section("Next Actions") {
                HStack {
                    Button {
                    } label: {
                        Label("Review Permissions", systemImage: "switch.2")
                    }
                    Button {
                    } label: {
                        Label("Open Cache", systemImage: "magnifyingglass")
                    }
                }
            }
        }
        .formStyle(.grouped)
    }
}

private struct AccountConnectionView: View {
    @Binding var account: MailAccount

    var body: some View {
        Form {
            Section("Provider") {
                Picker("Provider", selection: $account.provider) {
                    ForEach(Provider.allCases) { provider in
                        Text(provider.rawValue).tag(provider)
                    }
                }
                Picker("Login", selection: $account.loginMethod) {
                    ForEach(LoginMethod.allCases) { method in
                        Text(method.rawValue).tag(method)
                    }
                }
            }

            Section("Connection") {
                LabeledContent("IMAP", value: account.imapStatus)
                LabeledContent("SMTP", value: account.smtpStatus)
                LabeledContent("OAuth", value: account.oauthStatus)
                HStack {
                    Button {
                    } label: {
                        Label("Autodiscover", systemImage: "antenna.radiowaves.left.and.right")
                    }
                    Button {
                    } label: {
                        Label("OAuth", systemImage: "safari")
                    }
                    Button {
                    } label: {
                        Label("Test", systemImage: "checkmark.circle")
                    }
                }
            }
        }
        .formStyle(.grouped)
    }
}

private struct AccountPermissionsView: View {
    @Binding var permissions: PermissionSet

    var body: some View {
        Form {
            Section("Read") {
                Toggle("Read headers", isOn: $permissions.readHeaders)
                Toggle("Read body", isOn: $permissions.readBody)
                Toggle("Attachments", isOn: $permissions.attachments)
            }

            Section("Write") {
                Toggle("Drafts", isOn: $permissions.drafts)
                Toggle("Send", isOn: $permissions.send)
                Toggle("Mark", isOn: $permissions.mark)
                Toggle("Move", isOn: $permissions.move)
            }

            Section("Delete") {
                Toggle("Delete", isOn: $permissions.delete)
                Toggle("Permanent delete", isOn: $permissions.permanentDelete)
            }
        }
        .formStyle(.grouped)
    }
}

private struct AccountSearchCacheView: View {
    @Binding var searchCache: SearchCacheSettings

    var body: some View {
        Form {
            Section("Policy") {
                Toggle("Local cache", isOn: $searchCache.localCacheEnabled)
                Picker("Cache policy", selection: $searchCache.cacheMode) {
                    ForEach(CacheMode.allCases) { mode in
                        Text(mode.rawValue).tag(mode)
                    }
                }
                Toggle("Full text index", isOn: $searchCache.indexBodies)
                Toggle("Attachment index", isOn: $searchCache.indexAttachments)
            }

            Section("Status") {
                LabeledContent("Storage", value: searchCache.storage)
                LabeledContent("Last Sync", value: searchCache.lastSync)
                HStack {
                    Button {
                    } label: {
                        Label("Rebuild", systemImage: "arrow.clockwise")
                    }
                    Button(role: .destructive) {
                    } label: {
                        Label("Delete Index", systemImage: "trash")
                    }
                }
            }
        }
        .formStyle(.grouped)
    }
}

private struct AccountMailboxesView: View {
    @Binding var account: MailAccount

    var body: some View {
        Form {
            Section("Included") {
                MailboxToggle("INBOX", account: $account)
                MailboxToggle("Archive", account: $account)
                MailboxToggle("Sent", account: $account)
                MailboxToggle("Trash", account: $account)
            }

            Section("Mapping") {
                LabeledContent("Archive", value: "Archive")
                LabeledContent("Sent", value: "Sent")
                LabeledContent("Trash", value: "Trash")
            }
        }
        .formStyle(.grouped)
    }
}

private struct MailboxToggle: View {
    var name: String
    @Binding var account: MailAccount

    init(_ name: String, account: Binding<MailAccount>) {
        self.name = name
        self._account = account
    }

    var body: some View {
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

private struct AccountPendingActionsView: View {
    var account: MailAccount

    var body: some View {
        if account.pendingActions.isEmpty {
            ContentUnavailableView("No Pending Actions", systemImage: "checkmark.circle")
        } else {
            Table(account.pendingActions) {
                TableColumn("Tool", value: \.toolCall)
                TableColumn("Subject", value: \.subject)
                TableColumn("Recipient", value: \.recipient)
                TableColumn("Messages") { action in
                    Text(action.affectedMessages.formatted())
                }
                TableColumn("Expires", value: \.expiresIn)
            }
            .frame(minHeight: 280)
        }
    }
}

private struct AccountAdvancedView: View {
    @Binding var account: MailAccount

    var body: some View {
        Form {
            Section("Provider") {
                LabeledContent("Account ID", value: account.id)
                LabeledContent("Provider", value: account.provider.rawValue)
                LabeledContent("Secret Store", value: "Keychain")
            }

            Section("Support") {
                TextEditor(text: $account.notes)
                    .frame(minHeight: 120)
                HStack {
                    Button {
                    } label: {
                        Label("Export", systemImage: "square.and.arrow.up")
                    }
                    Button {
                    } label: {
                        Label("Reset Adapter", systemImage: "arrow.counterclockwise")
                    }
                }
            }
        }
        .formStyle(.grouped)
    }
}

private struct AIClientsView: View {
    @EnvironmentObject private var model: DonnyMailModel

    var body: some View {
        Form {
            Section("Clients") {
                ForEach(model.aiClients) { client in
                    LabeledContent {
                        Text(client.status)
                    } label: {
                        VStack(alignment: .leading) {
                            Text(client.name)
                            Text(client.approvalProfile)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }

            Section("Setup") {
                HStack {
                    Button {
                    } label: {
                        Label("Install", systemImage: "square.and.arrow.down")
                    }
                    Button {
                    } label: {
                        Label("Copy Snippet", systemImage: "doc.on.doc")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .padding(24)
        .navigationTitle("AI Clients")
    }
}

private struct GeneralSettingsView: View {
    @EnvironmentObject private var model: DonnyMailModel
    @EnvironmentObject private var mcpSupervisor: MCPServerSupervisor

    var body: some View {
        Form {
            Section("MCP Server") {
                Toggle("Start with DonnyMail", isOn: $model.generalSettings.startMcpServerWithApp)
                    .onChange(of: model.generalSettings.startMcpServerWithApp) { _, enabled in
                        if enabled {
                            mcpSupervisor.startIfNeeded(settings: model.generalSettings)
                        } else {
                            mcpSupervisor.stop()
                        }
                    }
                LabeledContent("Lifecycle", value: model.generalSettings.mcpLifecycleSummary)
                LabeledContent("Status", value: mcpSupervisor.status.label)
                LabeledContent("Detail", value: mcpSupervisor.status.detail)
                LabeledContent("Transport", value: model.generalSettings.transport)
                LabeledContent("Executable", value: model.generalSettings.executable)
                HStack {
                    Button {
                        mcpSupervisor.startIfNeeded(settings: model.generalSettings)
                    } label: {
                        Label("Start", systemImage: "play")
                    }
                    Button {
                        mcpSupervisor.stop()
                    } label: {
                        Label("Stop", systemImage: "stop")
                    }
                }
            }

            Section("App") {
                LabeledContent("Configuration", value: "Managed by GUI")
                LabeledContent("Secrets", value: "Keychain")
            }
        }
        .formStyle(.grouped)
        .padding(24)
        .navigationTitle("General Settings")
    }
}

private struct AuditLogView: View {
    @EnvironmentObject private var model: DonnyMailModel

    var body: some View {
        Table(model.audit) {
            TableColumn("Time", value: \.time)
            TableColumn("Client", value: \.client)
            TableColumn("Account", value: \.account)
            TableColumn("Event", value: \.event)
            TableColumn("Result", value: \.result)
        }
        .navigationTitle("Audit Log")
    }
}

private struct DiagnosticsView: View {
    var body: some View {
        Form {
            Section("Core") {
                LabeledContent("Rust Core", value: "Ready")
                LabeledContent("Mail Backend", value: "Adapter Pending")
                LabeledContent("MCP Supervision", value: "App-managed")
            }

            Section("Queues") {
                LabeledContent("IMAP", value: "Idle")
                LabeledContent("SMTP", value: "Idle")
                LabeledContent("Indexer", value: "Paused")
            }

            Section("Support") {
                HStack {
                    Button {
                    } label: {
                        Label("Export Diagnostics", systemImage: "square.and.arrow.up")
                    }
                    Button {
                    } label: {
                        Label("Open Logs", systemImage: "doc.text.magnifyingglass")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .padding(24)
        .navigationTitle("Diagnostics")
    }
}

private struct AccountWizardView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: DonnyMailModel
    @State private var name = ""
    @State private var email = ""
    @State private var provider = Provider.gmail
    @State private var loginMethod = LoginMethod.oauth

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section("Account") {
                    TextField("Display Name", text: $name)
                    TextField("Email", text: $email)
                    Picker("Provider", selection: $provider) {
                        ForEach(Provider.allCases) { provider in
                            Text(provider.rawValue).tag(provider)
                        }
                    }
                    Picker("Login", selection: $loginMethod) {
                        ForEach(LoginMethod.allCases) { method in
                            Text(method.rawValue).tag(method)
                        }
                    }
                }

                Section("Setup") {
                    HStack {
                        Button {
                        } label: {
                            Label("Autodiscover", systemImage: "antenna.radiowaves.left.and.right")
                        }
                        Button {
                        } label: {
                            Label("OAuth", systemImage: "safari")
                        }
                        Button {
                        } label: {
                            Label("Test", systemImage: "checkmark.circle")
                        }
                    }
                }
            }
            .formStyle(.grouped)

            HStack {
                Spacer()
                Button("Cancel", role: .cancel) {
                    dismiss()
                }
                Button("Add") {
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
        .frame(width: 560, height: 420)
    }
}
