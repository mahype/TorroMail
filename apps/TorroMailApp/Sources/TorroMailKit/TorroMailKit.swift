import Foundation
import SwiftUI

public enum TorroMailSidebarSelection: Hashable {
    case account(String)
    case settings
    case log
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
    public var selectedMailboxes: Set<String>
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
        selectedMailboxes: Set<String> = ["INBOX"],
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
        self.selectedMailboxes = selectedMailboxes
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

    /// Internal: which executable the supervisor launches. Never shown in the
    /// UI — diagnostics belong in the log.
    public var mcpExecutable: String

    public init(launchAtLogin: Bool = true, mcpExecutable: String = "torromail-mcp") {
        self.launchAtLogin = launchAtLogin
        self.mcpExecutable = mcpExecutable
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
    @Published public var showAccountWizard: Bool
    @Published public var generalSettings: GeneralSettings
    @Published public var audit: [AuditEntry]

    public init(
        accounts: [MailAccount],
        selectedSidebarItem: TorroMailSidebarSelection,
        showAccountWizard: Bool = false,
        generalSettings: GeneralSettings,
        audit: [AuditEntry]
    ) {
        self.accounts = accounts
        self.selectedSidebarItem = selectedSidebarItem
        self.showAccountWizard = showAccountWizard
        self.generalSettings = generalSettings
        self.audit = audit
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
        selectedSidebarItem = .account(account.id)
    }

    public func removeAccount(id: String) {
        accounts.removeAll { $0.id == id }
        if case let .account(selectedID) = selectedSidebarItem, selectedID == id {
            if let first = accounts.first {
                selectedSidebarItem = .account(first.id)
            } else {
                selectedSidebarItem = .settings
            }
        }
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
                    selectedMailboxes: ["INBOX", "Archive", "Sent"],
                    permissions: PermissionSet(
                        readBody: true,
                        drafts: true,
                        mark: true,
                        move: true
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
            selectedSidebarItem: .account("work"),
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
