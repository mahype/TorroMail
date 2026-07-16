import SwiftUI
import TorroMailKit

/// The MCP Server area: the one server TorroMail runs, and the assistants it
/// is wired into. It mirrors the accounts area — a card list that drills into a
/// per-client setup screen — because the two are the app's twin control
/// surfaces: mailboxes on one side, the assistants that reach them on the other.
struct MCPServerListView: View {
    @EnvironmentObject private var model: TorroMailModel
    /// Cheap install/config facts per client, refreshed on appear. The server
    /// self-test is left to the detail view — it launches a process, too heavy
    /// for a list that redraws.
    @State private var statuses: [String: MCPClientSetupStatus] = [:]

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 10) {
                ForEach(MCPClientRegistry.catalog) { descriptor in
                    NavigationLink(value: descriptor.id) {
                        MCPClientRow(
                            descriptor: descriptor,
                            status: statuses[descriptor.id]
                                ?? MCPClientSetupStatus(isInstalled: false, isConfigured: false)
                        )
                    }
                    .buttonStyle(.plain)
                }

                Text(L("Pick an assistant to connect it — TorroMail writes itself into the ones it knows, and hands you a snippet for the rest. Nothing is sent until you restart the assistant."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 2)
                    .padding(.top, 2)
            }
            .padding(20)
            .frame(maxWidth: 720, alignment: .top)
            .frame(maxWidth: .infinity)
        }
        .background(.background.secondary)
        .navigationTitle(L("MCP Clients"))
        .onAppear(perform: refresh)
        // Popping back from a client's detail view doesn't re-fire the root's
        // `.onAppear`, so a connect/disconnect made in there would leave the
        // cached statuses stale. The path shrinking back to the list is the
        // signal to re-read them.
        .onChange(of: model.mcpPath) { refresh() }
    }

    private func refresh() {
        var next: [String: MCPClientSetupStatus] = [:]
        for descriptor in MCPClientRegistry.catalog {
            next[descriptor.id] = MCPClientSetup.status(
                for: descriptor,
                executableName: model.generalSettings.mcpExecutable,
                runServerTest: false
            )
        }
        statuses = next
        model.connectedClients = MCPClientRegistry.catalog
            .filter { next[$0.id]?.isConfigured == true }
            .map(\.displayName)
    }
}

/// TorroMail's own server: is the background process the assistants talk to
/// running? It serves the mail accounts, so it leads the accounts list rather
/// than the client list here.
struct MCPServerStatusCard: View {
    @EnvironmentObject private var model: TorroMailModel
    @EnvironmentObject private var mcpSupervisor: MCPServerSupervisor

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(Color.torroRed.gradient)
                    .frame(width: 34, height: 34)
                    .overlay {
                        Image(systemName: "server.rack")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(.white)
                    }
                VStack(alignment: .leading, spacing: 1) {
                    Text(L("TorroMail server"))
                        .font(.headline)
                    Text(statusText)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 8)
                Circle()
                    .fill(statusColor)
                    .frame(width: 9, height: 9)
                if mcpSupervisor.status.isRunning {
                    Button(L("Stop")) { mcpSupervisor.stop() }
                        .torroButton()
                } else {
                    Button(L("Start")) {
                        mcpSupervisor.start(executableName: model.generalSettings.mcpExecutable)
                    }
                    .torroButton()
                }
            }
            if let errorDetail {
                Text(errorDetail)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .torroCard()
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
        case .running: L("Running in the background")
        case .starting: L("Starting")
        case .stopped: L("Stopped")
        case .notFound, .failed: L("Error")
        }
    }

    private var errorDetail: String? {
        switch mcpSupervisor.status {
        case let .notFound(name): String(format: L("MCP executable “%@” not found."), name)
        case let .failed(message): message
        default: nil
        }
    }
}

/// A client's card in the list. Mirrors `AccountCard`: badge, name, a summary
/// line that reads as the current state, a status dot, and the chevron in.
private struct MCPClientRow: View {
    var descriptor: MCPClientDescriptor
    var status: MCPClientSetupStatus
    @State private var isHovering = false

    private var isManual: Bool { descriptor.kind == .manual }

    var body: some View {
        HStack(spacing: 12) {
            MCPClientBadge(symbol: descriptor.symbol, dimmed: !isManual && !status.isInstalled)
            VStack(alignment: .leading, spacing: 1) {
                Text(L(descriptor.displayName))
                    .font(.headline)
                    .foregroundStyle(isManual || status.isInstalled ? .primary : .secondary)
                Text(summary)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 12)
            if let dotColor {
                Circle()
                    .fill(dotColor)
                    .frame(width: 9, height: 9)
                    .help(summary)
            }
            Image(systemName: "chevron.right")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 11)
        .torroCard(isHighlighted: isHovering)
        .onHover { isHovering = $0 }
        .animation(.easeOut(duration: 0.12), value: isHovering)
    }

    private var summary: String {
        if isManual { return L("Manual setup") }
        if !status.isInstalled { return L("Not found on this Mac") }
        return status.isConfigured ? L("Connected") : L("Not connected")
    }

    /// Manual entries carry no live state, so they show no dot.
    private var dotColor: Color? {
        if isManual { return nil }
        if !status.isInstalled { return .gray }
        return status.isConfigured ? .green : .orange
    }
}

/// The tile that stands for a client — a neutral counterpart to the red mail
/// badge, so the two lists read as siblings without the assistants borrowing
/// the mailbox's brand colour.
private struct MCPClientBadge: View {
    var symbol: String
    var dimmed: Bool
    var size: CGFloat = 34

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
            .fill(.quaternary)
            .frame(width: size, height: size)
            .overlay {
                Image(systemName: symbol)
                    .font(.system(size: size * 0.42, weight: .semibold))
                    .foregroundStyle(dimmed ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.secondary))
            }
    }
}

/// One client's setup screen. Connect or disconnect, run the setup test, and —
/// for the step TorroMail cannot see from here — read how to confirm inside the
/// client itself.
struct MCPClientDetailView: View {
    @EnvironmentObject private var model: TorroMailModel
    let descriptor: MCPClientDescriptor

    @State private var status = MCPClientSetupStatus(isInstalled: false, isConfigured: false)
    @State private var isBusy = false
    @State private var note: String?
    @State private var snippetCopied = false
    /// The exact text the copy button puts on the clipboard, so the code block
    /// and the button can never drift apart. Nil when the server binary is not
    /// found and there is no real path to show.
    @State private var configSnippet: String?

    var body: some View {
        Form {
            switch descriptor.kind {
            case .automatic:
                connectionSection
                testSection
                confirmSection
                manualSection
            case .manual:
                manualHeaderSection
                confirmSection
                manualSection
            }
        }
        .formStyle(.grouped)
        .navigationTitle(L(descriptor.displayName))
        .onAppear {
            refresh(runServerTest: false)
            loadSnippet()
        }
    }

    /// The lead section for a manual client: it says plainly there is no
    /// one-click path, so the snippet below is the way in.
    private var manualHeaderSection: some View {
        Section {
            HStack(spacing: 12) {
                MCPClientBadge(symbol: descriptor.symbol, dimmed: false)
                VStack(alignment: .leading, spacing: 1) {
                    Text(L(descriptor.displayName)).font(.headline)
                    Text(L("Set up by hand"))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(.vertical, 2)
        } header: {
            Text(L("Connection"))
        } footer: {
            Text(L("TorroMail does not configure this client automatically. Add the snippet below to its MCP configuration yourself."))
        }
    }

    private var connectionSection: some View {
        Section {
            HStack(spacing: 12) {
                MCPClientBadge(symbol: descriptor.symbol, dimmed: !status.isInstalled)
                VStack(alignment: .leading, spacing: 1) {
                    Text(L(descriptor.displayName)).font(.headline)
                    Text(stateText)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Circle().fill(dotColor).frame(width: 9, height: 9)
            }
            .padding(.vertical, 2)

            if status.isInstalled {
                HStack {
                    if status.isConfigured {
                        Button(L("Disconnect"), role: .destructive) { disconnect() }
                            .disabled(isBusy)
                        Spacer()
                        Button(L("Reconnect")) { connect() }
                            .torroButton()
                            .disabled(isBusy)
                    } else {
                        Spacer()
                        Button(L("Connect")) { connect() }
                            .torroButton()
                            .disabled(isBusy)
                    }
                }
            } else {
                Text(String(format: L("%@ is not installed on this Mac. Install it, or set it up by hand with the snippet below."), descriptor.displayName))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if let note {
                Text(note)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } header: {
            Text(L("Connection"))
        }
    }

    @ViewBuilder
    private var testSection: some View {
        if status.isInstalled {
            Section {
                Button(L("Test setup")) { refresh(runServerTest: true) }
                    .torroButton()
                    .disabled(isBusy)
                checkRow(
                    ok: status.isConfigured,
                    title: L("Written into %@’s configuration"),
                    pending: L("Not connected yet")
                )
                serverCheckRow
            } header: {
                Text(L("Setup test"))
            } footer: {
                Text(L("TorroMail can prove its own server answers and that this client points at it. Whether the client has loaded it only shows after a restart — see below."))
            }
        }
    }

    private var confirmSection: some View {
        Section {
            Text(L(descriptor.verificationHintKey))
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
        } header: {
            Text(descriptor.kind == .manual
                ? L("How to set it up")
                : String(format: L("Confirm inside %@"), descriptor.displayName))
                .textCase(nil)
        }
    }

    private var manualSection: some View {
        Section {
            if let path = configFilePath {
                LabeledContent(L("Config file")) {
                    Text(verbatim: path)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            if let configSnippet {
                codeBlock(configSnippet)
                HStack {
                    Button(L("Copy config snippet")) { copySnippet() }
                        .torroButton()
                    Spacer()
                    if snippetCopied {
                        Label(L("Copied to the clipboard."), systemImage: "checkmark")
                            .labelStyle(.titleAndIcon)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            } else {
                Text(L("The MCP server binary was not found."))
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        } header: {
            Text(L("Manual setup"))
        } footer: {
            Text(L("For a client TorroMail does not configure automatically, paste this into its MCP configuration."))
        }
    }

    /// The snippet as a code card: monospaced, on a tinted ground, scrolling
    /// sideways so a long binary path never forces an ugly wrap.
    private func codeBlock(_ text: String) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Text(verbatim: text)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(codeGround, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(.separator, lineWidth: 0.5)
        }
        .listRowInsets(EdgeInsets(top: 6, leading: 12, bottom: 6, trailing: 12))
    }

    private var codeGround: some ShapeStyle {
        Color.primary.opacity(0.05)
    }

    // MARK: - Rows

    private func checkRow(ok: Bool, title: String, pending: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: ok ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(ok ? AnyShapeStyle(.green) : AnyShapeStyle(.secondary))
            Text(ok ? String(format: title, descriptor.displayName) : pending)
            Spacer()
        }
    }

    @ViewBuilder
    private var serverCheckRow: some View {
        switch status.server {
        case .unknown:
            EmptyView()
        case let .responds(count):
            HStack(spacing: 8) {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                Text(String(format: L("TorroMail server answers — %d tools"), count))
                Spacer()
            }
        case let .failed(message):
            HStack(spacing: 8) {
                Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
                VStack(alignment: .leading, spacing: 1) {
                    Text(L("TorroMail server did not answer"))
                    Text(message).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
            }
        }
    }

    // MARK: - State

    private var stateText: String {
        if !status.isInstalled { return L("Not found on this Mac") }
        return status.isConfigured ? L("Connected") : L("Not connected")
    }

    private var dotColor: Color {
        if !status.isInstalled { return .gray }
        return status.isConfigured ? .green : .orange
    }

    private func prettyPath(_ url: URL) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return url.path.hasPrefix(home)
            ? "~" + url.path.dropFirst(home.count)
            : url.path
    }

    /// The config file to name in the manual section: an installed client's
    /// real path, or a manual client's documented one.
    private var configFilePath: String? {
        if let client = MCPClientRegistry.installedClient(id: descriptor.id) {
            return prettyPath(client.configURL)
        }
        return descriptor.manualConfigPath
    }

    // MARK: - Actions

    /// File facts are cheap and run inline; the server self-test launches a
    /// process, so it goes off the main actor and lands back when it returns.
    private func refresh(runServerTest: Bool) {
        let executable = model.generalSettings.mcpExecutable
        let descriptor = descriptor
        if !runServerTest {
            status = MCPClientSetup.status(for: descriptor, executableName: executable, runServerTest: false)
            model.connectedClients = MCPClientRegistry.catalog
                .filter { MCPClientSetup.status(for: $0, executableName: executable, runServerTest: false).isConfigured }
                .map(\.displayName)
            return
        }
        isBusy = true
        Task.detached(priority: .userInitiated) {
            let result = MCPClientSetup.status(for: descriptor, executableName: executable, runServerTest: true)
            await MainActor.run {
                status = result
                isBusy = false
            }
        }
    }

    private func connect() {
        note = nil
        guard let client = MCPClientRegistry.installedClient(id: descriptor.id) else { return }
        guard let commandPath = MCPClientSetup.serverCommandPath(
            executableName: model.generalSettings.mcpExecutable
        ) else {
            note = L("The MCP server binary was not found.")
            return
        }
        do {
            try MCPClientSetup.add(to: client, commandPath: commandPath)
            note = String(format: L("Connected. Restart %@ to load it."), descriptor.displayName)
        } catch let failure as MCPClientSetup.Failure {
            note = L(failure.reason)
        } catch {
            note = L("Could not update the configuration.")
        }
        refresh(runServerTest: false)
    }

    private func disconnect() {
        note = nil
        guard let client = MCPClientRegistry.installedClient(id: descriptor.id) else { return }
        do {
            try MCPClientSetup.remove(from: client)
            note = String(format: L("Removed from %@."), descriptor.displayName)
        } catch let failure as MCPClientSetup.Failure {
            note = L(failure.reason)
        } catch {
            note = L("Could not update the configuration.")
        }
        refresh(runServerTest: false)
    }

    /// Resolves the snippet once, so the code block shows exactly what the
    /// button copies.
    private func loadSnippet() {
        guard let commandPath = MCPClientSetup.serverCommandPath(
            executableName: model.generalSettings.mcpExecutable
        ) else {
            configSnippet = nil
            return
        }
        configSnippet = MCPClientSetup.configSnippet(
            commandPath: commandPath,
            format: descriptor.snippetFormat
        )
    }

    private func copySnippet() {
        guard let configSnippet else { return }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(configSnippet, forType: .string)
        snippetCopied = true
    }
}
