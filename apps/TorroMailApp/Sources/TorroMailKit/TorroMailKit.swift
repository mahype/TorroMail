import Foundation
import Security
import SwiftUI

public enum TorroMailSidebarSelection: Hashable {
    case dashboard
    case accounts
    case settings
    case log
}

/// What the app wants to say about itself in one word. The dashboard leads
/// with this and stays quiet when there is nothing to decide.
public enum SystemHealth: Hashable {
    /// Nothing set up yet — the dashboard explains the app instead.
    case notConfigured
    /// Something needs the user: broken credentials, or a stopped service.
    case needsAttention
    /// Everything running, nothing waiting.
    case ready
}

/// A note from Torro — a new release, another product. Local data for now;
/// nothing here reaches the network.
public struct NewsItem: Identifiable, Hashable {
    public var id: String
    public var title: String
    public var detail: String
    public var symbol: String
    public var isNew: Bool

    public init(id: String, title: String, detail: String, symbol: String, isNew: Bool = false) {
        self.id = id
        self.title = title
        self.detail = detail
        self.symbol = symbol
        self.isNew = isNew
    }
}

/// The product boundary, stated as data so the contract test can hold the
/// codebase to it: TorroMail configures and controls mail access — it never
/// becomes the place where a human reads mail.
public enum TorroMailProductBoundary {
    public static let allowedAppRole = "Setup, consent, MCP lifecycle, status, audit, and diagnostics"

    public static let disallowedUserMailSurfaces = [
        "Inbox UI",
        "Message reader",
        "Thread browser",
        "Manual triage workflow"
    ]
}

/// Appearance facts the control surface commits to: both system modes, an
/// 8-point spacing base, and semantic colors everywhere outside the declared
/// brand moments (the red ground, the wordmark).
public enum TorroMailAppearance {
    public static let spacingUnit = 8
    public static let supportsLightMode = true
    public static let supportsDarkMode = true
    public static let usesSemanticColors = true
}

/// Passwords live in the macOS keychain and nowhere else. The app writes
/// them under one service name; the policy document only ever carries the
/// reference (`keychain://TorroMail/{account}`), which the MCP server
/// resolves itself — macOS asks the user once to allow it.
public enum KeychainStore {
    public static let service = "TorroMail"

    public struct Failure: Error {
        public let status: OSStatus
    }

    public static func secretReference(forAccount accountID: String) -> String {
        "keychain://\(service)/\(accountID)"
    }

    public static func savePassword(_ password: String, forAccount accountID: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountID
        ]
        let payload: [String: Any] = [
            kSecValueData as String: Data(password.utf8)
        ]

        let updateStatus = SecItemUpdate(query as CFDictionary, payload as CFDictionary)
        if updateStatus == errSecItemNotFound {
            let addStatus = SecItemAdd(query.merging(payload) { _, new in new } as CFDictionary, nil)
            guard addStatus == errSecSuccess else { throw Failure(status: addStatus) }
            return
        }
        guard updateStatus == errSecSuccess else { throw Failure(status: updateStatus) }
    }
}

/// Runs the MCP binary's `--check-account`: the same secret resolution,
/// TLS and LOGIN the tools use — so a green dot means the real path works.
public enum AccountCheck {
    public static func run(accountID: String, executableName: String) -> ConnectionState {
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        guard let command = locator.resolve() else {
            return .failed("MCP executable not found")
        }

        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments + ["--check-account", accountID]
        var environment = ProcessInfo.processInfo.environment
        if let policyURL = try? PolicyDocument.defaultURL() {
            environment["TORROMAIL_POLICY_PATH"] = policyURL.path
        }
        process.environment = environment
        let errorPipe = Pipe()
        process.standardOutput = Pipe()
        process.standardError = errorPipe

        do {
            try process.run()
        } catch {
            return .failed(error.localizedDescription)
        }
        process.waitUntilExit()

        if process.terminationStatus == 0 {
            return .connected
        }
        let detail = String(
            data: errorPipe.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        )?.trimmingCharacters(in: .whitespacesAndNewlines)
        return .failed(detail?.isEmpty == false ? detail ?? "" : "connection check failed")
    }
}

/// The internal bridge between the app and every `torromail-mcp` instance:
/// the app publishes this document whenever permissions change, the server
/// reloads it per tool call. Internal plumbing — never a user-facing
/// configuration path.
public enum PolicyDocument {
    public static let version = 1

    /// `~/Library/Application Support/TorroMail/policy.json` — the path the
    /// server falls back to when `TORROMAIL_POLICY_PATH` is not set.
    public static func defaultURL(fileManager: FileManager = .default) throws -> URL {
        try fileManager
            .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("TorroMail", isDirectory: true)
            .appendingPathComponent("policy.json")
    }

    public static func data(for accounts: [MailAccount]) throws -> Data {
        let document: [String: Any] = [
            "version": version,
            "accounts": accounts.map(accountObject(for:))
        ]
        return try JSONSerialization.data(withJSONObject: document, options: [.sortedKeys])
    }

    /// Writes atomically so a reloading server never sees a half document.
    public static func publish(
        accounts: [MailAccount],
        to url: URL? = nil,
        fileManager: FileManager = .default
    ) throws {
        let target = try url ?? defaultURL(fileManager: fileManager)
        try fileManager.createDirectory(
            at: target.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try data(for: accounts).write(to: target, options: .atomic)
    }

    private static func accountObject(for account: MailAccount) -> [String: Any] {
        var object: [String: Any] = [
            "id": account.id,
            "read": name(for: account.permissions.read),
            "write": [
                "drafts": account.permissions.write.drafts,
                "mark": account.permissions.write.mark,
                "move": account.permissions.write.move,
                "trash": account.permissions.write.trash,
                "permanent_delete": account.permissions.write.permanentDelete
            ],
            "send": account.permissions.send,
            "per_folder": account.permissions.perFolder,
            "folder_rules": account.permissions.folderRules.mapValues { rule in
                ["read": rule.read, "write": rule.write]
            }
        ]
        // Connection facts travel once the user has provided them; the
        // password itself stays in the keychain, only the reference moves.
        if account.loginMethod == .password, !account.imapHost.isEmpty, !account.username.isEmpty {
            object["imap"] = [
                "host": account.imapHost,
                "port": 993,
                "username": account.username,
                "secret_ref": KeychainStore.secretReference(forAccount: account.id)
            ]
        }
        return object
    }

    private static func name(for read: ReadAccess) -> String {
        switch read {
        case .none: "none"
        case .headers: "headers"
        case .fullMessage: "full_message"
        case .withAttachments: "with_attachments"
        }
    }
}

public enum Provider: String, CaseIterable, Identifiable, Hashable {
    case imapSmtp = "IMAP/SMTP"
    case gmail = "Gmail"
    case microsoft = "Microsoft 365"
    case jmap = "JMAP"

    public var id: Self { self }
}

public enum LoginMethod: CaseIterable, Identifiable, Hashable {
    case password
    case oauth

    public var id: Self { self }
}

public enum CacheMode: CaseIterable, Identifiable, Hashable {
    case metadata
    case headers
    case body
    case fullText

    public var id: Self { self }
}

/// Connection health of one account. The UI stays quiet while everything is
/// fine and only surfaces states that need the user's attention.
public enum ConnectionState: Hashable {
    case notConfigured
    case needsTest
    case connected
    case failed(String)

    public var needsAttention: Bool {
        self != .connected
    }

    /// Whether the stored credentials are known to be broken, as opposed to
    /// merely unverified. Drives the red dot on the account card.
    public var isBroken: Bool {
        if case .failed = self { return true }
        return false
    }
}

/// How deep assistants may read. The levels build on each other: the message
/// implies its header, attachments imply the message.
public enum ReadAccess: Int, CaseIterable, Identifiable, Hashable, Comparable, Sendable {
    case none = 0
    case headers = 1
    case fullMessage = 2
    case withAttachments = 3

    public var id: Self { self }

    public static func < (lhs: Self, rhs: Self) -> Bool {
        lhs.rawValue < rhs.rawValue
    }
}

/// Mailbox mutations — everything that changes the mailbox but stays on the
/// server. Sending is deliberately not in here: it is the one right that acts
/// on the outside world.
public struct WriteAccess: Hashable, Sendable {
    public var drafts: Bool
    public var mark: Bool
    public var move: Bool
    public var trash: Bool
    /// Escalation of `trash`; meaningless without it, so `sanitized()` clears
    /// it when the trash right falls. Never part of a preset.
    public var permanentDelete: Bool

    public init(
        drafts: Bool = false,
        mark: Bool = false,
        move: Bool = false,
        trash: Bool = false,
        permanentDelete: Bool = false
    ) {
        self.drafts = drafts
        self.mark = mark
        self.move = move
        self.trash = trash
        self.permanentDelete = permanentDelete
    }

    public static let nothing = WriteAccess()

    public var isEmpty: Bool {
        !(drafts || mark || move || trash || permanentDelete)
    }

    /// Permanent delete cannot outlive the trash right it escalates.
    public func sanitized() -> WriteAccess {
        var copy = self
        if !copy.trash {
            copy.permanentDelete = false
        }
        return copy
    }
}

/// Per-mailbox exception to the account-wide groups. `true` means "the group
/// applies here" — the effective right is always the intersection with the
/// account, so a folder can never allow more than the account does.
public struct FolderRule: Hashable, Sendable {
    public var read: Bool
    public var write: Bool

    public init(read: Bool = true, write: Bool = true) {
        self.read = read
        self.write = write
    }

    /// What every folder starts as: follow the account.
    public static let standard = FolderRule()
}

/// What connected assistants may do with an account, in three groups modelled
/// on the file system: read (changes nothing), write (changes the mailbox but
/// stays on the server), send (leaves the house). Folder exceptions scope the
/// first two; sending is account-wide because SMTP is not bound to a mailbox.
public struct PermissionSet: Hashable, Sendable {
    public var read: ReadAccess
    public var write: WriteAccess
    public var send: Bool
    /// Whether folder exceptions apply. Off means the groups rule every
    /// folder; stored exceptions are kept, just inert.
    public var perFolder: Bool
    /// Exceptions by mailbox name. No entry = standard (follow the account).
    /// Folders the server reports later start with no entry, so they follow
    /// the account automatically.
    public var folderRules: [String: FolderRule]

    public init(
        read: ReadAccess = .fullMessage,
        write: WriteAccess = WriteAccess(drafts: true),
        send: Bool = false,
        perFolder: Bool = false,
        folderRules: [String: FolderRule] = [:]
    ) {
        self.read = read
        self.write = write
        self.send = send
        self.perFolder = perFolder
        self.folderRules = folderRules
    }

    public func rule(for mailbox: String) -> FolderRule {
        guard perFolder else { return .standard }
        return folderRules[mailbox] ?? .standard
    }

    public func readAccess(in mailbox: String) -> ReadAccess {
        rule(for: mailbox).read ? read : .none
    }

    public func writeAccess(in mailbox: String) -> WriteAccess {
        rule(for: mailbox).write ? write.sanitized() : .nothing
    }

    /// A mailbox with no effective rights is invisible to assistants.
    public func canAccess(_ mailbox: String) -> Bool {
        readAccess(in: mailbox) != .none || !writeAccess(in: mailbox).isEmpty
    }
}

/// The named starting points. A preset is a fact about the current values —
/// derived, never stored — so a "custom" state can never drift out of sync
/// with the switches.
public enum PermissionPreset: CaseIterable, Identifiable, Hashable, Sendable {
    /// Read everything, change nothing.
    case readOnly
    /// The everyday default: read mail and prepare drafts, send nothing.
    case readAndDrafts
    /// Let an assistant keep the mailbox in shape: mark, move, delete —
    /// without composing or sending.
    case tidyUp
    /// Everything except permanent deletion, which is never preset.
    case fullAccess

    public var id: Self { self }

    public var read: ReadAccess {
        self == .fullAccess ? .withAttachments : .fullMessage
    }

    public var write: WriteAccess {
        switch self {
        case .readOnly: WriteAccess()
        case .readAndDrafts: WriteAccess(drafts: true)
        case .tidyUp: WriteAccess(mark: true, move: true, trash: true)
        case .fullAccess: WriteAccess(drafts: true, mark: true, move: true, trash: true)
        }
    }

    public var send: Bool {
        self == .fullAccess
    }
}

extension PermissionSet {
    /// The preset these values match, if any. Folder exceptions do not count:
    /// presets decide the groups, exceptions only scope them.
    public var matchingPreset: PermissionPreset? {
        PermissionPreset.allCases.first {
            $0.read == read && $0.write == write && $0.send == send
        }
    }

    /// Applies the preset's groups. Folder exceptions survive — switching the
    /// profile is not meant to throw away per-folder decisions.
    public mutating func apply(_ preset: PermissionPreset) {
        read = preset.read
        write = preset.write
        send = preset.send
    }
}

public struct SearchCacheSettings: Hashable {
    public var localCacheEnabled: Bool
    public var cacheMode: CacheMode
    public var indexBodies: Bool
    public var indexAttachments: Bool
    public var storage: String

    public init(
        localCacheEnabled: Bool = true,
        cacheMode: CacheMode = .metadata,
        indexBodies: Bool = false,
        indexAttachments: Bool = false,
        storage: String = "0 MB"
    ) {
        self.localCacheEnabled = localCacheEnabled
        self.cacheMode = cacheMode
        self.indexBodies = indexBodies
        self.indexAttachments = indexAttachments
        self.storage = storage
    }
}

public struct PendingAction: Identifiable, Hashable {
    public var id: String
    public var accountID: String
    public var toolCall: String
    public var subject: String
    public var recipient: String
    public var affectedMessages: Int
    public var expiresIn: String

    public init(
        id: String,
        accountID: String,
        toolCall: String,
        subject: String,
        recipient: String,
        affectedMessages: Int,
        expiresIn: String
    ) {
        self.id = id
        self.accountID = accountID
        self.toolCall = toolCall
        self.subject = subject
        self.recipient = recipient
        self.affectedMessages = affectedMessages
        self.expiresIn = expiresIn
    }
}

public struct MailAccount: Identifiable, Hashable {
    public var id: String
    public var name: String
    public var email: String
    public var provider: Provider
    public var loginMethod: LoginMethod
    public var imapHost: String
    public var smtpHost: String
    public var username: String
    public var connectionState: ConnectionState
    /// Mailboxes the server reported (`mail_list_mailboxes`), in display
    /// order. Rights are decided in `permissions`; folders that appear here
    /// later automatically follow the account-wide standard.
    public var knownMailboxes: [String]
    public var permissions: PermissionSet
    public var searchCache: SearchCacheSettings
    public var pendingActions: [PendingAction]

    public init(
        id: String,
        name: String,
        email: String,
        provider: Provider,
        loginMethod: LoginMethod,
        imapHost: String = "",
        smtpHost: String = "",
        username: String = "",
        connectionState: ConnectionState = .notConfigured,
        knownMailboxes: [String] = ["INBOX"],
        permissions: PermissionSet = PermissionSet(),
        searchCache: SearchCacheSettings = SearchCacheSettings(),
        pendingActions: [PendingAction] = []
    ) {
        self.id = id
        self.name = name
        self.email = email
        self.provider = provider
        self.loginMethod = loginMethod
        self.imapHost = imapHost
        self.smtpHost = smtpHost
        self.username = username
        self.connectionState = connectionState
        self.knownMailboxes = knownMailboxes
        self.permissions = permissions
        self.searchCache = searchCache
        self.pendingActions = pendingActions
    }

    public var needsAttention: Bool {
        connectionState.needsAttention || !pendingActions.isEmpty
    }
}

public struct GeneralSettings: Hashable {
    /// The one lifecycle decision the user makes: TorroMail (and with it the
    /// MCP server) starts automatically at login. Everything else is derived.
    public var launchAtLogin: Bool

    /// Whether TorroMail keeps a Dock tile while it has no window open. Off by
    /// default: the app's job is done in the background, so it stays out of the
    /// Dock until the user opens the window.
    public var showDockIcon: Bool

    /// Whether TorroMail sits in the menu bar. Off by default for the same
    /// reason — presence is something the user opts into.
    public var showMenuBarIcon: Bool

    /// Internal: which executable the supervisor launches. Never shown in the
    /// UI — diagnostics belong in the log.
    public var mcpExecutable: String

    public init(
        launchAtLogin: Bool = true,
        showDockIcon: Bool = false,
        showMenuBarIcon: Bool = false,
        mcpExecutable: String = "torromail-mcp"
    ) {
        self.launchAtLogin = launchAtLogin
        self.showDockIcon = showDockIcon
        self.showMenuBarIcon = showMenuBarIcon
        self.mcpExecutable = mcpExecutable
    }
}

/// How much of TorroMail the system sees. Maps onto AppKit's activation
/// policy, but stated in the terms the setting is about.
public enum AppPresence: Hashable {
    /// Dock tile and app menu — a normal app.
    case foreground
    /// Neither. TorroMail keeps serving assistants, invisibly.
    case background

    /// With the Dock icon switched off, an open window still pulls TorroMail
    /// into the Dock — a window the user cannot switch to would be worse than
    /// the tile they wanted gone.
    public static func resolve(showDockIcon: Bool, hasOpenWindow: Bool) -> AppPresence {
        showDockIcon || hasOpenWindow ? .foreground : .background
    }
}

public enum MCPServerStatus: Hashable {
    case stopped
    case starting
    case running(String)
    case notFound(String)
    case failed(String)

    public var isRunning: Bool {
        if case .running = self { return true }
        return false
    }
}

public struct MCPLaunchCommand: Hashable {
    public var executableURL: URL
    public var arguments: [String]
    public var displayPath: String

    public init(executableURL: URL, arguments: [String], displayPath: String) {
        self.executableURL = executableURL
        self.arguments = arguments
        self.displayPath = displayPath
    }
}

public struct MCPExecutableLocator: Hashable {
    public var executableName: String
    public var workspaceRoot: String?

    public init(executableName: String, workspaceRoot: String? = nil) {
        self.executableName = executableName
        self.workspaceRoot = workspaceRoot
    }

    public var candidatePaths: [String] {
        var paths: [String] = []
        if let workspaceRoot {
            paths.append("\(workspaceRoot)/target/debug/\(executableName)")
        }
        paths.append(executableName)
        return paths
    }

    public func resolve(fileManager: FileManager = .default) -> MCPLaunchCommand? {
        for candidate in candidatePaths {
            if candidate.contains("/") {
                if fileManager.isExecutableFile(atPath: candidate) {
                    return MCPLaunchCommand(
                        executableURL: URL(fileURLWithPath: candidate),
                        arguments: [],
                        displayPath: candidate
                    )
                }
            } else if fileManager.isExecutableFile(atPath: "/usr/bin/env") {
                return MCPLaunchCommand(
                    executableURL: URL(fileURLWithPath: "/usr/bin/env"),
                    arguments: [candidate],
                    displayPath: candidate
                )
            }
        }
        return nil
    }
}

@MainActor
public final class MCPServerSupervisor: ObservableObject {
    @Published public private(set) var status: MCPServerStatus
    private var process: Process?
    private var standardInput: Pipe?
    private var standardOutput: Pipe?
    private var standardError: Pipe?

    public init(status: MCPServerStatus = .stopped) {
        self.status = status
    }

    public func start(executableName: String) {
        if process?.isRunning == true {
            return
        }

        status = .starting
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )

        guard let command = locator.resolve() else {
            status = .notFound(executableName)
            return
        }

        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments
        // The supervised server reads the same policy document the app
        // publishes, so the permissions set in the UI are what it enforces.
        var environment = ProcessInfo.processInfo.environment
        if let policyURL = try? PolicyDocument.defaultURL() {
            environment["TORROMAIL_POLICY_PATH"] = policyURL.path
        }
        process.environment = environment
        standardInput = Pipe()
        standardOutput = Pipe()
        standardError = Pipe()
        process.standardInput = standardInput
        process.standardOutput = standardOutput
        process.standardError = standardError

        do {
            try process.run()
            self.process = process
            status = .running(command.displayPath)
        } catch {
            self.process = nil
            status = .failed(error.localizedDescription)
        }
    }

    public func stop() {
        if process?.isRunning == true {
            process?.terminate()
        }
        process = nil
        standardInput = nil
        standardOutput = nil
        standardError = nil
        status = .stopped
    }
}

public struct AuditEntry: Identifiable, Hashable {
    public var id: UUID
    public var time: String
    public var client: String
    public var account: String
    public var event: String
    public var result: String

    public init(
        id: UUID = UUID(),
        time: String,
        client: String,
        account: String,
        event: String,
        result: String
    ) {
        self.id = id
        self.time = time
        self.client = client
        self.account = account
        self.event = event
        self.result = result
    }
}

@MainActor
public final class TorroMailModel: ObservableObject {
    @Published public var accounts: [MailAccount]
    @Published public var selectedSidebarItem: TorroMailSidebarSelection
    /// Drill-down inside the accounts destination. Empty shows the account
    /// list; one entry shows that account's settings.
    @Published public var accountPath: [String]
    @Published public var showAccountWizard: Bool
    @Published public var generalSettings: GeneralSettings
    @Published public var audit: [AuditEntry]
    /// Assistants currently talking to the MCP server.
    @Published public var connectedClients: [String]
    @Published public var news: [NewsItem]

    public init(
        accounts: [MailAccount],
        selectedSidebarItem: TorroMailSidebarSelection,
        accountPath: [String] = [],
        showAccountWizard: Bool = false,
        generalSettings: GeneralSettings,
        audit: [AuditEntry],
        connectedClients: [String] = [],
        news: [NewsItem] = []
    ) {
        self.accounts = accounts
        self.selectedSidebarItem = selectedSidebarItem
        self.accountPath = accountPath
        self.showAccountWizard = showAccountWizard
        self.generalSettings = generalSettings
        self.audit = audit
        self.connectedClients = connectedClients
        self.news = news
    }

    /// The headline state. Accounts that were never tested do not count as a
    /// problem — only credentials the server actually rejected do.
    public var health: SystemHealth {
        if accounts.isEmpty { return .notConfigured }
        if accounts.contains(where: { $0.connectionState.isBroken }) { return .needsAttention }
        return .ready
    }

    /// Every approval waiting, newest account first, flattened for the
    /// dashboard.
    public var pendingActions: [PendingAction] {
        accounts.flatMap(\.pendingActions)
    }

    public func accountName(id: String) -> String? {
        accounts.first { $0.id == id }?.name
    }

    /// Total approvals waiting across all accounts — the badge on the
    /// accounts sidebar item.
    public var pendingActionCount: Int {
        accounts.reduce(0) { $0 + $1.pendingActions.count }
    }

    public func openAccount(id: String) {
        selectedSidebarItem = .accounts
        accountPath = [id]
    }

    public func beginAccountWizard() {
        showAccountWizard = true
    }

    public func addAccount(name: String, email: String, provider: Provider, loginMethod: LoginMethod) {
        let account = MailAccount(
            id: UUID().uuidString,
            name: name,
            email: email,
            provider: provider,
            loginMethod: loginMethod,
            username: email,
            connectionState: .notConfigured
        )
        accounts.append(account)
        openAccount(id: account.id)
    }

    /// Removing the open account drops back to the account list rather than
    /// picking a neighbour the user never asked for.
    public func removeAccount(id: String) {
        accounts.removeAll { $0.id == id }
        accountPath.removeAll { $0 == id }
    }
}

extension TorroMailModel {
    public static func preview() -> TorroMailModel {
        TorroMailModel(
            accounts: [
                MailAccount(
                    id: "work",
                    name: "Work",
                    email: "work@example.com",
                    provider: .microsoft,
                    loginMethod: .oauth,
                    imapHost: "outlook.office365.com",
                    smtpHost: "smtp.office365.com",
                    username: "work@example.com",
                    connectionState: .connected,
                    knownMailboxes: [
                        "INBOX", "Archive", "Sent", "Drafts", "Trash",
                        "Invoices 2026", "Private"
                    ],
                    permissions: PermissionSet(
                        read: .withAttachments,
                        write: WriteAccess(drafts: true, mark: true, move: true, trash: true),
                        send: true,
                        perFolder: true,
                        folderRules: [
                            "Archive": FolderRule(read: true, write: false),
                            "Private": FolderRule(read: false, write: false)
                        ]
                    ),
                    searchCache: SearchCacheSettings(
                        cacheMode: .headers,
                        storage: "42 MB"
                    ),
                    pendingActions: [
                        PendingAction(
                            id: "pending-1",
                            accountID: "work",
                            toolCall: "mail_prepare_send",
                            subject: "Contract update",
                            recipient: "customer@example.com",
                            affectedMessages: 1,
                            expiresIn: "01:08"
                        )
                    ]
                ),
                MailAccount(
                    id: "personal",
                    name: "Personal",
                    email: "me@example.net",
                    provider: .imapSmtp,
                    loginMethod: .password,
                    username: "me@example.net",
                    connectionState: .needsTest
                )
            ],
            selectedSidebarItem: .dashboard,
            generalSettings: GeneralSettings(),
            audit: [
                AuditEntry(
                    time: "10:42:18",
                    client: "Claude Desktop",
                    account: "Work",
                    event: "mail_search",
                    result: "Allowed"
                ),
                AuditEntry(
                    time: "10:43:05",
                    client: "Claude Desktop",
                    account: "Work",
                    event: "mail_prepare_send",
                    result: "Pending"
                )
            ],
            connectedClients: ["Claude Desktop"],
            news: [
                NewsItem(
                    id: "news-oauth",
                    title: "Gmail und Microsoft 365 ohne Passwort",
                    detail: "Konten lassen sich jetzt per OAuth anmelden — kein App-Passwort mehr nötig.",
                    symbol: "key.fill",
                    isNew: true
                ),
                NewsItem(
                    id: "product-whisper",
                    title: "TorroWhisper",
                    detail: "Diktieren in jedem Programm, lokal auf deinem Mac.",
                    symbol: "waveform"
                )
            ]
        )
    }
}
