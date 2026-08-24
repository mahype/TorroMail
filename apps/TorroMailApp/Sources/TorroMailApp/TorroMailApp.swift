import CoreText
import ServiceManagement
import SwiftUI
import TorroMailKit

/// Localized user-facing string. English keys are the development language;
/// translations live in `Resources/<lang>.lproj/Localizable.strings`.
func L(_ key: String) -> String {
    NSLocalizedString(key, bundle: .module, comment: "")
}

/// The names the two encryptions go by everywhere else — mail clients say
/// "SSL/TLS" and "STARTTLS", so TorroMail does too rather than inventing its
/// own wording for a setting people copy from a provider's help page.
func label(for security: ConnectionSecurity) -> String {
    switch security {
    case .tls: L("SSL/TLS")
    case .startTLS: L("STARTTLS")
    }
}

/// Port and encryption on one row.
///
/// They belong together because they used to be one thing: the port decided
/// which TLS was spoken. Now that the server decides, side by side is what
/// keeps them readable as a single answer to "how do I reach this server".
///
/// The text is buffered rather than bound straight to the `Int`, so a field
/// cleared mid-edit stays empty instead of snapping to 0; the number only
/// travels once it parses.
struct PortField: View {
    let title: String
    @Binding var port: Int
    @Binding var security: ConnectionSecurity

    @State private var text: String = ""

    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 8) {
                TextField("", text: $text)
                    .labelsHidden()
                    // Right-aligned and only as wide as a port needs to be, so
                    // the number sits against the picker instead of floating
                    // in the middle of the row.
                    .multilineTextAlignment(.trailing)
                    .frame(width: 56)
                    .onChange(of: text) { _, typed in
                        if let value = Int(typed), (1...65535).contains(value) {
                            port = value
                        }
                    }
                Picker("", selection: $security) {
                    ForEach(ConnectionSecurity.allCases) { option in
                        Text(label(for: option)).tag(option)
                    }
                }
                .labelsHidden()
                // Hugging its content rather than a fixed width: with the row
                // pinned trailing, both pickers then end on the same edge as
                // every other value in the form, whichever option is showing.
                .fixedSize()
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
        .onAppear { text = String(port) }
        .onChange(of: port) { _, value in
            // Discovery filling the field in behind the user's back — the
            // guard keeps it from fighting what is being typed.
            if Int(text) != value { text = String(value) }
        }
    }
}

private func label(for method: LoginMethod) -> String {
    switch method {
    case .password: L("Password")
    case .oauth: L("OAuth")
    }
}

func label(for level: CacheLevel) -> String {
    switch level {
    case .off: L("Off")
    case .headers: L("Subject & sender")
    case .bodies: L("Full messages")
    case .attachments: L("Messages & attachments")
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
    case "mail_get_attachment": L("Downloaded an attachment")
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
        switch ReopenResponse.resolve(hasVisibleWindows: hasVisibleWindows) {
        case .showMainWindow:
            showMainWindow()
            // "Handled": returning true here lets AppKit run its own reopen
            // on top of ours, and the one window opens as two.
            return false
        case .letSystemProceed:
            return true
        }
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

/// The ServiceManagement side of `LoginItemSync`: read what the system has,
/// ask the Kit what should change, perform exactly that call.
///
/// `.requiresApproval` counts as registered — the item exists and macOS is
/// waiting for the user in System Settings; re-registering would change
/// nothing and nagging is not this app's call to make. Failures are
/// diagnostics, not decisions — they belong in the log.
///
/// `SMAppService.mainApp` registers the bundle it runs from, so a dev build
/// registers the dev bundle. Harmless as long as the toggle is flipped from
/// the same build, and the next launch of the installed app re-registers
/// itself anyway.
@MainActor
enum LoginItemService {
    static func apply(launchAtLogin: Bool) {
        let service = SMAppService.mainApp
        let registered = service.status == .enabled || service.status == .requiresApproval
        switch LoginItemSync.resolve(wantsLaunchAtLogin: launchAtLogin, systemHasLoginItem: registered) {
        case .register:
            do {
                try service.register()
            } catch {
                NSLog("TorroMail: login item registration failed: %@", error.localizedDescription)
            }
        case .unregister:
            do {
                try service.unregister()
            } catch {
                NSLog("TorroMail: login item removal failed: %@", error.localizedDescription)
            }
        case .inSync:
            break
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
    @StateObject private var updaterController = UpdaterController()
    @State private var auditWatcher = AuditWatcher()
    /// The same file watcher, pointed at `connections.jsonl` — the server
    /// appends there on every client handshake, so watching it keeps each
    /// client's "last connected" live rather than frozen at launch.
    @State private var connectionWatcher = AuditWatcher(url: try? ClientConnectionLog.defaultURL())
    /// The same file watcher again, pointed at `health.jsonl` — the server, a
    /// tool call and the monitor all append there, and this is what turns any
    /// of them into a status dot.
    @State private var healthWatcher = AuditWatcher(url: try? HealthLog.defaultURL())
    @State private var healthMonitor = AccountHealthMonitor()
    /// Turns the dots the watcher moves into something the user hears about
    /// while looking at something else entirely. Built in `init` rather than
    /// inline, because it has something to do there.
    @State private var healthNotifier: HealthNotifier

    init() {
        // Before any keychain access: the app never shows the consent dialog.
        // It reads its own items by Team ID; anything it cannot read that way
        // is skipped or re-minted rather than prompting the user.
        KeychainStore.silenceInteractivePrompts()
        registerBrandFont()
        // A notification tapped while TorroMail was not running is handed to
        // the delegate once, right after launch, so the delegate has to be in
        // place before launch finishes. Here rather than in the scene's
        // `.task`: measured, this runs some 170ms ahead of it. `.task` did in
        // fact also beat `applicationDidFinishLaunching` — but incidentally,
        // and its scheduling has already caught this file out once.
        let notifier = HealthNotifier()
        notifier.beginReceivingTaps()
        _healthNotifier = State(initialValue: notifier)
    }

    var body: some Scene {
        WindowGroup(id: Self.mainWindowID) {
            TorroMailRootView()
                .environmentObject(model)
                .environmentObject(mcpSupervisor)
                .environmentObject(presence)
                .environmentObject(updaterController)
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
                    connectionWatcher.start {
                        Task { @MainActor in model.reloadClientConnections() }
                    }
                    // Watch first, read second — the opposite of the two above,
                    // and deliberately: `start` takes its baseline
                    // synchronously, and a record landing between the read and
                    // the baseline would be inside the baseline and so never
                    // reported. This way that record moves the signature
                    // instead, and the worst case is one redundant re-read
                    // rather than a stale dot until the next pass. The server
                    // starting alongside the app writes exactly such a burst.
                    healthWatcher.start {
                        Task { @MainActor in model.applyHealth() }
                    }
                    // Whatever the log already says wins over the state file:
                    // the dots must be current before the first check lands.
                    model.applyHealth()
                    // Baseline *after* that derivation, never before it. The
                    // state file cannot store "broken": it round-trips an
                    // account through `isVerified`, so a failed one comes back
                    // as "not tested yet" — and seeding from the restored
                    // accounts would therefore read every standing problem as
                    // a fresh break and announce it, on every single launch.
                    // What the log says at launch is what the user last saw,
                    // and the only baseline that is not a lie.
                    healthNotifier.seed(accounts: model.accounts)
                    // Where a tap lands. Set here, once there is a model to
                    // navigate and a window to raise: a tap that arrived
                    // before this — the cold launch that the tap itself
                    // caused — has been held and is delivered by this
                    // assignment.
                    healthNotifier.reveal = { accountID in
                        // The account may be gone: removed while its
                        // notification sat in Notification Centre. The list is
                        // then the honest destination — a detail view for
                        // something that no longer exists is worse than the
                        // question "which account was that?".
                        if model.accounts.contains(where: { $0.id == accountID }) {
                            model.openAccount(id: accountID)
                        } else {
                            model.selectedSidebarItem = .accounts
                            model.accountPath = []
                        }
                        // The same door the menu bar item uses. TorroMail may
                        // have no window open at all — that is a normal state
                        // for it, not a broken one — and reopening it is
                        // something only the scene can do.
                        presence.showMainWindow()
                    }
                    healthMonitor.update(
                        accountIDs: model.checkableAccountIDs,
                        executableName: model.generalSettings.mcpExecutable
                    )
                    healthMonitor.start()
                }
                .onChange(of: model.generalSettings.showDockIcon, initial: true) { _, show in
                    presence.showDockIcon = show
                }
                // `initial: true` is the actual feature: the stored setting
                // alone starts nothing, so every launch walks it over to the
                // system's login-item list — including installs from before
                // this wiring existed, whose toggle said "on" into the void.
                .onChange(of: model.generalSettings.launchAtLogin, initial: true) { _, launch in
                    LoginItemService.apply(launchAtLogin: launch)
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
                    // An account added, removed or finally configured changes
                    // what the next pass may ask about. Kept here rather than
                    // read from the model by the monitor: the monitor checks
                    // from a background queue, where main-actor state is a
                    // trap, so the snapshot has to be pushed to it.
                    healthMonitor.update(
                        accountIDs: model.checkableAccountIDs,
                        executableName: model.generalSettings.mcpExecutable
                    )
                    // Last, and here rather than anywhere nearer the watcher:
                    // this is the one place every route to a changed dot —
                    // the log, a manual test, the wizard — has already
                    // converged, so a crossing is seen once however it came
                    // about.
                    healthNotifier.reconcile(accounts: accounts)
                }
                .onChange(of: model.generalSettings) { _, settings in
                    persistState(accounts: model.accounts, settings: settings)
                }
        }
        .commands {
            CommandGroup(after: .appInfo) {
                Button(L("Check for Updates…")) {
                    updaterController.checkForUpdates()
                }
                .disabled(!updaterController.isAvailable)
            }
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
                Label(L("Help"), systemImage: "questionmark.circle")
                    .tag(TorroMailSidebarSelection.help)
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 200, ideal: 220, max: 280)
            .navigationTitle("TorroMail")
            .safeAreaInset(edge: .bottom) {
                SidebarBrandFooter()
            }
            // The sidebar is an opaque surface, not translucent material
            // (design guide §Fenster): the sidebar material blends against
            // what is behind the window and falls back to exactly this grey
            // whenever the compositor cannot sample — first frames after the
            // window opens, window not key, Mission Control, screen capture.
            // The surface flickered between two states anyway; we take the
            // stable one. Applied after the footer inset so list and footer
            // share one ground, with the panel shadow towards the content.
            .scrollContentBackground(.hidden)
            .background(Color(nsColor: .windowBackgroundColor))
            .shadow(color: .black.opacity(0.12), radius: 6, x: 2, y: 0)
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
        case .help:
            HelpView()
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
                if model.health == .notConfigured {
                    GettingStartedCard()
                } else {
                    ServiceStatusCard()
                    BrokenAccountsCard()
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
        // The hero is the header of this pane: pinned above the scroll
        // content at full width, its red running up behind the toolbar. The
        // toolbar keeps no background and no title text — the wordmark takes
        // that role. An empty string (not a removed modifier) so the title a
        // previously selected pane set does not linger over the red.
        .safeAreaInset(edge: .top, spacing: 0) {
            BrandHero()
        }
        .navigationTitle("")
        .toolbarBackground(.hidden, for: .windowToolbar)
    }
}

/// The one place the brand gets to be loud: red ground, the wordmark, and in
/// one line what this app actually does. A full-bleed header band across the
/// detail pane, not a card: the text column mirrors the card column below so
/// wordmark and content share a left edge.
private struct BrandHero: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            TorroWordmark(capHeight: 15)
            // No `.fixedSize(horizontal: false, vertical: true)` here: outside
            // a ScrollView it drives the window's fitting-size negotiation
            // into the text's minimum width, and the whole split view lays
            // out collapsed and clipped. Plain wrapping needs no help in this
            // stack anyway.
            Text(L("Your mailboxes for AI assistants — nothing leaves without your say-so."))
                .font(.callout)
                .foregroundStyle(.white.opacity(0.92))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
        .frame(maxWidth: 720)
        .frame(maxWidth: .infinity)
        .padding(.top, 6)
        .padding(.bottom, 16)
        .background {
            // Diagonal, top-left to bottom-right, as torro-design specifies:
            // the red still reaches the window's top edge unbroken — the
            // expanded bounds see to that — it just runs to the deep-red foot
            // across the band rather than straight down it.
            LinearGradient(
                colors: [.torroRed, .torroRedDeep],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            // The signet as a watermark, the use torro-design lists for it. It
            // runs off the right edge on purpose — cropped by the band rather
            // than floating as a lone shape. It has to be an overlay: as a
            // sibling in a stack its fixed 150pt would set the height, and the
            // red would spill past the hero onto whatever sits below.
            .overlay(alignment: .trailing) {
                TorroSignet()
                    .fill(.white.opacity(0.10))
                    .frame(width: 260, height: 150)
                    .offset(x: 95)
            }
            // Clip before extending: the expanded frame is what the signet
            // gets cropped against, so the red — and the crop — reach the
            // window's top edge behind the background-less toolbar.
            .clipped()
            .shadow(color: .black.opacity(0.25), radius: 7, y: 2)
            .ignoresSafeArea(edges: .top)
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
                .help(title)
                .accessibilityLabel(title)
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
                            .buttonStyle(.bordered)
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

/// The accounts that stopped working, and what the server said about each.
///
/// Absent whenever everything is fine — deliberately, and this is the whole
/// design: a card that is always on screen saying "all good" gets skimmed
/// past, and then it goes unread on the one morning it says something else.
/// Its appearing *is* the message, which is also why it carries no green or
/// "last checked" state of its own; that lives one click away in the detail.
///
/// Rows lead to the account detail, because that is where the repair is —
/// re-entering a password, correcting a host. It is the same destination a
/// health notification opens, so both routes land on the same screen.
private struct BrokenAccountsCard: View {
    @EnvironmentObject private var model: TorroMailModel

    var body: some View {
        let broken = model.accounts.filter(\.connectionState.isBroken)
        if !broken.isEmpty {
            DashboardCard(
                title: L("Needs your attention"),
                footer: L("Assistants cannot use these accounts until this is fixed.")
            ) {
                VStack(spacing: 0) {
                    ForEach(Array(broken.enumerated()), id: \.element.id) { index, account in
                        if index > 0 { Divider().padding(.leading, 38) }
                        Button {
                            model.openAccount(id: account.id)
                        } label: {
                            // Top-aligned: the reason is the server's own
                            // sentence and can run to two or three lines, and
                            // the badge and chevron belong beside its first
                            // one rather than floating in the middle.
                            HStack(alignment: .top, spacing: 10) {
                                MailBadge(size: 26)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(account.name).font(.body)
                                    Text(reason(for: account))
                                        .font(.subheadline)
                                        .foregroundStyle(.secondary)
                                        // Full sentence or nothing: the tail of
                                        // these messages is the half that says
                                        // what to do about it, so it must not
                                        // be what gets dropped when the row
                                        // runs short of width.
                                        .fixedSize(horizontal: false, vertical: true)
                                }
                                Spacer(minLength: 8)
                                Image(systemName: "chevron.right")
                                    .font(.system(size: 11, weight: .semibold))
                                    .foregroundStyle(.tertiary)
                                    .padding(.top, 4)
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

    /// No dot and no "failed" label on these rows: every account in this card
    /// is broken by definition, so the only thing left worth the space is why.
    private func reason(for account: MailAccount) -> String {
        guard case let .failed(message) = account.connectionState, !message.isEmpty else {
            return L("TorroMail can no longer reach this account.")
        }
        return message
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
            // The title bar stays macOS: the app tint would paint this brand
            // red, and the chrome carries no brand colour.
            Button {
                model.beginAccountWizard()
            } label: {
                Label(L("Add Account"), systemImage: "plus")
            }
            .tint(nil)
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
        case .needsTest: L("Not tested yet — run Test Connection.")
        case .notConfigured: L("No credentials stored yet.")
        case let .failed(message): message
        }
    }
}

private struct AccountDetailView: View {
    @EnvironmentObject private var model: TorroMailModel
    @Binding var account: MailAccount
    // Held only for the moment of saving; the keychain is the home.
    @State private var password = ""
    /// Whether a password is on file for this account. An empty secure field
    /// is ambiguous — nothing stored, or stored and simply not shown — and the
    /// field is cleared the moment it is saved, so without this the save looks
    /// like a deletion. Asked by the item's existence, so it stays true for an
    /// account whose stored password can no longer be read.
    @State private var hasStoredPassword = false
    @State private var isCheckingConnection = false
    @State private var confirmRemoval = false
    // Remembered so switching a group off and on again restores the last
    // sub-selection instead of resetting the user's choice.
    @State private var lastRead: ReadAccess = .fullMessage
    @State private var lastWrite = WriteAccess(drafts: true, mark: true)
    // The cache section's live state: the measured figure, and the rebuild
    // in flight, if any. The figure is measured, never stored.
    @State private var cacheStorageBytes: Int64 = 0
    @State private var rebuildProgress: CacheRebuild.Progress?
    @State private var rebuildHandle: CacheRebuild?
    @State private var rebuildFailure: String?

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
        .onAppear {
            hasStoredPassword = KeychainStore.hasPassword(forAccount: account.id)
        }
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
                // Ports and encryption belong here too: setup guesses them,
                // and a wrong guess was previously uncorrectable without
                // removing the account and starting over.
                PortField(
                    title: L("IMAP Port"),
                    port: $account.imapPort,
                    security: $account.imapSecurity
                )
                TextField(L("SMTP Server"), text: $account.smtpHost, prompt: Text(verbatim: "smtp.example.com"))
                PortField(
                    title: L("SMTP Port"),
                    port: $account.smtpPort,
                    security: $account.smtpSecurity
                )
                TextField(L("Username"), text: $account.username)
                // The dots are the whole point: they say a password is on
                // file without pretending to show it, so leaving the screen
                // or saving does not read as having wiped it. Typing replaces
                // them, which is the only thing that ever changes it.
                SecureField(
                    L("Password"),
                    text: $password,
                    prompt: hasStoredPassword
                        ? Text(verbatim: "••••••••••••")
                        : Text(L("Not stored yet"))
                )
            }

            // Top-aligned so the buttons stay level with the badge's first
            // line: a rejected credential brings the server's own sentence
            // with it, which wraps to two or three lines here.
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 3) {
                    ConnectionStatusBadge(state: account.connectionState)
                    if let lastChecked {
                        Text(lastChecked)
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                    }
                }
                Spacer(minLength: 12)
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

    /// "Last checked 5 min. ago" — shown for a healthy account too, which is
    /// the one place this app volunteers status without an exception to
    /// report. It earns it: a green dot that nothing ever re-checked was the
    /// original bug, and "is this current?" is the question a dot alone cannot
    /// answer. Kept in caption/tertiary so it reads as a footnote to the
    /// badge rather than as a second status line.
    ///
    /// Nil until the log has seen this account at all — an account nothing has
    /// ever checked has no timestamp to be honest about.
    private var lastChecked: String? {
        guard let when = model.lastHealthCheck[account.id] else { return nil }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return String(format: L("Last checked %@"), formatter.localizedString(for: when, relativeTo: Date()))
    }

    /// Saves the password to the keychain and runs the real check: the MCP
    /// binary resolves the secret, connects over TLS and logs in — the dot
    /// reports what actually happened.
    private func testConnection() {
        if account.loginMethod == .password, !password.isEmpty {
            do {
                try KeychainStore.savePassword(password, forAccount: account.id)
                password = ""
                hasStoredPassword = true
            } catch {
                account.connectionState = .failed(L("Could not save the password to the keychain."))
                return
            }
        }

        isCheckingConnection = true
        let accountID = account.id
        let executable = model.generalSettings.mcpExecutable
        Task.detached(priority: .userInitiated) {
            let result = AccountCheck.run(accountID: accountID, executableName: executable)
            // The user pressed the button, so this check is a record like any
            // other — the log is where every check lands.
            HealthLog.append(
                HealthRecord(
                    accountID: accountID,
                    at: Date(),
                    outcome: result.outcome,
                    source: "manual",
                    detail: result.detail
                )
            )
            await MainActor.run {
                account.connectionState = result.state
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

    /// The level the disk actually follows: the picker's choice, capped by
    /// what assistants may read at all.
    private var effectiveCacheLevel: CacheLevel {
        min(account.searchCache.level, CacheLevel.ceiling(for: account.permissions.read))
    }

    private var cacheSection: some View {
        Section {
            Picker(L("Cache"), selection: $account.searchCache.level) {
                ForEach(CacheLevel.allCases) { level in
                    Text(label(for: level)).tag(level)
                }
            }
            if effectiveCacheLevel < account.searchCache.level {
                Text(
                    String(
                        format: L("Limited to “%@” by the read permission."),
                        label(for: effectiveCacheLevel)
                    )
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            if account.searchCache.level != .off {
                if let progress = rebuildProgress {
                    LabeledContent(L("Rebuilding…")) {
                        ProgressView(value: progress.fraction)
                            .frame(maxWidth: 140)
                        Button(L("Cancel")) { cancelRebuild() }
                            .buttonStyle(.bordered)
                    }
                } else {
                    LabeledContent(L("Storage")) {
                        Text(CacheStorage.label(bytes: cacheStorageBytes))
                        Button(L("Rebuild")) { startRebuild() }
                            .buttonStyle(.bordered)
                        Button(L("Delete")) { deleteCache() }
                            .torroButton()
                    }
                }
            }
            if let failure = rebuildFailure {
                Text(failure)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        } header: {
            Text(L("Search & Cache"))
        } footer: {
            Text(L("More caching makes search faster but stores mail content on this Mac."))
        }
        .onAppear {
            cacheStorageBytes = CacheStorage.sizeBytes(accountID: account.id)
        }
        .onChange(of: account.searchCache.level) {
            // The disk follows the decision, whichever direction it went.
            CacheTrim.run(
                accountID: account.id,
                executableName: model.generalSettings.mcpExecutable
            )
            refreshCacheStorage()
        }
        .onChange(of: account.permissions.read) {
            // A lowered read permission caps the cache; trim right away.
            CacheTrim.run(
                accountID: account.id,
                executableName: model.generalSettings.mcpExecutable
            )
            refreshCacheStorage()
        }
    }

    /// The size figure is measured slightly later than the action that
    /// changed it — the trim runs in its own process.
    private func refreshCacheStorage() {
        let accountID = account.id
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            cacheStorageBytes = CacheStorage.sizeBytes(accountID: accountID)
        }
    }

    private func startRebuild() {
        rebuildFailure = nil
        let accountID = account.id
        let handle = CacheRebuild.start(
            accountID: accountID,
            executableName: model.generalSettings.mcpExecutable,
            onProgress: { progress in
                DispatchQueue.main.async {
                    rebuildProgress = progress
                }
            },
            onFinish: { outcome in
                DispatchQueue.main.async {
                    rebuildProgress = nil
                    rebuildHandle = nil
                    if case let .failure(error) = outcome {
                        rebuildFailure = error.localizedDescription
                    }
                    cacheStorageBytes = CacheStorage.sizeBytes(accountID: accountID)
                }
            }
        )
        if let handle {
            rebuildHandle = handle
            rebuildProgress = CacheRebuild.Progress(phase: "headers", mailbox: "", done: 0, total: 0)
        } else {
            rebuildFailure = L("MCP executable not found")
        }
    }

    private func cancelRebuild() {
        rebuildHandle?.cancel()
    }

    private func deleteCache() {
        do {
            try CacheStorage.delete(accountID: account.id)
            cacheStorageBytes = 0
            rebuildFailure = nil
        } catch {
            rebuildFailure = error.localizedDescription
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
                        .buttonStyle(.bordered)
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

    /// For a failure the badge's text *is* the reason, and those reasons are no
    /// longer short — a rejected credential arrives as a full sentence from the
    /// server or from the keychain repair path, three lines of it at this
    /// width. Centred, the dot then sat halfway down the paragraph with no
    /// text beside it, reading as a bullet for the middle line; the same
    /// happened to the buttons across the row. Top alignment with the dot
    /// nudged onto the first line fixes that. `fixedSize` is belt and braces:
    /// the text wraps here on its own, but it stops the row's other content
    /// from ever compressing it into a truncated line.
    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
                .padding(.top, 5)
            Text(text)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
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
        case .needsTest: L("Not tested yet")
        case .notConfigured: L("No credentials stored yet")
        case let .failed(message): message
        }
    }
}

private struct SettingsView: View {
    @EnvironmentObject private var model: TorroMailModel
    @EnvironmentObject private var updaterController: UpdaterController

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

            Section {
                Toggle(
                    L("Automatically check for updates"),
                    isOn: $updaterController.automaticallyChecksForUpdates
                )
                .disabled(!updaterController.isAvailable)

                Button(L("Check for updates now")) {
                    updaterController.checkForUpdates()
                }
                .disabled(!updaterController.isAvailable)

                if updaterController.isAvailable {
                    HStack(spacing: 4) {
                        Text(L("Last checked:"))
                        if let date = updaterController.lastUpdateCheckDate {
                            Text(date, format: .dateTime.day().month().year().hour().minute())
                        } else {
                            Text(L("Never"))
                        }
                    }
                    .font(.callout)
                    .foregroundStyle(.secondary)
                }
            } header: {
                Text(L("Updates"))
            } footer: {
                if updaterController.isAvailable {
                    Text(L("TorroMail checks for new versions at launch and then every 24 hours. Updates download in the background and install when you restart."))
                } else {
                    Text(L("Updates are available in signed release builds."))
                }
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
            .tint(nil)
        }
    }
}

/// Where the app says which version it is, and where to go when something is
/// wrong. Built like TorroWhisper's help page so the two apps answer "what am
/// I running?" in the same place and the same words.
private struct HelpView: View {
    @EnvironmentObject private var model: TorroMailModel

    /// The repository is the whole support surface: releases, docs, issues.
    private static let repository = "https://github.com/mahype/TorroMail"

    var body: some View {
        Form {
            Section {
                HStack(spacing: 12) {
                    AppIconTile(size: 40)

                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: "TorroMail")
                            .font(.title3.weight(.semibold))
                        Text(L("Your mailboxes for AI assistants"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    Spacer(minLength: 0)
                }
                .padding(.vertical, 2)

                LabeledContent(L("Version")) {
                    Text(verbatim: appVersion).monospacedDigit().textSelection(.enabled)
                }
                LabeledContent(L("Bundle")) {
                    Text(verbatim: bundleIdentifier).textSelection(.enabled)
                }

                Button(L("Open release notes on GitHub")) {
                    open("\(Self.repository)/releases/tag/v\(appVersion)")
                }
                .torroButton()
                // A dev build carries no released version, so there is no
                // release page to open — the tag would 404.
                .disabled(!hasReleasedVersion)
            } header: {
                Text(L("About TorroMail"))
            }

            Section {
                Button(L("Open documentation")) {
                    open(Self.repository)
                }
                .torroButton()
                Button(L("Report a problem")) {
                    open("\(Self.repository)/issues/new")
                }
                .torroButton()
            } header: {
                Text(L("Help"))
            } footer: {
                Text(L("Please include the version above when you report something."))
            }

            Section {
                Button(L("Open Log")) {
                    model.selectedSidebarItem = .log
                }
                .torroButton()
            } header: {
                Text(L("Diagnostics"))
            } footer: {
                Text(L("Every action an assistant takes is recorded in the log — that is the first place to look when something went differently than expected."))
            }
        }
        .formStyle(.grouped)
        .navigationTitle(L("Help"))
    }

    /// Stamped into `Info.plist` at build time from the workspace version; the
    /// dev bundle stamps `dev` instead.
    private var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—"
    }

    private var bundleIdentifier: String {
        Bundle.main.bundleIdentifier ?? "—"
    }

    /// Only a plain SemVer has a release page behind it. Dev bundles stamp
    /// `git describe` — `0.2.1-dirty`, `0.2.1-3-gabc123` — and those tags do
    /// not exist, so the button would open a 404.
    private var hasReleasedVersion: Bool {
        appVersion.wholeMatch(of: /\d+\.\d+\.\d+(-rc\.\d+)?/) != nil
    }

    private func open(_ string: String) {
        guard let url = URL(string: string) else { return }
        NSWorkspace.shared.open(url)
    }
}

/// The app icon itself, at tile size — the real artwork from the bundle rather
/// than a redrawn lookalike, so it cannot drift from what the Dock shows. The
/// red signet stands in while running unbundled, where there is no icon.
private struct AppIconTile: View {
    var size: CGFloat = 40

    var body: some View {
        Group {
            if let icon = NSApp?.applicationIconImage {
                Image(nsImage: icon).resizable()
            } else {
                RoundedRectangle(cornerRadius: size * 0.2237, style: .continuous)
                    .fill(
                        LinearGradient(
                            colors: [.torroRed, .torroRedDeep],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    )
                    .overlay {
                        TorroSignet()
                            .fill(.white)
                            .frame(width: size * 0.62, height: size * 0.35)
                    }
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}
