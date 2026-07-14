import Foundation
import SwiftUI

public enum TorroMailSidebarSelection: Hashable {
    case account(String)
    case aiClients
    case generalSettings
    case auditLog
    case diagnostics
}

public struct TorroMailSidebarItem: Identifiable, Hashable {
    public var id: String
    public var label: String
    public var symbol: String
    public var selection: TorroMailSidebarSelection

    public init(
        id: String,
        label: String,
        symbol: String,
        selection: TorroMailSidebarSelection
    ) {
        self.id = id
        self.label = label
        self.symbol = symbol
        self.selection = selection
    }
}

public struct TorroMailSidebarGroup: Identifiable, Hashable {
    public var id: String
    public var label: String
    public var items: [TorroMailSidebarItem]

    public init(id: String, label: String, items: [TorroMailSidebarItem]) {
        self.id = id
        self.label = label
        self.items = items
    }
}

public enum TorroMailAccountSection: String, CaseIterable, Identifiable {
    case overview
    case connection
    case permissions
    case searchCache
    case mailboxes
    case pendingActions
    case advanced

    public var id: Self { self }

    public var label: String {
        switch self {
        case .overview: "Overview"
        case .connection: "Connection"
        case .permissions: "Permissions"
        case .searchCache: "Search & Cache"
        case .mailboxes: "Mailboxes"
        case .pendingActions: "Pending Actions"
        case .advanced: "Advanced"
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

public enum LoginMethod: String, CaseIterable, Identifiable, Hashable {
    case password = "Password"
    case oauth = "OAuth"

    public var id: Self { self }
}

public enum CacheMode: String, CaseIterable, Identifiable, Hashable {
    case metadata = "Metadata"
    case headers = "Headers"
    case body = "Body Cache"
    case fullText = "Full Text"

    public var id: Self { self }
}

public struct PermissionSet: Hashable {
    public var readHeaders: Bool
    public var readBody: Bool
    public var attachments: Bool
    public var drafts: Bool
    public var send: Bool
    public var mark: Bool
    public var move: Bool
    public var delete: Bool
    public var permanentDelete: Bool

    public init(
        readHeaders: Bool = true,
        readBody: Bool = false,
        attachments: Bool = false,
        drafts: Bool = false,
        send: Bool = false,
        mark: Bool = false,
        move: Bool = false,
        delete: Bool = false,
        permanentDelete: Bool = false
    ) {
        self.readHeaders = readHeaders
        self.readBody = readBody
        self.attachments = attachments
        self.drafts = drafts
        self.send = send
        self.mark = mark
        self.move = move
        self.delete = delete
        self.permanentDelete = permanentDelete
    }

    public var summary: String {
        let enabled = [
            readBody ? "Read body" : nil,
            attachments ? "Attachments" : nil,
            drafts ? "Drafts" : nil,
            send ? "Send" : nil,
            mark ? "Mark" : nil,
            move ? "Move" : nil,
            delete ? "Delete" : nil,
            permanentDelete ? "Permanent delete" : nil
        ].compactMap { $0 }

        return enabled.isEmpty ? "Headers only" : enabled.joined(separator: ", ")
    }
}

public struct SearchCacheSettings: Hashable {
    public var localCacheEnabled: Bool
    public var cacheMode: CacheMode
    public var indexBodies: Bool
    public var indexAttachments: Bool
    public var storage: String
    public var lastSync: String

    public init(
        localCacheEnabled: Bool = true,
        cacheMode: CacheMode = .metadata,
        indexBodies: Bool = false,
        indexAttachments: Bool = false,
        storage: String = "0 MB",
        lastSync: String = "Never"
    ) {
        self.localCacheEnabled = localCacheEnabled
        self.cacheMode = cacheMode
        self.indexBodies = indexBodies
        self.indexAttachments = indexAttachments
        self.storage = storage
        self.lastSync = lastSync
    }

    public var summary: String {
        let cache = switch cacheMode {
        case .metadata: "Metadata cached"
        case .headers: "Headers cached"
        case .body: "Bodies cached"
        case .fullText: "Full-text cache"
        }
        return "\(cache), body index \(indexBodies ? "on" : "off")"
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
    public var imapStatus: String
    public var smtpStatus: String
    public var oauthStatus: String
    public var selectedMailboxes: Set<String>
    public var permissions: PermissionSet
    public var searchCache: SearchCacheSettings
    public var pendingActions: [PendingAction]
    public var notes: String

    public init(
        id: String,
        name: String,
        email: String,
        provider: Provider,
        loginMethod: LoginMethod,
        imapStatus: String,
        smtpStatus: String,
        oauthStatus: String,
        selectedMailboxes: Set<String>,
        permissions: PermissionSet,
        searchCache: SearchCacheSettings,
        pendingActions: [PendingAction] = [],
        notes: String = ""
    ) {
        self.id = id
        self.name = name
        self.email = email
        self.provider = provider
        self.loginMethod = loginMethod
        self.imapStatus = imapStatus
        self.smtpStatus = smtpStatus
        self.oauthStatus = oauthStatus
        self.selectedMailboxes = selectedMailboxes
        self.permissions = permissions
        self.searchCache = searchCache
        self.pendingActions = pendingActions
        self.notes = notes
    }
}

public struct AIClient: Identifiable, Hashable {
    public var id: String
    public var name: String
    public var status: String
    public var approvalProfile: String

    public init(id: String, name: String, status: String, approvalProfile: String) {
        self.id = id
        self.name = name
        self.status = status
        self.approvalProfile = approvalProfile
    }
}

public struct GeneralSettings: Hashable {
    public var startMcpServerWithApp: Bool
    public var transport: String
    public var executable: String

    public init(
        startMcpServerWithApp: Bool = true,
        transport: String = "stdio",
        executable: String = "torromail-mcp"
    ) {
        self.startMcpServerWithApp = startMcpServerWithApp
        self.transport = transport
        self.executable = executable
    }

    public var mcpLifecycleSummary: String {
        startMcpServerWithApp ? "Starts with TorroMail" : "Manual start"
    }
}

public enum MCPServerStatus: Hashable {
    case stopped
    case starting
    case running(String)
    case notFound(String)
    case failed(String)

    public var label: String {
        switch self {
        case .stopped: "Stopped"
        case .starting: "Starting"
        case .running: "Running"
        case .notFound: "Executable not found"
        case .failed: "Failed"
        }
    }

    public var detail: String {
        switch self {
        case .stopped: "TorroMail is not supervising an MCP process."
        case .starting: "TorroMail is starting the local MCP process."
        case let .running(path): path
        case let .notFound(name): name
        case let .failed(message): message
        }
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

    public func startIfNeeded(settings: GeneralSettings) {
        guard settings.startMcpServerWithApp else {
            stop()
            return
        }

        if process?.isRunning == true {
            return
        }

        status = .starting
        let locator = MCPExecutableLocator(
            executableName: settings.executable,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )

        guard let command = locator.resolve() else {
            status = .notFound(settings.executable)
            return
        }

        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments
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
    @Published public var selectedAccountSection: TorroMailAccountSection
    @Published public var showAccountWizard: Bool
    @Published public var aiClients: [AIClient]
    @Published public var generalSettings: GeneralSettings
    @Published public var audit: [AuditEntry]

    public init(
        accounts: [MailAccount],
        selectedSidebarItem: TorroMailSidebarSelection,
        selectedAccountSection: TorroMailAccountSection = .overview,
        showAccountWizard: Bool = false,
        aiClients: [AIClient],
        generalSettings: GeneralSettings,
        audit: [AuditEntry]
    ) {
        self.accounts = accounts
        self.selectedSidebarItem = selectedSidebarItem
        self.selectedAccountSection = selectedAccountSection
        self.showAccountWizard = showAccountWizard
        self.aiClients = aiClients
        self.generalSettings = generalSettings
        self.audit = audit
    }

    public var sidebarItems: [TorroMailSidebarItem] {
        sidebarGroups.flatMap(\.items)
    }

    public var sidebarGroups: [TorroMailSidebarGroup] {
        let accountItems = accounts.map { account in
            TorroMailSidebarItem(
                id: "account-\(account.id)",
                label: account.name,
                symbol: "envelope",
                selection: .account(account.id)
            )
        }
        let generalItems = [
            TorroMailSidebarItem(id: "ai-clients", label: "AI Clients", symbol: "person.2.badge.gearshape", selection: .aiClients),
            TorroMailSidebarItem(id: "general-settings", label: "General Settings", symbol: "gearshape", selection: .generalSettings),
            TorroMailSidebarItem(id: "audit-log", label: "Audit Log", symbol: "list.bullet.rectangle", selection: .auditLog),
            TorroMailSidebarItem(id: "diagnostics", label: "Diagnostics", symbol: "waveform.path.ecg", selection: .diagnostics)
        ]

        return [
            TorroMailSidebarGroup(id: "accounts", label: "Accounts", items: accountItems),
            TorroMailSidebarGroup(id: "general", label: "General", items: generalItems)
        ]
    }

    public var selectedAccount: MailAccount {
        guard case let .account(accountID) = selectedSidebarItem,
              let account = accounts.first(where: { $0.id == accountID })
        else {
            return accounts[0]
        }
        return account
    }

    public func bindingForSelectedAccount() -> Binding<MailAccount> {
        Binding(
            get: { self.selectedAccount },
            set: { updated in
                if let index = self.accounts.firstIndex(where: { $0.id == updated.id }) {
                    self.accounts[index] = updated
                }
            }
        )
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
            imapStatus: "Needs Test",
            smtpStatus: "Needs Test",
            oauthStatus: loginMethod == .oauth ? "Pending" : "Off",
            selectedMailboxes: ["INBOX"],
            permissions: PermissionSet(),
            searchCache: SearchCacheSettings(),
            pendingActions: []
        )
        accounts.append(account)
        selectedSidebarItem = .account(account.id)
        selectedAccountSection = .overview
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
                    imapStatus: "Connected",
                    smtpStatus: "Connected",
                    oauthStatus: "Keychain",
                    selectedMailboxes: ["INBOX", "Archive", "Sent"],
                    permissions: PermissionSet(
                        readBody: true,
                        drafts: true,
                        mark: true,
                        move: true
                    ),
                    searchCache: SearchCacheSettings(
                        cacheMode: .headers,
                        storage: "42 MB",
                        lastSync: "Today 10:41"
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
                    imapStatus: "Needs Test",
                    smtpStatus: "Not Configured",
                    oauthStatus: "Off",
                    selectedMailboxes: ["INBOX"],
                    permissions: PermissionSet(),
                    searchCache: SearchCacheSettings()
                )
            ],
            selectedSidebarItem: .account("work"),
            aiClients: [
                AIClient(
                    id: "claude",
                    name: "Claude Desktop",
                    status: "Configured",
                    approvalProfile: "Confirm risky actions"
                ),
                AIClient(
                    id: "codex",
                    name: "Codex",
                    status: "Available",
                    approvalProfile: "Read-only until approved"
                )
            ],
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
            ]
        )
    }
}
