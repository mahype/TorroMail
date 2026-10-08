import SwiftUI
import TorroMailKit

/// Operational setup only: pick bots, grant accounts, configure and revoke.
struct HermesClientView: View {
    @EnvironmentObject private var model: TorroMailModel
    @AppStorage("hermesSetupConnectionID") private var savedConnectionID = ""
    @State private var connections: [HermesConnection] = []
    @State private var connectionID: String = ""
    @State private var bots: [HermesBot] = []
    @State private var accounts: [HermesSharedAccount] = []
    @State private var selected = Set<String>()
    @State private var access: [String: ClientAccountAccess] = [:]
    @State private var results: [String: HermesBotResult] = [:]
    @State private var busy = false
    @State private var problem: String?
    @State private var remoteWithoutSSH = false
    @State private var loaded = false

    var body: some View {
        Form {
            Section {
                Picker(L("Hermes connection"), selection: $connectionID) {
                    if connectionID.isEmpty { Text(L("Choose a connection")).tag("") }
                    ForEach(connections) { connection in
                        Text(connection.isRemote ? connection.name : L("This computer")).tag(connection.id)
                    }
                }
                .disabled(busy)
                if remoteWithoutSSH {
                    Text(L("Hermes uses a remote backend. Choose its saved SSH connection to configure bots on that computer."))
                        .foregroundStyle(.secondary)
                }
                Button(L("Refresh bots")) { refresh() }.disabled(busy || connectionID.isEmpty)
            }
            if let problem {
                Section {
                    Text(L(problem)).foregroundStyle(.red)
                    Button(L("Try again")) { refresh() }.disabled(busy)
                }
            }
            if busy {
                Section { HStack { ProgressView().controlSize(.small); Text(L("Updating Hermes bots…")) } }
            }
            if loaded, bots.isEmpty {
                Section { Text(L("No Hermes bots were found on this computer.")) }
            }
            ForEach(bots) { bot in
                botSection(bot)
            }
            if !bots.isEmpty {
                Section {
                    HStack {
                        Spacer()
                        Button(L("Set up selected bots")) { perform(.connect, ids: selected) }
                            .torroButton()
                            .disabled(busy || selected.isEmpty)
                    }
                } footer: {
                    Text(L("Only selected bots are configured. Each bot gets its own access key. Account permissions still apply."))
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Hermes")
        .task { discover() }
        .onChange(of: connectionID) { _, _ in
            if !connectionID.isEmpty { savedConnectionID = connectionID }
            bots = []; accounts = []; selected = []; access = [:]; results = [:]; loaded = false
            if !connectionID.isEmpty { refresh() }
        }
    }

    private func botSection(_ bot: HermesBot) -> some View {
        Section {
            Toggle(isOn: Binding(get: { selected.contains(bot.id) }, set: { value in
                if value { selected.insert(bot.id) } else { selected.remove(bot.id) }
            })) {
                HStack {
                    Text(verbatim: bot.name == "default" ? L("Standard bot") : bot.name)
                    Spacer()
                    if bot.isReady { Circle().fill(.green).frame(width: 8, height: 8).accessibilityLabel(L("Configured")) }
                }
            }
            .toggleStyle(.checkbox)
            .disabled(busy)
            if selected.contains(bot.id) {
                DisclosureGroup {
                    accountPicker(bot)
                } label: {
                    HStack {
                        Text(L("Shared accounts"))
                        Spacer()
                        Text(accountSummary(bot)).foregroundStyle(.secondary)
                    }
                }
            }
            if let result = results[bot.id] {
                if let issue = result.problem { Text(L(issue)).foregroundStyle(.red) }
            } else if let issue = bot.problem { Text(L(issue)).foregroundStyle(.red) }
            if bot.needsReload {
                Text(L("Setup verified. Hermes can load the change automatically. If this bot still has no mail access, run “/reload-mcp” in Hermes or start a new session."))
                    .foregroundStyle(.secondary)
            }
            if bot.configured {
                HStack {
                    Button(L("Disconnect"), role: .destructive) { perform(.disconnect, ids: [bot.id]) }
                    Spacer()
                    Button(L("Test access")) { perform(.test, ids: [bot.id]) }
                }
                .disabled(busy)
            }
        }
    }

    private func accountPicker(_ bot: HermesBot) -> some View {
        let grant = access[bot.id] ?? .selected([])
        return VStack(alignment: .leading, spacing: 8) {
            Picker(L("Shared accounts"), selection: Binding(get: { grant == .all }, set: { all in
                access[bot.id] = all ? .all : .selected(Set(accounts.map(\.id)))
            })) {
                Text(L("All accounts")).tag(true)
                Text(L("Selected accounts")).tag(false)
            }
            if case .selected(let ids) = grant {
                ForEach(accounts) { account in
                    Toggle(isOn: Binding(get: { ids.contains(account.id) }, set: { allowed in
                        var next = ids
                        if allowed { next.insert(account.id) } else { next.remove(account.id) }
                        access[bot.id] = .selected(next)
                    })) {
                        VStack(alignment: .leading) {
                            Text(verbatim: account.name)
                            Text(verbatim: account.email).scaledFont(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .toggleStyle(.checkbox)
                }
                if ids.intersection(Set(accounts.map(\.id))).isEmpty {
                    Text(L("No accounts are shared with this client.")).foregroundStyle(.secondary)
                }
            } else {
                Text(L("This client can access all accounts, including accounts added later. Account permissions still apply."))
                    .scaledFont(.caption).foregroundStyle(.secondary)
            }
        }
        .disabled(busy)
    }

    private func accountSummary(_ bot: HermesBot) -> String {
        switch access[bot.id] ?? .selected([]) {
        case .all: L("All accounts")
        case .selected(let ids): String(format: L("%d accounts"), ids.intersection(Set(accounts.map(\.id))).count)
        }
    }

    private func discover() {
        guard connections.isEmpty, !busy else { return }
        busy = true
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try HermesConnections.discover() }
            await MainActor.run {
                busy = false
                switch outcome {
                case .success(let found):
                    connections = found.connections
                    remoteWithoutSSH = found.remoteWithoutSSH
                    connectionID = found.connections.contains(where: { $0.id == savedConnectionID }) ? savedConnectionID : found.preferredID ?? ""
                case .failure(let error): problem = reason(error)
                }
            }
        }
    }

    private func refresh() { perform(.list, ids: []) }

    private func perform(_ action: HermesControlRequest.Action, ids: Set<String>) {
        guard !busy, let connection = connections.first(where: { $0.id == connectionID }) else { return }
        let request = HermesControlRequest(action: action, selections: ids.sorted().map { HermesBotSelection(id: $0, access: access[$0] ?? .selected([])) })
        busy = true; problem = nil
        Task.detached(priority: .userInitiated) {
            let outcome = Result { try HermesConnections.run(request, on: connection) }
            await MainActor.run {
                busy = false
                switch outcome {
                case .success(let response):
                    let firstLoad = !loaded
                    bots = response.bots; accounts = response.accounts; loaded = true
                    if firstLoad { selected = Set(bots.filter(\.configured).map(\.id)) }
                    selected.formIntersection(Set(bots.map(\.id)))
                    for bot in bots {
                        if action == .list || response.results.first(where: { $0.id == bot.id })?.success == true { access[bot.id] = bot.access }
                    }
                    for result in response.results { results[result.id] = result }
                    if action == .disconnect { for result in response.results where result.success { selected.remove(result.id); results[result.id] = nil } }
                    if !connection.isRemote { model.reloadClientConnections() }
                case .failure(let error): problem = reason(error)
                }
            }
        }
    }

    private func reason(_ error: Error) -> String {
        let message = (error as? MCPClientSetup.Failure)?.reason ?? "The Hermes connections could not be read."
        NSLog("TorroMail Hermes setup: %@", message)
        return message
    }
}
