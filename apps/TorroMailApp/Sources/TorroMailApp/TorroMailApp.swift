import CoreText
import SwiftUI
import TorroMailKit

/// Localized user-facing string. English keys are the development language;
/// translations live in `Resources/<lang>.lproj/Localizable.strings`.
func L(_ key: String) -> String {
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

func label(for preset: PermissionPreset) -> String {
    switch preset {
    case .readOnly: L("Read only")
    case .readAndDrafts: L("Read + drafts")
    case .tidyUp: L("Tidy up")
    case .fullAccess: L("Full access")
    }
}

/// A log entry's tool name in plain language. An unknown tool — an entry from
/// a newer server than this app — shows its raw name rather than nothing.
func auditEventLabel(_ tool: String) -> String {
    switch tool {
    case "mail_list_accounts": L("Listed accounts")
    case "mail_search": L("Searched mail")
    case "mail_refine_search": L("Refined a search")
    case "mail_get_message": L("Read a message")
    case "mail_get_thread": L("Read a conversation")
    case "mail_list_mailboxes": L("Listed mailboxes")
    case "mail_mark": L("Marked a message")
    case "mail_create_draft": L("Created a draft")
    case "mail_prepare_send": L("Prepared a send")
    case "mail_prepare_move": L("Prepared a move")
    case "mail_prepare_delete": L("Prepared a delete")
    case "mail_confirm_action": L("Confirmed an action")
    case "mail_get_policy": L("Checked permissions")
    case "mail_get_cache_status": L("Checked cache status")
    case "": ""
    default: tool
    }
}

/// A log entry's outcome in plain language: the server records only whether
/// the call produced a result or an error.
func auditResultLabel(_ result: String) -> String {
    switch result {
    case "ok": L("Completed")
    case "error": L("Refused")
    case "": ""
    default: result
    }
}

/// SF Symbol for a mailbox by its common IMAP name; a plain folder otherwise.
private func mailboxSymbol(_ name: String) -> String {
    switch name.lowercased() {
    case "inbox": "tray"
    case "archive": "archivebox"
    case "sent": "paperplane"
    case "drafts": "pencil"
    case "trash", "deleted items": "trash"
    case "junk", "spam": "xmark.bin"
    default: "folder"
    }
}

/// Brand colors from torro-design `tokens/tokens.json`.
extension Color {
    /// torro-red #D50C0C — the binding brand value.
    static let torroRed = Color(red: 213 / 255, green: 12 / 255, blue: 12 / 255)
    /// torro-red-deep #A50A0A — gradients, hover, depth.
    static let torroRedDeep = Color(red: 165 / 255, green: 10 / 255, blue: 10 / 255)
    /// silver #C4C3C3 — the wordmark's second half.
    static let torroSilver = Color(red: 196 / 255, green: 195 / 255, blue: 195 / 255)
}

extension View {
    /// The stock macOS push button. The brand red is a background and accent
    /// color — on a control it tints the label instead of the fill, which is
    /// unreadable, so buttons opt out of the app-wide tint entirely.
    func torroButton() -> some View {
        buttonStyle(.bordered).tint(nil)
    }
}

/// Frutiger LT 95 Ultra Black is the brand's display cut (torro-design
/// `font.display`). It ships inside the app rather than relying on the Mac
/// having it installed; registering it for this process keeps it out of the
/// user's font book.
private func registerBrandFont() {
    guard let url = Bundle.module.url(forResource: "FrutigerLT-UltraBlack", withExtension: "ttf") else {
        return
    }
    CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
}

extension Font {
    /// The display cut, by PostScript name. Falls back to a heavy system face
    /// if registration ever fails.
    static func torroDisplay(size: CGFloat) -> Font {
        .custom("FrutigerLT-UltraBlack", size: size)
    }
}

/// The Torro horns signet, traced from `Icon/AppIcon.svg` so the wordmark and
/// the app icon stay the same artwork.
private struct TorroSignet: Shape {
    /// Bounding box of the traced artwork in its own coordinate space.
    private static let artwork = CGRect(x: 12.3, y: 38.7, width: 123.7, height: 69.9)

    func path(in rect: CGRect) -> Path {
        let scale = min(rect.width / Self.artwork.width, rect.height / Self.artwork.height)
        let drawn = CGSize(width: Self.artwork.width * scale, height: Self.artwork.height * scale)
        let origin = CGPoint(x: rect.midX - drawn.width / 2, y: rect.midY - drawn.height / 2)

        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
            CGPoint(
                x: origin.x + (x - Self.artwork.minX) * scale,
                y: origin.y + (y - Self.artwork.minY) * scale
            )
        }

        var path = Path()
        path.move(to: p(54.0000, 108.4000))
        path.addCurve(to: p(31.6000, 103.3000), control1: p(44.9000, 106.9000), control2: p(38.5000, 105.5000))
        path.addCurve(to: p(12.3000, 78.6000), control1: p(18.0000, 99.0000), control2: p(11.1000, 90.2000))
        path.addCurve(to: p(25.2000, 56.5000), control1: p(13.1000, 70.8000), control2: p(16.8000, 64.4000))
        path.addCurve(to: p(50.1000, 38.7000), control1: p(33.3000, 48.7000), control2: p(49.0000, 37.6000))
        path.addCurve(to: p(48.9000, 50.0000), control1: p(50.4000, 39.2000), control2: p(49.9000, 44.2000))
        path.addCurve(to: p(47.8000, 68.6000), control1: p(46.9000, 61.3000), control2: p(46.6000, 66.8000))
        path.addCurve(to: p(61.3000, 72.8000), control1: p(49.3000, 70.9000), control2: p(53.4000, 72.2000))
        path.addLine(to: p(69.5000, 73.5000))
        path.addLine(to: p(69.5000, 91.0000))
        path.addLine(to: p(69.5000, 108.5000))
        path.addLine(to: p(63.0000, 108.6000))
        path.addCurve(to: p(54.0000, 108.4000), control1: p(59.4000, 108.7000), control2: p(55.4000, 108.6000))
        path.closeSubpath()
        path.move(to: p(79.5000, 107.9000))
        path.addCurve(to: p(79.2000, 90.0000), control1: p(79.2000, 107.1000), control2: p(79.1000, 99.0000))
        path.addLine(to: p(79.5000, 73.5000))
        path.addLine(to: p(87.7000, 72.8000))
        path.addCurve(to: p(101.2000, 68.6000), control1: p(95.6000, 72.2000), control2: p(99.7000, 70.9000))
        path.addCurve(to: p(100.1000, 50.0000), control1: p(102.4000, 66.8000), control2: p(102.1000, 61.3000))
        path.addCurve(to: p(98.9000, 38.7000), control1: p(99.1000, 44.2000), control2: p(98.6000, 39.2000))
        path.addCurve(to: p(123.8000, 56.5000), control1: p(100.0000, 37.6000), control2: p(115.7000, 48.7000))
        path.addCurve(to: p(133.7000, 68.4000), control1: p(128.0000, 60.4000), control2: p(132.3000, 65.6000))
        path.addCurve(to: p(136.0000, 87.5000), control1: p(136.7000, 74.4000), control2: p(137.7000, 82.4000))
        path.addCurve(to: p(121.2000, 102.0000), control1: p(134.1000, 93.4000), control2: p(127.7000, 99.6000))
        path.addCurve(to: p(79.5000, 107.9000), control1: p(106.9000, 107.4000), control2: p(80.7000, 111.0000))
        path.closeSubpath()
        return path
    }
}

/// A single outward-flaring horn, traced from the TORROFORMS wordmark in
/// torro-design (`logo/wordmark/torroforms-wordmark.svg`). Mirror it for the
/// right-hand side.
private struct TorroHorn: Shape {
    private static let artwork = CGRect(x: 46.429688, y: 59.398438, width: 55.765624, height: 68.832031)

    func path(in rect: CGRect) -> Path {
        let scaleX = rect.width / Self.artwork.width
        let scaleY = rect.height / Self.artwork.height

        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
            CGPoint(
                x: rect.minX + (x - Self.artwork.minX) * scaleX,
                y: rect.minY + (y - Self.artwork.minY) * scaleY
            )
        }

        var path = Path()
        path.move(to: p(102.195312, 128.230469))
        path.addCurve(to: p(46.429688, 100.71875), control1: p(92.625, 128.230469), control2: p(45.835938, 128.230469))
        path.addCurve(to: p(82.910156, 59.398438), control1: p(47.664062, 76.40625), control2: p(82.910156, 59.398438))
        path.addCurve(to: p(79.394531, 88.992188), control1: p(82.910156, 59.398438), control2: p(77.566406, 85.226562))
        path.addCurve(to: p(102.195312, 93.8125), control1: p(80.632812, 91.546875), control2: p(83.589844, 94.261719))
        path.closeSubpath()
        return path
    }
}

/// The TorroMail lockup, built like the TORROFORMS wordmark it descends from:
/// horns flaring out either side of the name, set in the display cut, the
/// brand half white and the product half silver, all on the red ground.
private struct TorroWordmark: View {
    /// Height of the capitals. Everything else is derived from it, the same
    /// proportions the original wordmark uses.
    var capHeight: CGFloat = 10

    private var hornHeight: CGFloat { capHeight * 2.0 }
    private var hornWidth: CGFloat { hornHeight * 55.765624 / 68.832031 }

    var body: some View {
        HStack(alignment: .lastTextBaseline, spacing: capHeight * 0.14) {
            horn(mirrored: false)
            Text(verbatim: "TORRO").foregroundStyle(.white)
                + Text(verbatim: "MAIL").foregroundStyle(Color.torroSilver)
            horn(mirrored: true)
        }
        // Frutiger's capitals sit at ~0.7 em.
        .font(.torroDisplay(size: capHeight / 0.7))
        .lineLimit(1)
        .fixedSize()
        .accessibilityElement()
        .accessibilityLabel("TorroMail")
    }

    private func horn(mirrored: Bool) -> some View {
        TorroHorn()
            .fill(.white)
            .frame(width: hornWidth, height: hornHeight)
            .scaleEffect(x: mirrored ? -1 : 1)
            // The horns end on the baseline, exactly as the letters do.
            .alignmentGuide(.lastTextBaseline) { $0[.bottom] }
    }
}

/// The brand footer at the foot of the sidebar. No ground of its own — the
/// mark sits quietly on the sidebar material and leaves the red to the
/// dashboard.
private struct SidebarBrandFooter: View {
    var body: some View {
        TorroWordmark(capHeight: 11)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 14)
    }
}

/// The raised surface every panel in this app sits on: a lit top edge that
/// fades downwards, over a soft ambient shadow. The shadow hangs on the
/// surface alone — putting it on the whole panel makes every label inside
/// cast one too, which just smears the text.
private struct TorroCard: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme
    var cornerRadius: CGFloat = 12
    var isHighlighted = false

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        return content
            .background {
                shape
                    .fill(.background)
                    .shadow(color: .black.opacity(colorScheme == .dark ? 0.5 : 0.14), radius: 5, y: 3)
                    .shadow(color: .black.opacity(colorScheme == .dark ? 0.3 : 0.05), radius: 1, y: 1)
            }
            .overlay {
                shape.strokeBorder(edgeGradient, lineWidth: 1)
            }
            .contentShape(shape)
    }

    private var edgeGradient: LinearGradient {
        let top: Color
        let bottom: Color
        if colorScheme == .dark {
            top = .white.opacity(isHighlighted ? 0.34 : 0.20)
            bottom = .white.opacity(isHighlighted ? 0.10 : 0.05)
        } else {
            top = .white.opacity(0.9)
            bottom = .black.opacity(isHighlighted ? 0.18 : 0.10)
        }
        return LinearGradient(colors: [top, bottom], startPoint: .top, endPoint: .bottom)
    }
}

extension View {
    func torroCard(cornerRadius: CGFloat = 12, isHighlighted: Bool = false) -> some View {
        modifier(TorroCard(cornerRadius: cornerRadius, isHighlighted: isHighlighted))
    }
}

/// Owns where TorroMail shows up. Both the Dock tile and the menu bar item are
/// the user's decision, so with both off the window is the only place the app
/// appears — which is why closing it must not quit the process.
@MainActor
final class TorroMailPresence: NSObject, NSApplicationDelegate, ObservableObject {
    /// Mirrors `GeneralSettings.showDockIcon`; the scene keeps it in sync.
    var showDockIcon = false {
        didSet { apply() }
    }

    /// SwiftUI owns window creation, so the root view hands its `openWindow`
    /// action over for the reopen path.
    var openMainWindow: (() -> Void)?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // The window's own lifecycle is the signal: SwiftUI's `onDisappear`
        // does not fire for a WindowGroup window that the user closes.
        let center = NotificationCenter.default
        for name in [NSWindow.willCloseNotification, NSWindow.didBecomeKeyNotification] {
            center.addObserver(self, selector: #selector(windowsChanged), name: name, object: nil)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool {
        false
    }

    /// Clicking the Dock tile, or launching TorroMail while it already runs in
    /// the background, brings the window back.
    func applicationShouldHandleReopen(_ app: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !hasVisibleWindows {
            showMainWindow()
        }
        return true
    }

    func showMainWindow() {
        openMainWindow?()
        NSApp.activate(ignoringOtherApps: true)
    }

    @objc private func windowsChanged() {
        // A closing window is still listed while `willClose` is in flight, so
        // let AppKit settle and then look at what is actually left.
        DispatchQueue.main.async { [weak self] in
            self?.apply()
        }
    }

    /// Panels, the status item and other AppKit scaffolding all show up in
    /// `NSApp.windows`; only a window the user can bring to the front counts.
    private var hasOpenWindow: Bool {
        NSApp.windows.contains { $0.isVisible && $0.canBecomeMain }
    }

    private func apply() {
        let presence = AppPresence.resolve(showDockIcon: showDockIcon, hasOpenWindow: hasOpenWindow)
        let policy: NSApplication.ActivationPolicy = presence == .foreground ? .regular : .accessory
        guard NSApp.activationPolicy() != policy else { return }
        NSApp.setActivationPolicy(policy)
        // Coming back from `.accessory` the app is not frontmost by itself, so
        // the window would open behind whatever the user is looking at.
        if policy == .regular {
            NSApp.activate(ignoringOtherApps: true)
        }
    }
}

/// Failures are diagnostics, not decisions — they belong in the log.
/// Internal, not private: connect/disconnect in the MCP views changes the
/// pairing list and republishes from there.
func publishPolicyDocument(for accounts: [MailAccount]) {
    do {
        // The app is a paired client of its own server (connection checks,
        // the supervised instance) — minting here keeps it on every
        // allowlist this document will ever carry.
        _ = try MCPClientKeyStore.appToken()
        try PolicyDocument.publish(accounts: accounts, clients: MCPClientKeyStore.pairings())
    } catch {
        NSLog("TorroMail: policy document write failed: %@", error.localizedDescription)
    }
}

private func persistState(accounts: [MailAccount], settings: GeneralSettings) {
    do {
        try AppStateStore.save(AppStateStore.State(accounts: accounts, settings: settings))
    } catch {
        NSLog("TorroMail: state write failed: %@", error.localizedDescription)
    }
}

@main
struct TorroMailApp: App {
    static let mainWindowID = "main"

    @NSApplicationDelegateAdaptor(TorroMailPresence.self) private var presence
    @StateObject private var model = TorroMailModel.stored()
    @StateObject private var mcpSupervisor = MCPServerSupervisor()
    @State private var auditWatcher = AuditWatcher()

    init() {
        // Before any keychain access: the app never shows the consent dialog.
        // It reads its own items by Team ID; anything it cannot read that way
        // is skipped or re-minted rather than prompting the user.
        KeychainStore.silenceInteractivePrompts()
        registerBrandFont()
    }

    var body: some Scene {
        WindowGroup(id: Self.mainWindowID) {
            TorroMailRootView()
                .environmentObject(model)
                .environmentObject(mcpSupervisor)
                .environmentObject(presence)
                .frame(minWidth: 1080, minHeight: 660)
                .tint(.torroRed)
                .task {
                    mcpSupervisor.start(executableName: model.generalSettings.mcpExecutable)
                    let executable = model.generalSettings.mcpExecutable
                    // Configs written before access keys existed get theirs
                    // now. Off the main actor — the CLI-owned ones are
                    // rewritten by their own tools, and that is a process
                    // launch. Republishing puts the healed pairings on the
                    // allowlist.
                    let healed = await Task.detached(priority: .utility) {
                        MCPClientSetup.refreshManagedKeys(executableName: executable)
                    }.value
                    if healed {
                        publishPolicyDocument(for: model.accounts)
                    }
                    // The MCP server appends to the log as assistants work;
                    // watching it keeps the activity card and log live rather
                    // than frozen at whatever launch read.
                    auditWatcher.start {
                        Task { @MainActor in model.reloadAudit() }
                    }
                }
                .onChange(of: model.generalSettings.showDockIcon, initial: true) { _, show in
                    presence.showDockIcon = show
                }
                // Every permission switch lands in the policy document the
                // MCP server enforces — flipped in the UI, live on the wire —
                // and in the state file, so it is still there next launch.
                .onChange(of: model.accounts, initial: true) { _, accounts in
                    publishPolicyDocument(for: accounts)
                    persistState(accounts: accounts, settings: model.generalSettings)
                    // A renamed or removed account changes how the log reads —
                    // re-map its entries against the current names.
                    model.reloadAudit()
                }
                .onChange(of: model.generalSettings) { _, settings in
                    persistState(accounts: model.accounts, settings: settings)
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

        MenuBarExtra(
            L("TorroMail"),
            systemImage: "envelope.fill",
            isInserted: menuBarInserted
        ) {
            MenuBarContent()
                .environmentObject(model)
                .environmentObject(presence)
        }
    }

    /// `MenuBarExtra` writes `isInserted` back on every scene update, including
    /// when the value has not changed. Writing into `generalSettings` — a
    /// struct behind `@Published` — republishes regardless, which re-runs this
    /// scene body, which writes again: the app spins at 100% CPU and never
    /// takes a click. Swallowing the no-op writes breaks the cycle.
    private var menuBarInserted: Binding<Bool> {
        Binding(
            get: { model.generalSettings.showMenuBarIcon },
            set: { show in
                guard show != model.generalSettings.showMenuBarIcon else { return }
                model.generalSettings.showMenuBarIcon = show
            }
        )
    }
}

/// The menu bar item stays a doorway, not a second control surface: open the
/// window, or stop the app.
private struct MenuBarContent: View {
    @EnvironmentObject private var model: TorroMailModel
    @EnvironmentObject private var presence: TorroMailPresence

    var body: some View {
        Button(L("Open TorroMail")) {
            presence.showMainWindow()
        }
        if model.pendingActionCount > 0 {
            Button(pendingTitle) {
                model.selectedSidebarItem = .dashboard
                presence.showMainWindow()
            }
        }
        Divider()
        Button(L("Quit TorroMail")) {
            NSApp.terminate(nil)
        }
    }

    private var pendingTitle: String {
        String(format: L("%d waiting for you"), model.pendingActionCount)
    }
}

private struct TorroMailRootView: View {
    @EnvironmentObject private var model: TorroMailModel
    @EnvironmentObject private var presence: TorroMailPresence
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        NavigationSplitView {
            List(selection: sidebarSelection) {
                Label(L("Overview"), systemImage: "square.grid.2x2")
                    .tag(TorroMailSidebarSelection.dashboard)
                // `.badge` has to come before `.tag`: it wraps the row, and a
                // tag applied to the wrapper does not reach the row, leaving
                // the item unselectable.
                Label(L("Mail Accounts"), systemImage: "envelope")
                    .badge(model.pendingActionCount)
                    .tag(TorroMailSidebarSelection.accounts)
                Label(L("MCP Clients"), systemImage: "link")
                    .tag(TorroMailSidebarSelection.mcp)
                Label(L("Settings"), systemImage: "gearshape")
                    .tag(TorroMailSidebarSelection.settings)
                Label(L("Log"), systemImage: "list.bullet.rectangle")
                    .tag(TorroMailSidebarSelection.log)
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 200, ideal: 220, max: 280)
            .navigationTitle("TorroMail")
            .safeAreaInset(edge: .bottom) {
                SidebarBrandFooter()
            }
        } detail: {
            detailView
        }
        .sheet(isPresented: $model.showAccountWizard) {
            AccountSetupWizard()
                .environmentObject(model)
        }
        // The Dock tile and the menu bar item both need to reopen the window,
        // and only a view can reach SwiftUI's window actions.
        .onAppear {
            presence.openMainWindow = { openWindow(id: TorroMailApp.mainWindowID) }
        }
    }

    @ViewBuilder
    private var detailView: some View {
        switch model.selectedSidebarItem {
        case .dashboard:
            DashboardView()
        case .accounts:
            NavigationStack(path: $model.accountPath) {
                AccountListView()
                    .navigationDestination(for: String.self) { accountID in
                        accountDetail(for: accountID)
                    }
            }
        case .mcp:
            NavigationStack(path: $model.mcpPath) {
                MCPServerListView()
                    .navigationDestination(for: String.self) { clientID in
                        mcpClientDetail(for: clientID)
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

    @ViewBuilder
    private func mcpClientDetail(for clientID: String) -> some View {
        if let descriptor = MCPClientRegistry.descriptor(id: clientID) {
            MCPClientDetailView(descriptor: descriptor)
                .id(clientID)
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

/// The landing view. It answers, in order: is TorroMail doing its job, does
/// anything need me, what happened lately — and, while nothing is set up yet,
/// what this app is for.
private struct DashboardView: View {
    @EnvironmentObject private var model: TorroMailModel
    @EnvironmentObject private var mcpSupervisor: MCPServerSupervisor

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                BrandHero()
                if model.health == .notConfigured {
                    GettingStartedCard()
                } else {
                    ServiceStatusCard()
                    if !model.pendingActions.isEmpty {
                        PendingApprovalsCard()
                    }
                    AccountsOverviewCard()
                    RecentActivityCard()
                }
                NewsCard()
            }
            .padding(20)
            .frame(maxWidth: 720, alignment: .top)
            .frame(maxWidth: .infinity)
        }
        .background(.background.secondary)
        .navigationTitle(L("Overview"))
    }
}

/// The one place the brand gets to be loud: red ground, the wordmark, and in
/// one line what this app actually does.
private struct BrandHero: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            TorroWordmark(capHeight: 15)
            Text(L("Your mailboxes for AI assistants — nothing leaves without your say-so."))
                .font(.callout)
                .foregroundStyle(.white.opacity(0.92))
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 18)
        .padding(.vertical, 15)
        .background {
            let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)
            LinearGradient(
                colors: [.torroRed, .torroRedDeep],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            // The signet as a watermark, the use torro-design lists for it. It
            // runs off the right edge on purpose — cropped by the card rather
            // than floating as a lone shape. It has to be an overlay: as a
            // sibling in a stack its fixed 150pt would set the height, and the
            // red would spill past the hero onto whatever sits below.
            .overlay(alignment: .trailing) {
                TorroSignet()
                    .fill(.white.opacity(0.09))
                    .frame(width: 260, height: 150)
                    .offset(x: 95)
            }
            .clipShape(shape)
            .overlay {
                shape.strokeBorder(.white.opacity(0.18), lineWidth: 1)
            }
            .shadow(color: .black.opacity(0.28), radius: 6, y: 3)
        }
    }
}

/// Is the app doing its job right now? Phrased as the user's question, not as
/// process state — the mechanics belong in the log.
private struct ServiceStatusCard: View {
    @EnvironmentObject private var model: TorroMailModel
    @EnvironmentObject private var mcpSupervisor: MCPServerSupervisor

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(L("Status"))
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.secondary)
            // Two independent services, so two boxes side by side rather than a
            // stacked list: each tile is one service and its own state, and the
            // pair costs a fraction of the height the list did.
            HStack(alignment: .top, spacing: 10) {
                StatusTile(
                    color: mcpSupervisor.status.isRunning ? .green : .red,
                    title: mcpSupervisor.status.isRunning
                        ? L("Ready for assistants")
                        : L("Not reachable for assistants"),
                    detail: mcpSupervisor.status.isRunning
                        ? L("TorroMail is running in the background.")
                        : L("Start TorroMail in the Mail Accounts section so assistants can reach your mail.")
                )
                StatusTile(
                    color: model.connectedClients.isEmpty ? .orange : .green,
                    title: model.connectedClients.isEmpty
                        ? L("No assistant connected")
                        : clientTitle,
                    detail: model.connectedClients.isEmpty
                        ? L("Connect one in the MCP Clients section to start using your mail.")
                        : L("Every access is logged.")
                )
            }
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var clientTitle: String {
        String(format: L("%@ connected"), model.connectedClients.formatted(.list(type: .and)))
    }
}

private struct StatusTile: View {
    var color: Color
    var title: String
    var detail: String

    var body: some View {
        // The dot gets its own column so title and detail share one left edge —
        // as siblings under the dot, the detail would hang out to its left.
        HStack(alignment: .top, spacing: 7) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
                .padding(.top, 5)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.headline)
                    .fixedSize(horizontal: false, vertical: true)
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .torroCard()
    }
}

/// The only card that asks for a decision, so it leads with the buttons and
/// says plainly what it is about to do.
private struct PendingApprovalsCard: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        DashboardCard(
            title: L("Waiting for you"),
            footer: L("An assistant prepared these. Nothing happens until you approve.")
        ) {
            VStack(spacing: 0) {
                ForEach(Array(model.pendingActions.enumerated()), id: \.element.id) { index, action in
                    if index > 0 { Divider() }
                    HStack(spacing: 12) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(action.subject).font(.headline).lineLimit(1)
                            Text(sentence(for: action))
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        Spacer(minLength: 8)
                        Text(action.expiresIn)
                            .font(.caption)
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                        Button(L("Reject"), role: .destructive) {}
                        Button(L("Approve")) {}
                            .torroButton()
                    }
                    .padding(.vertical, 10)
                }
            }
        }
    }

    private func sentence(for action: PendingAction) -> String {
        let account = model.accountName(id: action.accountID) ?? action.accountID
        return String(format: L("Send to %@ · from %@"), action.recipient, account)
    }
}

/// The accounts at a glance. Clicking through lands on the same detail view
/// the account list leads to.
private struct AccountsOverviewCard: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        DashboardCard(title: L("Mail Accounts")) {
            VStack(spacing: 0) {
                ForEach(Array(model.accounts.enumerated()), id: \.element.id) { index, account in
                    if index > 0 { Divider().padding(.leading, 38) }
                    Button {
                        model.openAccount(id: account.id)
                    } label: {
                        HStack(spacing: 10) {
                            MailBadge(size: 26)
                            Text(account.name).font(.body)
                            Spacer(minLength: 8)
                            Text(account.email)
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            CredentialStatusDot(state: account.connectionState)
                            Image(systemName: "chevron.right")
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(.tertiary)
                        }
                        .contentShape(.rect)
                        .padding(.vertical, 8)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }
}

/// A glance at what assistants have been doing, with the full story one click
/// away in the log.
private struct RecentActivityCard: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        DashboardCard(title: L("Recent Activity")) {
            if model.audit.isEmpty {
                Text(L("No activity yet. It appears here the moment an assistant does something."))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 7)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(model.audit.suffix(4).reversed().enumerated()), id: \.element.id) { index, entry in
                        if index > 0 { Divider() }
                        HStack(spacing: 10) {
                            Text(entry.time)
                                .font(.caption)
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                            Text(entry.client)
                            Text(auditEventLabel(entry.event))
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .layoutPriority(1)
                            if !entry.detail.isEmpty {
                                Text(entry.detail)
                                    .font(.caption)
                                    .foregroundStyle(.tertiary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                            Spacer(minLength: 8)
                            Text(auditResultLabel(entry.result))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        .padding(.vertical, 7)
                    }
                }
            }
        } accessory: {
            Button(L("Open Log")) {
                model.selectedSidebarItem = .log
            }
            .buttonStyle(.plain)
            .font(.subheadline)
            .foregroundStyle(Color.torroRed)
        }
    }
}

/// Shown instead of the status cards while there is nothing to show a status
/// for. Explains the app and offers the single next step.
private struct GettingStartedCard: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        DashboardCard(title: L("Getting started")) {
            VStack(alignment: .leading, spacing: 14) {
                StepRow(
                    number: 1,
                    title: L("Add a mail account"),
                    detail: L("TorroMail talks to your mail server. It never becomes your mail client.")
                )
                StepRow(
                    number: 2,
                    title: L("Decide what is allowed"),
                    detail: L("Per account you pick what assistants may read and do.")
                )
                StepRow(
                    number: 3,
                    title: L("Connect an assistant"),
                    detail: L("Sending, moving and deleting always come back to you for approval.")
                )
                Button(L("Add Account")) {
                    model.beginAccountWizard()
                }
                .torroButton()
            }
        }
    }
}

private struct StepRow: View {
    var number: Int
    var title: String
    var detail: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Text(number.formatted())
                .font(.caption.bold())
                .monospacedDigit()
                .foregroundStyle(.white)
                .frame(width: 18, height: 18)
                .background(Color.torroRed, in: Circle())
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.headline)
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
    }
}

private struct NewsCard: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        if !model.news.isEmpty {
            DashboardCard(title: L("News from Torro")) {
                VStack(spacing: 0) {
                    ForEach(Array(model.news.enumerated()), id: \.element.id) { index, item in
                        if index > 0 { Divider().padding(.leading, 34) }
                        HStack(alignment: .top, spacing: 10) {
                            Image(systemName: item.symbol)
                                .font(.system(size: 13))
                                .foregroundStyle(Color.torroRed)
                                .frame(width: 24, height: 24)
                                .background(Color.torroRed.opacity(0.12), in: Circle())
                            VStack(alignment: .leading, spacing: 1) {
                                HStack(spacing: 6) {
                                    Text(item.title).font(.headline)
                                    if item.isNew {
                                        Text(L("New"))
                                            .font(.caption2.bold())
                                            .padding(.horizontal, 5)
                                            .padding(.vertical, 1)
                                            .background(Color.torroRed, in: Capsule())
                                            .foregroundStyle(.white)
                                    }
                                }
                                Text(item.detail)
                                    .font(.subheadline)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.vertical, 9)
                    }
                }
            }
        }
    }
}

/// A titled panel on the shared card surface.
private struct DashboardCard<Content: View, Accessory: View>: View {
    var title: String
    var footer: String?
    @ViewBuilder var content: Content
    @ViewBuilder var accessory: Accessory

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                Spacer()
                accessory
            }
            VStack(alignment: .leading, spacing: 0) {
                content
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 4)
            .torroCard()

            if let footer {
                Text(footer)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 2)
            }
        }
    }
}

extension DashboardCard where Accessory == EmptyView {
    init(title: String, footer: String? = nil, @ViewBuilder content: () -> Content) {
        self.init(title: title, footer: footer, content: content, accessory: { EmptyView() })
    }
}

private struct AccountListView: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 10) {
                MCPServerStatusCard()
                    .padding(.bottom, 6)
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
    @State private var isHovering = false

    var body: some View {
        HStack(spacing: 12) {
            MailBadge()

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
        .torroCard(isHighlighted: isHovering)
        .onHover { isHovering = $0 }
        .animation(.easeOut(duration: 0.12), value: isHovering)
    }
}

/// The red envelope tile that stands for a mail account.
private struct MailBadge: View {
    var size: CGFloat = 34

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
            .fill(Color.torroRed.gradient)
            .frame(width: size, height: size)
            .overlay {
                Image(systemName: "envelope.fill")
                    .font(.system(size: size * 0.41, weight: .semibold))
                    .foregroundStyle(.white)
            }
            .shadow(color: .black.opacity(0.2), radius: 1.5, y: 1)
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
    // Held only for the moment of saving; the keychain is the home.
    @State private var password = ""
    @State private var isCheckingConnection = false
    @State private var confirmRemoval = false
    // Remembered so switching a group off and on again restores the last
    // sub-selection instead of resetting the user's choice.
    @State private var lastRead: ReadAccess = .fullMessage
    @State private var lastWrite = WriteAccess(drafts: true, mark: true)

    var body: some View {
        Form {
            identitySection
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

    /// Who this account is: the name assistants send mail as, and the
    /// address it comes from. The wizard asks once — this is where it stays
    /// changeable.
    private var identitySection: some View {
        Section {
            TextField(L("Sender Name"), text: $account.name, prompt: Text(verbatim: "Sven Wagener"))
            TextField(L("Email"), text: $account.email, prompt: Text(verbatim: "name@example.com"))
        } header: {
            Text(L("Identity"))
        } footer: {
            Text(L("Shown in TorroMail, and used as the sender of anything you approve."))
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
                        .torroButton()
                }
                Button(L("Test Connection")) {
                    testConnection()
                }
                .torroButton()
                .disabled(isCheckingConnection)
            }
        }
    }

    /// Saves the password to the keychain and runs the real check: the MCP
    /// binary resolves the secret, connects over TLS and logs in — the dot
    /// reports what actually happened.
    private func testConnection() {
        if account.loginMethod == .password, !password.isEmpty {
            do {
                try KeychainStore.savePassword(password, forAccount: account.id)
                password = ""
            } catch {
                account.connectionState = .failed(L("Could not save the password to the keychain."))
                return
            }
        }

        isCheckingConnection = true
        let accountID = account.id
        let executable = model.generalSettings.mcpExecutable
        Task.detached(priority: .userInitiated) {
            let state = AccountCheck.run(accountID: accountID, executableName: executable)
            await MainActor.run {
                account.connectionState = state
                isCheckingConnection = false
            }
        }
    }

    private var permissionsSection: some View {
        Section {
            groupRow(icon: "eye", title: L("Read"), subtitle: readSummary) {
                if account.permissions.read != .none {
                    Picker(L("Read depth"), selection: $account.permissions.read) {
                        Text(L("Subject and sender")).tag(ReadAccess.headers)
                        Text(L("Full message")).tag(ReadAccess.fullMessage)
                        Text(L("Message and attachments")).tag(ReadAccess.withAttachments)
                    }
                    .labelsHidden()
                    .fixedSize()
                }
                Toggle(L("Read"), isOn: readEnabled)
                    .labelsHidden()
            }

            groupRow(icon: "pencil", title: L("Write"), subtitle: writeSummary) {
                Toggle(L("Write"), isOn: writeEnabled)
                    .labelsHidden()
            }
            if !account.permissions.write.isEmpty {
                subToggle(L("Create drafts"), isOn: $account.permissions.write.drafts)
                subToggle(L("Mark"), annotation: L("read state, flags"), isOn: $account.permissions.write.mark)
                subToggle(L("Move"), isOn: $account.permissions.write.move)
                subToggle(L("Delete"), annotation: L("to the Trash"), isOn: trashEnabled)
                subToggle(
                    L("Permanent delete"),
                    annotation: L("never preset, manual only"),
                    isOn: $account.permissions.write.permanentDelete
                )
                .disabled(!account.permissions.write.trash)
            }

            groupRow(
                icon: "paperplane",
                title: L("Send"),
                subtitle: L("Applies to the whole account. Every send waits for your approval in TorroMail.")
            ) {
                Toggle(L("Send"), isOn: $account.permissions.send)
                    .labelsHidden()
            }
        } header: {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(L("Permissions"))
                    Text(L("What connected assistants may do with this account."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                PresetChips(permissions: $account.permissions)
            }
        }
    }

    /// One rights group: icon, name, a summary line that reads as the current
    /// decision, and the switch (plus inline detail control) trailing.
    private func groupRow(
        icon: String,
        title: String,
        subtitle: String,
        @ViewBuilder trailing: () -> some View
    ) -> some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 15))
                .foregroundStyle(.secondary)
                .frame(width: 20)
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
            trailing()
        }
        .padding(.vertical, 2)
    }

    /// A sub-right beneath its group, indented to the group's text column,
    /// with an optional muted annotation after the name.
    private func subToggle(_ title: String, annotation: String? = nil, isOn: Binding<Bool>) -> some View {
        Toggle(isOn: isOn) {
            HStack(spacing: 4) {
                Text(title)
                if let annotation {
                    Text(verbatim: "– \(annotation)")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(.leading, 32)
    }

    private var readSummary: String {
        switch account.permissions.read {
        case .none: L("No read access")
        case .headers: L("Subject and sender")
        case .fullMessage: L("Full message")
        case .withAttachments: L("Message and attachments")
        }
    }

    private var writeSummary: String {
        let names = [
            account.permissions.write.drafts ? L("Drafts") : nil,
            account.permissions.write.mark ? L("Mark") : nil,
            account.permissions.write.move ? L("Move") : nil,
            account.permissions.write.trash ? L("Delete") : nil
        ].compactMap { $0 }
        return names.isEmpty ? L("No mailbox changes") : names.joined(separator: ", ")
    }

    private var foldersSection: some View {
        Section {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(L("Per-folder permissions"))
                    Text(L("Off: all folders use the permissions above."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Toggle(L("Per-folder permissions"), isOn: $account.permissions.perFolder)
                    .labelsHidden()
            }
            .padding(.vertical, 2)
            if account.permissions.perFolder {
                folderColumnHeader
                standardFolderRow
                ForEach(account.knownMailboxes, id: \.self) { name in
                    folderRow(name)
                }
            }
        } header: {
            Text(L("Folders"))
        } footer: {
            if account.permissions.perFolder {
                Text(L("New folders follow the account automatically. Write access to Drafts, Sent, and Trash follows from the decisions above."))
            }
        }
    }

    private var folderColumnHeader: some View {
        HStack {
            Text(L("Folder"))
            Spacer()
            Text(L("Read")).frame(width: 64)
            Text(L("Write")).frame(width: 64)
            Color.clear.frame(width: 74, height: 1)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    /// The row every folder inherits from — it mirrors the groups above, so
    /// it is display-only here.
    private var standardFolderRow: some View {
        HStack {
            Label {
                Text(L("Standard – all folders")).fontWeight(.medium)
            } icon: {
                Image(systemName: "asterisk")
            }
            Spacer()
            standardMark(account.permissions.read != .none)
            standardMark(!account.permissions.write.isEmpty)
            Color.clear.frame(width: 74, height: 1)
        }
    }

    private func standardMark(_ allowed: Bool) -> some View {
        Image(systemName: allowed ? "checkmark" : "minus")
            .foregroundStyle(allowed ? AnyShapeStyle(.secondary) : AnyShapeStyle(.tertiary))
            .frame(width: 64)
    }

    private func folderRow(_ name: String) -> some View {
        let overridden = account.permissions.folderRules[name] != nil
        let accessible = account.permissions.canAccess(name)
        return HStack {
            Label {
                HStack(spacing: 4) {
                    Text(name)
                    if !accessible {
                        Text(verbatim: "· \(L("no access"))")
                            .font(.caption)
                    }
                }
            } icon: {
                Image(systemName: mailboxSymbol(name))
            }
            .foregroundStyle(accessible ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
            Spacer()
            folderCheckbox(L("Read"), folderRuleBinding(name, \.read), enabled: account.permissions.read != .none)
            folderCheckbox(L("Write"), folderRuleBinding(name, \.write), enabled: !account.permissions.write.isEmpty)
            // The exception marker doubles as the way back: clicking it
            // restores the standard.
            Group {
                if overridden {
                    Button {
                        account.permissions.folderRules[name] = nil
                    } label: {
                        folderChip(L("adjusted"), prominent: true)
                    }
                    .buttonStyle(.plain)
                    .help(L("Reset to standard"))
                } else {
                    folderChip(L("Standard"), prominent: false)
                }
            }
            .frame(width: 74, alignment: .trailing)
        }
    }

    private func folderChip(_ text: String, prominent: Bool) -> some View {
        Text(text)
            .font(.caption2)
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(
                prominent ? Color.orange.opacity(0.16) : Color.primary.opacity(0.06),
                in: Capsule()
            )
            .foregroundStyle(prominent ? AnyShapeStyle(Color.orange) : AnyShapeStyle(.secondary))
    }

    /// A bare checkbox in a fixed column; the title stays for accessibility.
    private func folderCheckbox(_ title: String, _ isOn: Binding<Bool>, enabled: Bool) -> some View {
        Toggle(title, isOn: isOn)
            .toggleStyle(.checkbox)
            .labelsHidden()
            .disabled(!enabled)
            .frame(width: 64)
    }

    private var readEnabled: Binding<Bool> {
        Binding(
            get: { account.permissions.read != .none },
            set: { on in
                if on {
                    account.permissions.read = lastRead
                } else {
                    if account.permissions.read != .none {
                        lastRead = account.permissions.read
                    }
                    account.permissions.read = .none
                }
            }
        )
    }

    private var writeEnabled: Binding<Bool> {
        Binding(
            get: { !account.permissions.write.isEmpty },
            set: { on in
                if on {
                    account.permissions.write = lastWrite
                } else {
                    if !account.permissions.write.isEmpty {
                        lastWrite = account.permissions.write
                    }
                    account.permissions.write = .nothing
                }
            }
        )
    }

    /// Permanent delete escalates the trash right, so it falls with it.
    private var trashEnabled: Binding<Bool> {
        Binding(
            get: { account.permissions.write.trash },
            set: { on in
                account.permissions.write.trash = on
                if !on {
                    account.permissions.write.permanentDelete = false
                }
            }
        )
    }

    /// An exception that ends up equal to the standard dissolves — so the
    /// reset arrow only ever shows on rows that truly differ.
    private func folderRuleBinding(_ name: String, _ keyPath: WritableKeyPath<FolderRule, Bool>) -> Binding<Bool> {
        Binding(
            get: { (account.permissions.folderRules[name] ?? .standard)[keyPath: keyPath] },
            set: { allowed in
                var rule = account.permissions.folderRules[name] ?? .standard
                rule[keyPath: keyPath] = allowed
                account.permissions.folderRules[name] = rule == .standard ? nil : rule
            }
        )
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
                        .torroButton()
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
                        .torroButton()
                }
                .padding(.vertical, 2)
            }
        } header: {
            Text(L("Pending Actions"))
        } footer: {
            Text(L("Actions an assistant prepared and is waiting for you to approve."))
        }
    }

}

/// The named profiles as a compact chip strip — like a segmented control,
/// with the one difference a segmented control cannot show: in a custom
/// state, nothing is selected.
/// Shared with the setup wizard: the same control in both places, so the
/// choice made during setup is the one the user finds again later.
struct PresetChips: View {
    @Binding var permissions: PermissionSet

    var body: some View {
        HStack(spacing: 2) {
            ForEach(PermissionPreset.allCases) { preset in
                chip(preset)
            }
        }
        .padding(2)
        .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
    }

    private func chip(_ preset: PermissionPreset) -> some View {
        let isActive = permissions.matchingPreset == preset
        return Button {
            permissions.apply(preset)
        } label: {
            Text(label(for: preset))
                .font(.caption)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .foregroundStyle(isActive ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                .background {
                    if isActive {
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .fill(.background)
                            .overlay {
                                RoundedRectangle(cornerRadius: 5, style: .continuous)
                                    .strokeBorder(.separator, lineWidth: 0.5)
                            }
                    }
                }
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
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

    var body: some View {
        Form {
            Section {
                Toggle(L("Start at login"), isOn: $model.generalSettings.launchAtLogin)
            } footer: {
                Text(L("TorroMail and its MCP server start automatically in the background."))
            }

            Section {
                Toggle(L("Show in menu bar"), isOn: $model.generalSettings.showMenuBarIcon)
                Toggle(L("Keep in Dock"), isOn: $model.generalSettings.showDockIcon)
            } header: {
                Text(L("Where TorroMail shows up"))
            } footer: {
                Text(L("TorroMail serves assistants either way. With both off it only shows up while this window is open — open TorroMail again to bring it back."))
            }

            Section {
                Button(L("Set up assistants…")) {
                    model.selectedSidebarItem = .mcp
                }
                .torroButton()
            } header: {
                Text(L("MCP Clients"))
            } footer: {
                Text(L("The assistants connected to TorroMail live in the MCP Clients section."))
            }
        }
        .formStyle(.grouped)
        .navigationTitle(L("Settings"))
    }
}

private struct LogView: View {
    @EnvironmentObject private var model: TorroMailModel
    // Newest first by default. Ordering keys off the real timestamp, never the
    // rendered `time` string (which is not orderable across days); clicking any
    // header re-sorts by that column.
    @State private var sortOrder = [KeyPathComparator(\AuditEntry.timestamp, order: .reverse)]

    var body: some View {
        Table(model.audit.sorted(using: sortOrder), sortOrder: $sortOrder) {
            TableColumn(L("Time"), value: \.timestamp) { Text($0.time).monospacedDigit() }
            TableColumn(L("Client"), value: \.client) { Text($0.client) }
            TableColumn(L("Account"), value: \.account) { Text($0.account) }
            TableColumn(L("Event"), value: \.event) { Text(auditEventLabel($0.event)) }
            TableColumn(L("Details"), value: \.detail) { entry in
                Text(entry.detail).textSelection(.enabled)
            }
            TableColumn(L("Result"), value: \.result) { Text(auditResultLabel($0.result)) }
        }
        .overlay {
            if model.audit.isEmpty {
                ContentUnavailableView(
                    L("No activity yet"),
                    systemImage: "clock.arrow.circlepath",
                    description: Text(L("Every action an assistant takes is recorded here — reading, searching, drafting and the rest."))
                )
            }
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
