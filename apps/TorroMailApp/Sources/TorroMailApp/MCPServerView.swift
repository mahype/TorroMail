import SwiftUI
import TorroMailKit

/// The MCP Server area: the one server TorroMail runs, and the assistants it
/// is wired into. It mirrors the accounts area — a card list that drills into a
/// per-client setup screen — because the two are the app's twin control
/// surfaces: mailboxes on one side, the assistants that reach them on the other.
struct MCPServerListView: View {
    @Environment(\.textScale) private var textScale
    @EnvironmentObject private var model: TorroMailModel
    /// Cheap install/config facts per client, refreshed on appear. The server
    /// self-test is left to the detail view — it launches a process, too heavy
    /// for a list that redraws.
    @State private var statuses: [String: MCPClientSetupStatus] = [:]

    /// Two equal columns. Each card is then "half the width, minus the gap" —
    /// the pairing the layout is built around.
    private var columns: [GridItem] {
        let column = GridItem(.flexible(), spacing: 10 * textScale)
        return textScale > 1.45 ? [column] : [column, column]
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 12 * textScale) {
                // Two columns: the cards go half-width and pair up, leaving room
                // to grow the field of assistants without a taller scroll.
                LazyVGrid(columns: columns, spacing: 10 * textScale) {
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
                }

                Text(L("Pick an assistant to connect it — TorroMail writes itself into the ones it knows, and hands you a snippet for the rest. Nothing is sent until you restart the assistant."))
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .scaledPadding(.horizontal, 2)
            }
            .scaledPadding(20)
            .readableContentWidth()
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
        let executable = model.generalSettings.mcpExecutable
        let openClawSettings = model.generalSettings.openClaw
        let catalog = MCPClientRegistry.catalog
        Task {
            let next = await Task.detached(priority: .utility) {
                var result: [String: MCPClientSetupStatus] = [:]
                for descriptor in catalog {
                    result[descriptor.id] = MCPClientSetup.status(
                        for: descriptor,
                        executableName: executable,
                        runServerTest: false,
                        openClawSettings: openClawSettings
                    )
                }
                return result
            }.value
            statuses = next
            model.connectedClients = catalog
                .filter { next[$0.id]?.hasCurrentKey == true }
                .map(\.displayName)
        }
    }
}

/// TorroMail's own server: is the background process the assistants talk to
/// running? It serves the mail accounts, so it leads the accounts list rather
/// than the client list here.
struct MCPServerStatusCard: View {
    @Environment(\.textScale) private var textScale
    @EnvironmentObject private var model: TorroMailModel
    @EnvironmentObject private var mcpSupervisor: MCPServerSupervisor

    var body: some View {
        VStack(alignment: .leading, spacing: 10 * textScale) {
            HStack(spacing: 12 * textScale) {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(Color.torroRed.gradient)
                    .frame(width: 34 * textScale, height: 34 * textScale)
                    .overlay {
                        Image(systemName: "server.rack")
                            .scaledSymbolFont(size: 15, weight: .semibold)
                            .foregroundStyle(.white)
                    }
                VStack(alignment: .leading, spacing: 1 * textScale) {
                    Text(L("TorroMail server"))
                        .scaledFont(.headline)
                    Text(statusText)
                        .scaledFont(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 8)
                Circle()
                    .fill(statusColor)
                    .frame(width: 9 * textScale, height: 9 * textScale)
                    .help(statusText)
                    .accessibilityLabel(statusText)
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
            // A diagnosis, not a decision: it sits as a footnote where it
            // happened. The dot above already carries the colour, and the raw
            // process message stays in the log.
            if let errorDetail {
                Text(errorDetail)
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .scaledPadding(.horizontal, 14)
        .scaledPadding(.vertical, 12)
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

    /// The answer to the user's question — "can assistants reach my mail?" —
    /// never the process state behind it.
    private var statusText: String {
        switch mcpSupervisor.status {
        case .running: L("Ready for assistants")
        case .starting: L("Starting up…")
        case .stopped: L("Not reachable for assistants")
        case .notFound, .failed: L("Not answering")
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
    @Environment(\.textScale) private var textScale
    var descriptor: MCPClientDescriptor
    var status: MCPClientSetupStatus
    @State private var isHovering = false

    private var isManual: Bool { descriptor.kind == .manual }

    var body: some View {
        HStack(spacing: 12 * textScale) {
            MCPClientBadge(symbol: descriptor.symbol, dimmed: !isManual && !status.isInstalled)
            VStack(alignment: .leading, spacing: 1 * textScale) {
                Text(L(descriptor.displayName))
                    .scaledFont(.headline)
                    .foregroundStyle(isManual || status.isInstalled ? .primary : .secondary)
                Text(summary)
                    .scaledFont(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 12)
            if let dotColor {
                Circle()
                    .fill(dotColor)
                    .frame(width: 9 * textScale, height: 9 * textScale)
                    .help(summary)
                    .accessibilityLabel(summary)
            }
            Image(systemName: "chevron.right")
                .scaledSymbolFont(size: 12, weight: .semibold)
                .foregroundStyle(.tertiary)
        }
        .scaledPadding(.horizontal, 14)
        .scaledPadding(.vertical, 11)
        .torroCard(isHighlighted: isHovering)
        .onHover { isHovering = $0 }
        .animation(.easeOut(duration: 0.12), value: isHovering)
    }

    private var summary: String {
        if isManual { return L("Manual setup") }
        if !status.isInstalled { return L("Not found on this Mac") }
        if status.setupAvailability == .setupToolMissing { return L("Setup tool not found") }
        if status.setupAvailability == .mcpUnavailable { return L("MCP setup unavailable") }
        if status.isConfigured && !status.hasCurrentKey { return L("Access key missing") }
        return status.isConfigured ? L("Connected") : L("Not connected")
    }

    /// Manual entries carry no live state, so they show no dot.
    private var dotColor: Color? {
        if isManual { return nil }
        if !status.isInstalled { return .gray }
        if status.setupAvailability != .ready { return .orange }
        return status.hasCurrentKey ? .green : .orange
    }
}

/// The tile that stands for a client — a neutral counterpart to the red mail
/// badge, so the two lists read as siblings without the assistants borrowing
/// the mailbox's brand colour.
private struct MCPClientBadge: View {
    @Environment(\.textScale) private var textScale
    var symbol: String
    var dimmed: Bool
    var size: CGFloat = 34

    var body: some View {
        RoundedRectangle(cornerRadius: size * textScale * 0.24, style: .continuous)
            .fill(.quaternary)
            .frame(width: size * textScale, height: size * textScale)
            .overlay {
                Image(systemName: symbol)
                    .scaledSymbolFont(size: size * 0.42, weight: .semibold)
                    .foregroundStyle(dimmed ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.secondary))
            }
    }
}

/// One client's setup screen. Connect or disconnect, run the setup test, and —
/// for the step TorroMail cannot see from here — read how to confirm inside the
/// client itself.
struct MCPClientDetailView: View {
    private static let openClawRestartCommand = "openclaw gateway restart"

    @Environment(\.textScale) private var textScale
    @EnvironmentObject private var model: TorroMailModel
    let descriptor: MCPClientDescriptor

    @State private var status = MCPClientSetupStatus(isInstalled: false, isConfigured: false)
    /// The default state is not evidence that a client is absent. OpenClaw's
    /// CLI discovery runs off the main actor, so show an honest checking state
    /// until the first result lands instead of briefly claiming it is missing.
    @State private var hasCheckedStatus = false
    @State private var isBusy = false
    @State private var note: String?
    @State private var snippetCopied = false
    /// The exact text the copy button puts on the clipboard — access key in
    /// the clear, because a masked config is not a config. Nil when the
    /// server binary is not found and there is no real path to show.
    @State private var realSnippet: String?
    /// The same snippet with the key masked: what the screen shows until the
    /// user opts to reveal. Identical text otherwise, so what you read is
    /// what you paste.
    @State private var maskedSnippet: String?
    @State private var revealKey = false
    @State private var showOpenClawLocation = false
    /// When this client's access key last changed in this sitting. The client
    /// carries the old one until it restarts, so this is what the restart
    /// notice is measured against — see `MCPClientKeyStore.restartPending`.
    @State private var keyChangedAt: Date?

    var body: some View {
        Form {
            restartSection
            ClientAccountAccessSection(clientID: descriptor.id)
            switch descriptor.kind {
            case .automatic:
                connectionSection
                if descriptor.id == "clawbot", showOpenClawLocation {
                    openClawLocationSection
                }
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
            showOpenClawLocation = model.generalSettings.openClaw.hasOverrides
                || status.setupAvailability != .ready
            loadSnippet()
        }
    }

    /// The one thing TorroMail cannot do for the user, said where it cannot be
    /// missed: a new key only reaches the assistant when the assistant starts
    /// again. It leads the screen because until it is done, everything below
    /// it — green dots included — describes a client that is still failing
    /// every call. It clears itself the moment the client connects again, so
    /// it never becomes a banner people learn to ignore.
    @ViewBuilder
    private var restartSection: some View {
        if mustRestart {
            Section {
                HStack(alignment: .top, spacing: 12 * textScale) {
                    Image(systemName: "arrow.clockwise.circle.fill")
                        .scaledFont(.title2)
                        .foregroundStyle(Color.torroRed)
                    VStack(alignment: .leading, spacing: 3 * textScale) {
                        Text(restartTitle)
                            .scaledFont(.headline)
                        Text(restartExplanation)
                        .scaledFont(.subheadline)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        if descriptor.id == "clawbot" {
                            Text(verbatim: Self.openClawRestartCommand)
                                .scaledFont(.callout, design: .monospaced)
                                .textSelection(.enabled)
                                .scaledPadding(.top, 4)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .scaledPadding(.vertical, 4)
                .accessibilityElement(children: .combine)
            }
        }
    }

    private var restartTitle: String {
        if descriptor.id == "clawbot" {
            return L("Restart the OpenClaw Gateway now")
        }
        return String(format: L("Restart %@ now"), L(descriptor.displayName))
    }

    private var restartExplanation: String {
        if descriptor.id == "clawbot" {
            return L("Run this command in Terminal so OpenClaw loads the TorroMail MCP server:")
        }
        return String(
            format: L("%@ read its access key when it started and keeps using the old one. Until you quit and reopen it, every mail request it makes will fail."),
            L(descriptor.displayName)
        )
    }

    /// True while this client still carries a key TorroMail has replaced.
    private var mustRestart: Bool {
        MCPClientKeyStore.restartPending(
            keyChangedAt: keyChangedAt,
            lastConnected: model.clientConnections[descriptor.id]?.lastConnected
        )
    }

    /// The lead section for a manual client: it says plainly there is no
    /// one-click path, so the snippet below is the way in.
    private var manualHeaderSection: some View {
        Section {
            HStack(spacing: 12 * textScale) {
                MCPClientBadge(symbol: descriptor.symbol, dimmed: false)
                VStack(alignment: .leading, spacing: 1 * textScale) {
                    Text(L(descriptor.displayName)).scaledFont(.headline)
                    Text(L("Set up by hand"))
                        .scaledFont(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .scaledPadding(.vertical, 2)
        } header: {
            Text(L("Connection"))
        } footer: {
            Text(L("TorroMail does not configure this client automatically. Add the snippet below to its MCP configuration yourself."))
        }
    }

    private var connectionSection: some View {
        Section {
            HStack(spacing: 12 * textScale) {
                MCPClientBadge(
                    symbol: descriptor.symbol,
                    dimmed: hasCheckedStatus && !status.isInstalled
                )
                VStack(alignment: .leading, spacing: 1 * textScale) {
                    Text(L(descriptor.displayName)).scaledFont(.headline)
                    Text(stateText)
                        .scaledFont(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Circle().fill(dotColor).frame(width: 9 * textScale, height: 9 * textScale)
                    .help(stateText)
                    .accessibilityLabel(stateText)
            }
            .scaledPadding(.vertical, 2)

            if !hasCheckedStatus {
                HStack(spacing: 8 * textScale) {
                    ProgressView().controlSize(.small)
                    Text(L("Checking installation…"))
                        .scaledFont(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                }
            } else if status.isInstalled && status.setupAvailability == .ready {
                HStack {
                    if status.isConfigured {
                        // The destructive role gives it its colour; the style
                        // keeps it the same system button as its neighbour.
                        Button(L("Disconnect"), role: .destructive) { disconnect() }
                            .buttonStyle(.bordered)
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
            } else if !status.isInstalled {
                Text(String(format: L("%@ is not installed on this Mac. Install it, or set it up by hand with the snippet below."), descriptor.displayName))
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
            } else if status.setupAvailability == .setupToolMissing {
                Text(L("OpenClaw is installed, but its setup tool was not found."))
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text(L("This OpenClaw installation does not provide the native MCP setup commands."))
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
            }

            if descriptor.id == "clawbot", !showOpenClawLocation {
                Button(L("Use another OpenClaw installation…")) {
                    showOpenClawLocation = true
                }
                .torroButton()
            }

            if let note {
                Text(note)
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
            }
        } header: {
            Text(L("Connection"))
        }
    }

    private var openClawLocationSection: some View {
        Section {
            TextField(
                L("OpenClaw executable"),
                text: $model.generalSettings.openClaw.executablePath,
                prompt: Text("~/.openclaw/bin/openclaw")
            )
            TextField(
                "OPENCLAW_STATE_DIR",
                text: $model.generalSettings.openClaw.stateDirectory,
                prompt: Text("~/.openclaw")
            )
            TextField(
                "OPENCLAW_CONFIG_PATH",
                text: $model.generalSettings.openClaw.configPath,
                prompt: Text("~/.openclaw/openclaw.json")
            )
            HStack {
                if model.generalSettings.openClaw.hasOverrides {
                    Button(L("Use automatic locations")) {
                        model.generalSettings.openClaw = OpenClawSettings()
                        refresh(runServerTest: false)
                        loadSnippet()
                    }
                    .torroButton()
                }
                Spacer()
                Button(L("Check location")) {
                    refresh(runServerTest: false)
                    loadSnippet()
                }
                .torroButton()
            }
        } header: {
            Text(L("OpenClaw installation"))
        } footer: {
            Text(L("Leave fields empty for automatic detection. Custom state and config paths are used by OpenClaw’s setup commands and the safe configuration fallback."))
        }
    }

    @ViewBuilder
    private var testSection: some View {
        if hasCheckedStatus && status.isInstalled && status.setupAvailability == .ready {
            Section {
                // The headline: did this client actually reach the server? The
                // one fact a config file cannot give — read live from the log
                // the server writes on every handshake.
                connectionStatusRow

                // One level down, what TorroMail can prove on its own: the
                // server answers, and this client is set up to reach it.
                checkRow(
                    ok: status.isConfigured,
                    title: L("Written into %@’s configuration"),
                    pending: L("Not connected yet")
                )
                if status.isConfigured {
                    checkRow(
                        ok: status.hasCurrentKey,
                        title: L("%@ holds its access key"),
                        pending: L("Access key missing — reconnect to fix it")
                    )
                }
                serverCheckRow
                Button(L("Check again")) { refresh(runServerTest: true) }
                    .torroButton()
                    .disabled(isBusy)
            } header: {
                Text(L("Connection status"))
            } footer: {
                Text(L("The top line turns green when the client itself connects — the real proof, which shows after you restart it. The checks below are what TorroMail can verify on its own: its server answers, and this client is set up to reach it."))
            }
        }
    }

    /// The section's lead: the real connection, read live from `model`. Green
    /// once the client has handshaked; a plain hint until it does, because a
    /// configured client that never launched has never actually connected.
    @ViewBuilder
    private var connectionStatusRow: some View {
        if let connection = model.clientConnections[descriptor.id] {
            HStack(spacing: 8 * textScale) {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                VStack(alignment: .leading, spacing: 1 * textScale) {
                    Text(String(format: L("%@ has connected to the server"), descriptor.displayName))
                    Text(connectedDetail(connection))
                        .scaledFont(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
        } else {
            HStack(spacing: 8 * textScale) {
                Image(systemName: "bolt.horizontal.circle").foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 1 * textScale) {
                    Text(L("Has not connected yet"))
                    Text(String(format: L("Restart %@ to connect — it turns green here once it has."), descriptor.displayName))
                        .scaledFont(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
        }
    }

    /// "Last connected 3 minutes ago", with the client's reported version when
    /// it sent one — a small proof the handshake was real, not just configured.
    private func connectedDetail(_ connection: ClientConnection) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        let when = formatter.localizedString(for: connection.lastConnected, relativeTo: Date())
        let base = String(format: L("Last connected %@"), when)
        guard !connection.reportedVersion.isEmpty else { return base }
        return base + " · " + String(format: L("version %@"), connection.reportedVersion)
    }

    private var confirmSection: some View {
        Section {
            Text(L(descriptor.verificationHintKey))
                .scaledFont(.callout)
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
                        .scaledFont(.caption, design: .monospaced)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            if let realSnippet, let maskedSnippet {
                codeBlock(revealKey ? realSnippet : maskedSnippet)
                HStack {
                    Button(L("Copy config snippet")) { copySnippet() }
                        .torroButton()
                    Button {
                        revealKey.toggle()
                    } label: {
                        Label(
                            revealKey ? L("Hide key") : L("Reveal key"),
                            systemImage: revealKey ? "eye.slash" : "eye"
                        )
                    }
                    .torroButton()
                    // Automatic clients rotate their key by disconnecting
                    // and connecting; a manual client has no such buttons,
                    // so renewal lives here.
                    if descriptor.kind == .manual {
                        Button(L("Renew key")) { renewKey() }
                            .torroButton()
                    }
                    Spacer()
                    if snippetCopied {
                        Label(L("Copied to the clipboard."), systemImage: "checkmark")
                            .labelStyle(.titleAndIcon)
                            .scaledFont(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                if descriptor.kind == .manual, let note {
                    Text(note)
                        .scaledFont(.caption)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text(L("The snippet appears here as soon as TorroMail finds its server."))
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
            }
        } header: {
            Text(L("Manual setup"))
        } footer: {
            Text(L("For a client TorroMail does not configure automatically, paste this into its MCP configuration. The snippet carries this client’s personal access key — treat it like a password."))
        }
    }

    /// The snippet as a code card: monospaced, on a tinted ground, scrolling
    /// sideways so a long binary path never forces an ugly wrap.
    private func codeBlock(_ text: String) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Text(verbatim: text)
                .scaledFont(.caption, design: .monospaced)
                .textSelection(.enabled)
                .scaledPadding(12)
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
        HStack(spacing: 8 * textScale) {
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
            HStack(spacing: 8 * textScale) {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                Text(String(format: L("TorroMail server answers — %d tools"), count))
                Spacer()
            }
        case let .failed(message):
            HStack(spacing: 8 * textScale) {
                Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
                VStack(alignment: .leading, spacing: 1 * textScale) {
                    Text(L("TorroMail server did not answer"))
                    Text(message).scaledFont(.caption).foregroundStyle(.secondary)
                }
                Spacer()
            }
        }
    }

    // MARK: - State

    private var stateText: String {
        if !hasCheckedStatus { return L("Checking installation…") }
        if !status.isInstalled { return L("Not found on this Mac") }
        if status.setupAvailability == .setupToolMissing { return L("Setup tool not found") }
        if status.setupAvailability == .mcpUnavailable { return L("MCP setup unavailable") }
        if status.isConfigured && !status.hasCurrentKey { return L("Access key missing") }
        return status.isConfigured ? L("Connected") : L("Not connected")
    }

    private var dotColor: Color {
        if !hasCheckedStatus { return .orange }
        if !status.isInstalled { return .gray }
        if status.setupAvailability != .ready { return .orange }
        return status.hasCurrentKey ? .green : .orange
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
        if let client = MCPClientRegistry.installedClient(
            id: descriptor.id,
            openClawSettings: model.generalSettings.openClaw
        ) {
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
        let openClawSettings = model.generalSettings.openClaw
        // The watcher keeps this current within a couple of seconds; re-reading
        // here makes "Check again" — and returning to the screen — immediate.
        model.reloadClientConnections()
        if !runServerTest {
            status = MCPClientSetup.status(
                for: descriptor,
                executableName: executable,
                runServerTest: false,
                openClawSettings: openClawSettings
            )
            hasCheckedStatus = true
            model.connectedClients = MCPClientRegistry.catalog
                .filter {
                    MCPClientSetup.status(
                        for: $0,
                        executableName: executable,
                        runServerTest: false,
                        openClawSettings: openClawSettings
                    ).hasCurrentKey
                }
                .map(\.displayName)
            return
        }
        isBusy = true
        Task.detached(priority: .userInitiated) {
            let result = MCPClientSetup.status(
                for: descriptor,
                executableName: executable,
                runServerTest: runServerTest,
                openClawSettings: openClawSettings
            )
            await MainActor.run {
                status = result
                hasCheckedStatus = true
                if descriptor.id == "clawbot", result.setupAvailability != .ready {
                    showOpenClawLocation = true
                }
                model.reloadClientConnections()
                isBusy = false
            }
        }
    }

    private func connect() {
        note = nil
        guard let client = MCPClientRegistry.installedClient(
            id: descriptor.id,
            openClawSettings: model.generalSettings.openClaw
        ) else { return }
        guard let commandPath = MCPClientSetup.serverCommandPath(
            executableName: model.generalSettings.mcpExecutable
        ) else {
            note = L("The MCP server binary was not found.")
            return
        }
        do {
            // Key first, config second, allowlist last: the moment the
            // client restarts and presents the key, the document already
            // admits it.
            _ = try ClientAccountAccessStore.load(legacyClientIDs: MCPClientKeyStore.pairings().map(\.clientID))
            let previousKey = MCPClientKeyStore.token(forClient: descriptor.id)
            let token = try MCPClientKeyStore.tokenCreatingIfNeeded(forClient: descriptor.id)
            try MCPClientSetup.add(to: client, commandPath: commandPath, token: token)
            publishPolicyDocument(for: model.accounts)
            // A key that changed — freshly minted, or healed from an item this
            // build could not read — is one a running client has never seen,
            // and so is a client that never connected at all. Both have to
            // start again before anything works, which the notice up top says
            // in the one place nobody scrolls past.
            if previousKey != token || model.clientConnections[descriptor.id] == nil {
                keyChangedAt = Date()
            }
            if descriptor.id == "clawbot" {
                note = L("Connected. Run “openclaw gateway restart” to load the TorroMail MCP server.")
            } else {
                note = String(format: L("Connected. Restart %@ to load it."), descriptor.displayName)
            }
        } catch let failure as MCPClientSetup.Failure {
            note = L(failure.reason)
        } catch {
            note = L("Could not update the configuration.")
        }
        refresh(runServerTest: descriptor.id == "clawbot")
        loadSnippet()
    }

    private func disconnect() {
        note = nil
        guard let client = MCPClientRegistry.installedClient(
            id: descriptor.id,
            openClawSettings: model.generalSettings.openClaw
        ) else { return }
        do {
            try MCPClientSetup.remove(from: client)
            // Revoking, not just unlisting: any copy of the old config dies
            // with the key — the server re-reads the allowlist per call, so
            // this bites even mid-session.
            MCPClientKeyStore.revokeToken(forClient: descriptor.id)
            publishPolicyDocument(for: model.accounts)
            // Nothing to restart for: the server re-reads the allowlist per
            // call, so the revocation already bit. Asking for a restart here
            // would be the kind of instruction that teaches people to ignore
            // the one that matters.
            keyChangedAt = nil
            note = String(format: L("Removed from %@. Its access key no longer works."), descriptor.displayName)
        } catch let failure as MCPClientSetup.Failure {
            note = L(failure.reason)
        } catch {
            note = L("Could not update the configuration.")
        }
        refresh(runServerTest: false)
        loadSnippet()
    }

    /// A new key for a client whose config TorroMail cannot rewrite: the old
    /// one stops working the moment the allowlist is republished, and the
    /// snippet on screen switches to the replacement.
    private func renewKey() {
        note = nil
        do {
            _ = try MCPClientKeyStore.renewToken(forClient: descriptor.id)
            publishPolicyDocument(for: model.accounts)
            keyChangedAt = Date()
            revealKey = false
            snippetCopied = false
            loadSnippet()
            note = L("Key renewed. The old key no longer works — paste the new snippet into the client.")
        } catch {
            note = L("Could not renew the key.")
        }
    }

    /// Resolves the snippet once, so the code block shows exactly what the
    /// button copies — in two renderings: the key masked for the screen, in
    /// the clear for the clipboard. Opening this view mints the client's key
    /// if it never had one and publishes the pairing, so the snippet on
    /// screen is honored the moment it is pasted.
    private func loadSnippet() {
        guard let commandPath = MCPClientSetup.serverCommandPath(
            executableName: model.generalSettings.mcpExecutable
        ) else {
            realSnippet = nil
            maskedSnippet = nil
            return
        }
        do {
            // Migrate existing pairings before this view can mint a new key.
            _ = try ClientAccountAccessStore.load(legacyClientIDs: MCPClientKeyStore.pairings().map(\.clientID))
        } catch {
            note = L("Could not apply account sharing. Please try again.")
            realSnippet = nil
            maskedSnippet = nil
            return
        }
        let hadKey = MCPClientKeyStore.token(forClient: descriptor.id) != nil
        guard let token = try? MCPClientKeyStore.tokenCreatingIfNeeded(forClient: descriptor.id) else {
            realSnippet = nil
            maskedSnippet = nil
            return
        }
        if !hadKey {
            publishPolicyDocument(for: model.accounts)
        }
        realSnippet = MCPClientSetup.configSnippet(
            commandPath: commandPath,
            format: descriptor.snippetFormat,
            token: token
        )
        maskedSnippet = MCPClientSetup.configSnippet(
            commandPath: commandPath,
            format: descriptor.snippetFormat,
            token: MCPClientKeyStore.maskedToken(forClient: descriptor.id)
        )
    }

    private func copySnippet() {
        guard let realSnippet else { return }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(realSnippet, forType: .string)
        snippetCopied = true
    }
}
