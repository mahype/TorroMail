import CryptoKit
import Foundation
import Security
import SwiftUI

public enum TorroMailSidebarSelection: Hashable {
    case dashboard
    case accounts
    case mcp
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
/// resolves itself. Each item's access list names the server binary,
/// because the consent dialog macOS would otherwise show cannot appear
/// for a headless process — it answers "no UI possible" and the lookup
/// fails instead.
public enum KeychainStore {
    public static let service = "TorroMail"

    public struct Failure: Error {
        public let status: OSStatus
    }

    public static func secretReference(forAccount accountID: String) -> String {
        "keychain://\(service)/\(accountID)"
    }

    public static func savePassword(
        _ password: String,
        forAccount accountID: String,
        alsoTrusting executablePaths: [String] = []
    ) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountID
        ]
        // Delete and re-add instead of updating in place: the access list
        // is fixed at creation, and every save must bless the binaries as
        // they exist right now — an update would keep the item trusting
        // builds that are already gone.
        SecItemDelete(query as CFDictionary)

        var attributes = query
        attributes[kSecValueData as String] = Data(password.utf8)
        attributes[kSecAttrAccess as String] = try access(alsoTrusting: executablePaths)
        let addStatus = SecItemAdd(attributes as CFDictionary, nil)
        guard addStatus == errSecSuccess else { throw Failure(status: addStatus) }
    }

    /// The stored secret, or nil when it is missing or this process may not
    /// read it. Callers treat both the same way: write a fresh one.
    public static func readPassword(forAccount accountID: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountID,
            kSecReturnData as String: true
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    /// Re-writes stored secrets so their access lists bless the binaries as
    /// they are right now. Trust pins to a binary's hash while nothing is
    /// code-signed, so every rebuild orphans the existing grants; running
    /// this on launch heals them. An item this process may not read is
    /// skipped — its next regular save carries the fresh list.
    public static func refreshAccessControl(
        forAccounts accountIDs: [String],
        alsoTrusting executablePaths: [String]
    ) {
        for accountID in accountIDs {
            guard let password = readPassword(forAccount: accountID) else {
                NSLog("TorroMail: keychain item for %@ is unreadable, leaving it as is", accountID)
                continue
            }
            do {
                try savePassword(password, forAccount: accountID, alsoTrusting: executablePaths)
            } catch {
                NSLog("TorroMail: keychain access refresh for %@ failed", accountID)
            }
        }
    }

    /// An access list naming the app and the given executables. The MCP
    /// server is a separate binary, so without it on the list every lookup
    /// wants a consent dialog — and the server runs headless under an
    /// assistant, where macOS cannot show one and fails the lookup outright.
    /// These APIs are deprecated without a file-keychain replacement;
    /// keychain access groups can take over once both binaries share a
    /// code signature.
    private static func access(alsoTrusting executablePaths: [String]) throws -> SecAccess {
        var trusted: [SecTrustedApplication] = []
        var appItself: SecTrustedApplication?
        if SecTrustedApplicationCreateFromPath(nil, &appItself) == errSecSuccess,
           let appItself {
            trusted.append(appItself)
        }
        for path in executablePaths {
            var executable: SecTrustedApplication?
            if SecTrustedApplicationCreateFromPath(path, &executable) == errSecSuccess,
               let executable {
                trusted.append(executable)
            }
        }
        var created: SecAccess?
        let status = SecAccessCreate(service as CFString, trusted as CFArray, &created)
        guard status == errSecSuccess, let created else {
            throw Failure(status: status)
        }
        return created
    }

    /// Drops a secret. An abandoned setup writes a password to the keychain
    /// before it knows whether the account will exist; without this, cancelling
    /// would leave it there forever under an id nothing refers to.
    public static func deletePassword(forAccount accountID: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountID
        ]
        SecItemDelete(query as CFDictionary)
    }
}

/// The access keys that pair MCP clients with the server. Connecting a client
/// mints one; it travels as `TORROMAIL_TOKEN` in that client's own MCP config,
/// its plaintext is kept only here in the keychain (so a snippet can be shown
/// again), and the policy document publishes nothing but its SHA-256 — the
/// server checks against that allowlist on every tool call, so revoking a key
/// here locks the client out immediately.
public enum MCPClientKeyStore {
    /// Keychain account names for client keys, kept clear of mail account ids
    /// under the same service.
    private static let accountPrefix = "client-key-"

    /// The identity the app itself presents when it spawns the server —
    /// connection checks take the same gate the assistants do.
    public static let appClientID = "torromail-app"

    /// One paired client as the policy document publishes it: labels for
    /// attribution and the key's hash, never the key.
    public struct Pairing: Hashable, Sendable {
        public let clientID: String
        public let name: String
        public let tokenSHA256: String

        public init(clientID: String, name: String, tokenSHA256: String) {
            self.clientID = clientID
            self.name = name
            self.tokenSHA256 = tokenSHA256
        }
    }

    /// The stored key for a client, or nil when it was never connected.
    public static func token(forClient clientID: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: KeychainStore.service,
            kSecAttrAccount as String: accountPrefix + clientID,
            kSecReturnData as String: true
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    /// The client's key, minting one on first use. Reconnecting keeps the
    /// key stable — rotation is an explicit renewal, never a side effect.
    public static func tokenCreatingIfNeeded(forClient clientID: String) throws -> String {
        if let existing = token(forClient: clientID) {
            return existing
        }
        let minted = mintToken(forClient: clientID)
        try KeychainStore.savePassword(minted, forAccount: accountPrefix + clientID)
        return minted
    }

    /// Renewal: the old key dies with the keychain entry, the new one only
    /// starts working once the caller republishes the policy document.
    public static func renewToken(forClient clientID: String) throws -> String {
        let minted = mintToken(forClient: clientID)
        try KeychainStore.savePassword(minted, forAccount: accountPrefix + clientID)
        return minted
    }

    public static func revokeToken(forClient clientID: String) {
        KeychainStore.deletePassword(forAccount: accountPrefix + clientID)
    }

    /// Every paired client, hashes freshly computed from the stored keys —
    /// what the policy document's `clients` allowlist is built from.
    ///
    /// Enumeration fetches attributes only; each key's value is read in its
    /// own query. The service also holds the mail account passwords, and a
    /// bulk data fetch fails whole on the first item whose keychain ACL says
    /// no (one written by a previous dev build is enough) — which would
    /// publish an empty allowlist and lock every paired client out at once.
    /// An unreadable key skips only its own client instead.
    public static func pairings() -> [Pairing] {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: KeychainStore.service,
            kSecMatchLimit as String: kSecMatchLimitAll,
            kSecReturnAttributes as String: true
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let items = result as? [[String: Any]] else {
            return []
        }

        return items.compactMap { item -> Pairing? in
            guard let account = item[kSecAttrAccount as String] as? String,
                  account.hasPrefix(accountPrefix) else {
                return nil
            }
            let clientID = String(account.dropFirst(accountPrefix.count))
            guard let token = token(forClient: clientID) else {
                NSLog(
                    "TorroMail: access key for %@ is unreadable, leaving it off the allowlist",
                    clientID
                )
                return nil
            }
            return Pairing(
                clientID: clientID,
                name: displayName(forClient: clientID),
                tokenSHA256: sha256Hex(token)
            )
        }
        .sorted { $0.clientID < $1.clientID }
    }

    /// The app's own key, minted on first use — every spawn of the server by
    /// the app (connection checks, the supervised instance) presents it.
    public static func appToken() throws -> String {
        try tokenCreatingIfNeeded(forClient: appClientID)
    }

    public static func sha256Hex(_ token: String) -> String {
        SHA256.hash(data: Data(token.utf8))
            .map { String(format: "%02x", $0) }
            .joined()
    }

    /// The visible half of a key: enough to recognize it, none of the secret.
    /// Shown in snippets until the user reveals the real one.
    public static func maskedToken(forClient clientID: String) -> String {
        "torro_\(clientID)_••••••••••••"
    }

    /// `torro_<client>_<32 bytes of system randomness>` — the prefix names
    /// the client for humans reading a config; the entropy does the work.
    private static func mintToken(forClient clientID: String) -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        let status = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        precondition(status == errSecSuccess, "system randomness unavailable")
        let hex = bytes.map { String(format: "%02x", $0) }.joined()
        return "torro_\(clientID)_\(hex)"
    }

    private static func displayName(forClient clientID: String) -> String {
        if clientID == appClientID { return "TorroMail" }
        return MCPClientRegistry.descriptor(id: clientID)?.displayName ?? clientID
    }
}

/// Runs the MCP binary's `--check-account`: the same secret resolution,
/// TLS and LOGIN the tools use — so a green dot means the real path works.
public enum AccountCheck {
    /// Checks an account the app has already published. `policyURL` overrides
    /// which document to check against — the wizard points it at a throwaway
    /// document so a candidate account can be proven before it joins the list.
    public static func run(
        accountID: String,
        executableName: String,
        policyURL: URL? = nil
    ) -> ConnectionState {
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
        if let url = policyURL ?? (try? PolicyDocument.defaultURL()) {
            environment["TORROMAIL_POLICY_PATH"] = url.path
        }
        // The check logs into the real mailbox, so it sits behind the same
        // pairing gate as the tools — the app presents its own key.
        if let appToken = try? MCPClientKeyStore.appToken() {
            environment["TORROMAIL_TOKEN"] = appToken
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

/// An assistant TorroMail can wire itself into. A client is defined by where
/// it keeps its MCP servers, not by what it is called: Claude Desktop, Gemini
/// CLI and Cursor all read a JSON file with a top-level `mcpServers` object,
/// so they differ only in path.
public struct MCPClient: Identifiable, Hashable {
    public enum Setup: Hashable {
        /// A JSON file with a top-level `mcpServers` object we own and merge.
        case mcpServersJSON(configURL: URL)
        /// ChatGPT keeps its servers in TOML and bundles the codex CLI that
        /// owns that file. Handing the edit to that tool beats writing TOML
        /// here — preserving the user's other servers stays its problem. The
        /// config is read (not launched) to tell whether we are set up.
        case codexCLI(executableURL: URL, configURL: URL)
        /// Claude Code owns a large, frequently-rewritten JSON file
        /// (`~/.claude.json`). Editing it by hand would race its own writes,
        /// so we register through its `claude mcp add -s user` CLI and only
        /// read the file to detect our server.
        case claudeCodeCLI(executableURL: URL, configURL: URL)
    }

    public let id: String
    public let displayName: String
    public let setup: Setup

    public init(id: String, displayName: String, setup: Setup) {
        self.id = id
        self.displayName = displayName
        self.setup = setup
    }

    /// Where this client keeps its MCP servers — shown in the detail view so a
    /// manual edit has somewhere to go.
    public var configURL: URL {
        switch setup {
        case let .mcpServersJSON(url), let .codexCLI(_, url), let .claudeCodeCLI(_, url):
            return url
        }
    }
}

/// The clients TorroMail knows how to configure, and where each keeps its
/// servers. Only what is installed is listed: TorroMail does not offer to
/// set up an assistant that is not on this Mac.
public enum MCPClientRegistry {
    /// The name TorroMail registers itself under in every client.
    public static let serverName = "torromail"

    public static func installed(fileManager: FileManager = .default) -> [MCPClient] {
        let home = fileManager.homeDirectoryForCurrentUser
        var clients: [MCPClient] = []

        let claudeDirectory = home
            .appendingPathComponent("Library/Application Support/Claude", isDirectory: true)
        if fileManager.fileExists(atPath: claudeDirectory.path) {
            clients.append(MCPClient(
                id: "claude-desktop",
                displayName: "Claude Desktop",
                setup: .mcpServersJSON(
                    configURL: claudeDirectory.appendingPathComponent("claude_desktop_config.json")
                )
            ))
        }

        let codex = URL(fileURLWithPath: "/Applications/ChatGPT.app/Contents/Resources/codex")
        if fileManager.isExecutableFile(atPath: codex.path) {
            clients.append(MCPClient(
                id: "chatgpt",
                displayName: "ChatGPT",
                setup: .codexCLI(
                    executableURL: codex,
                    configURL: home.appendingPathComponent(".codex/config.toml")
                )
            ))
        }

        let geminiDirectory = home.appendingPathComponent(".gemini", isDirectory: true)
        if fileManager.fileExists(atPath: geminiDirectory.path) {
            clients.append(MCPClient(
                id: "gemini-cli",
                displayName: "Gemini CLI",
                setup: .mcpServersJSON(
                    configURL: geminiDirectory.appendingPathComponent("settings.json")
                )
            ))
        }

        let cursorDirectory = home.appendingPathComponent(".cursor", isDirectory: true)
        if fileManager.fileExists(atPath: cursorDirectory.path) {
            clients.append(MCPClient(
                id: "cursor",
                displayName: "Cursor",
                setup: .mcpServersJSON(
                    configURL: cursorDirectory.appendingPathComponent("mcp.json")
                )
            ))
        }

        // Claude Code is a CLI: a GUI app inherits no shell PATH, so the
        // binary is found by absolute path across the ways it ships.
        if let claude = resolveExecutable(
            candidates: [
                home.appendingPathComponent(".claude/local/claude").path,
                "/opt/homebrew/bin/claude",
                "/usr/local/bin/claude",
                home.appendingPathComponent(".local/bin/claude").path
            ],
            fileManager: fileManager
        ) {
            clients.append(MCPClient(
                id: "claude-code",
                displayName: "Claude Code",
                setup: .claudeCodeCLI(
                    executableURL: claude,
                    configURL: home.appendingPathComponent(".claude.json")
                )
            ))
        }

        return clients
    }

    /// First executable candidate that exists, or nil. Used for CLIs whose
    /// install location depends on how the user installed them.
    public static func resolveExecutable(candidates: [String], fileManager: FileManager) -> URL? {
        for path in candidates where fileManager.isExecutableFile(atPath: path) {
            return URL(fileURLWithPath: path)
        }
        return nil
    }
}

/// Proves a candidate account before it becomes one.
///
/// The setup wizard needs the real answer — TLS, login, mailboxes — from an
/// account that does not exist yet. Publishing it to the live policy document
/// first would mean an unproven account is briefly reachable by assistants,
/// so this writes a throwaway document instead and points the check at that.
public enum AccountTrial {
    /// Runs the same `--check-account` path the app's own button uses, against
    /// a document containing only this candidate.
    public static func check(
        account: MailAccount,
        executableName: String
    ) -> ConnectionState {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("torromail-trial-\(account.id).json")
        defer { try? FileManager.default.removeItem(at: url) }

        do {
            // The throwaway document admits exactly one client: the app
            // itself, whose key the check presents.
            let appToken = try MCPClientKeyStore.appToken()
            let appPairing = MCPClientKeyStore.Pairing(
                clientID: MCPClientKeyStore.appClientID,
                name: "TorroMail",
                tokenSHA256: MCPClientKeyStore.sha256Hex(appToken)
            )
            try PolicyDocument.data(for: [account], clients: [appPairing])
                .write(to: url, options: .atomic)
        } catch {
            return .failed(error.localizedDescription)
        }
        return AccountCheck.run(
            accountID: account.id,
            executableName: executableName,
            policyURL: url
        )
    }
}

/// Wiring TorroMail into MCP clients. The snippet points at the bundled
/// server binary and carries the client's access key as `TORROMAIL_TOKEN`;
/// the policy document lives at its default path, so nothing else travels.
public enum MCPClientSetup {
    public struct Failure: Error {
        public let reason: String

        public init(_ reason: String) {
            self.reason = reason
        }
    }

    /// Absolute path of the server binary for client configs. A bare PATH
    /// fallback is useless there, so only real paths count.
    public static func serverCommandPath(executableName: String) -> String? {
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        guard let command = locator.resolve(), command.displayPath.contains("/") else {
            return nil
        }
        return command.displayPath
    }

    /// What a fresh keychain item trusts besides the app: the server binary,
    /// resolved the same way client configs are — the grant has to land on
    /// the binary the assistants actually launch.
    public static func trustedExecutablePaths(executableName: String) -> [String] {
        guard let path = serverCommandPath(executableName: executableName) else {
            return []
        }
        return [path]
    }

    /// Which assistants are set up to reach TorroMail — configured *and*
    /// carrying a working key. Read from their own configuration — the app
    /// does not guess, and does not pretend: an entry whose key would be
    /// refused is not "connected".
    public static func configuredClientNames(fileManager: FileManager = .default) -> [String] {
        MCPClientRegistry.installed(fileManager: fileManager)
            .filter { isConfigured($0) && hasCurrentKey($0) }
            .map(\.displayName)
    }

    /// Whether a client already points at TorroMail.
    public static func isConfigured(_ client: MCPClient) -> Bool {
        switch client.setup {
        case let .mcpServersJSON(configURL), let .claudeCodeCLI(_, configURL):
            return jsonConfigHasServer(at: configURL)
        case let .codexCLI(_, configURL):
            // Reading the file beats launching the CLI on every refresh, and
            // a TOML table header is unambiguous enough to scan for.
            guard let text = try? String(contentsOf: configURL, encoding: .utf8) else {
                return false
            }
            return text.contains("[mcp_servers.\(MCPClientRegistry.serverName)]")
        }
    }

    /// Whether a top-level `mcpServers` object names our server. Shared by the
    /// clients whose config is JSON, including Claude Code's `~/.claude.json`.
    private static func jsonConfigHasServer(at configURL: URL) -> Bool {
        guard let data = try? Data(contentsOf: configURL),
              let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let servers = root["mcpServers"] as? [String: Any] else {
            return false
        }
        return servers[MCPClientRegistry.serverName] != nil
    }

    /// Whether the client's config carries the key the keychain holds for it
    /// — the difference between "points at TorroMail" and "will get in".
    /// False for a config written before keys existed, and after a renewal
    /// the config missed.
    public static func hasCurrentKey(_ client: MCPClient) -> Bool {
        guard let token = MCPClientKeyStore.token(forClient: client.id) else {
            return false
        }
        switch client.setup {
        case let .mcpServersJSON(configURL), let .claudeCodeCLI(_, configURL):
            return jsonConfigToken(at: configURL) == token
        case let .codexCLI(_, configURL):
            // The key is high-entropy, so plain containment on the TOML is
            // unambiguous — better than parsing a format we never write.
            guard let text = try? String(contentsOf: configURL, encoding: .utf8) else {
                return false
            }
            return text.contains(token)
        }
    }

    private static func jsonConfigToken(at configURL: URL) -> String? {
        guard let data = try? Data(contentsOf: configURL),
              let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let servers = root["mcpServers"] as? [String: Any],
              let server = servers[MCPClientRegistry.serverName] as? [String: Any],
              let env = server["env"] as? [String: Any] else {
            return nil
        }
        return env["TORROMAIL_TOKEN"] as? String
    }

    /// Launch-time self-healing: every automatic client that already points
    /// at TorroMail but carries no key — a config from before keys existed —
    /// gets its entry rewritten with one. Returns whether anything changed,
    /// so the caller knows the policy document needs republishing.
    @discardableResult
    public static func refreshManagedKeys(
        executableName: String,
        fileManager: FileManager = .default
    ) -> Bool {
        guard let commandPath = serverCommandPath(executableName: executableName) else {
            return false
        }
        var changed = false
        for descriptor in MCPClientRegistry.catalog where descriptor.kind == .automatic {
            guard
                let client = MCPClientRegistry.installedClient(
                    id: descriptor.id,
                    fileManager: fileManager
                ),
                isConfigured(client),
                !hasCurrentKey(client),
                let token = try? MCPClientKeyStore.tokenCreatingIfNeeded(forClient: descriptor.id),
                (try? add(to: client, commandPath: commandPath, token: token, fileManager: fileManager)) != nil
            else { continue }
            changed = true
        }
        return changed
    }

    /// The snippet to paste, in the shape the target client expects. Clients
    /// differ: most read a top-level `mcpServers` JSON object, Hermes reads
    /// YAML under `mcp_servers`, OpenClaw nests it under `mcp.servers`.
    /// `token` is the access key to embed — pass the masked stand-in for
    /// display and the real key for the clipboard, so the code on screen and
    /// the code that leaves it are the same text with one word swapped.
    public static func configSnippet(
        commandPath: String,
        format: MCPClientDescriptor.SnippetFormat = .mcpServersJSON,
        token: String
    ) -> String {
        let name = MCPClientRegistry.serverName
        switch format {
        case .mcpServersJSON:
            return """
            {
              "mcpServers": {
                "\(name)": {
                  "command": "\(commandPath)",
                  "env": {
                    "TORROMAIL_TOKEN": "\(token)"
                  }
                }
              }
            }
            """
        case .hermesYAML:
            return """
            mcp_servers:
              \(name):
                command: "\(commandPath)"
                env:
                  TORROMAIL_TOKEN: "\(token)"
            """
        case .openClawJSON:
            return """
            {
              "mcp": {
                "servers": {
                  "\(name)": {
                    "command": "\(commandPath)",
                    "env": {
                      "TORROMAIL_TOKEN": "\(token)"
                    }
                  }
                }
              }
            }
            """
        }
    }

    /// Registers the server with a client, access key included. Servers the
    /// user configured elsewhere survive: the JSON path merges, and the CLI
    /// paths hand the file to the tool that owns it. An unreadable
    /// configuration is an error and is never overwritten.
    public static func add(
        to client: MCPClient,
        commandPath: String,
        token: String,
        fileManager: FileManager = .default
    ) throws {
        switch client.setup {
        case let .mcpServersJSON(configURL):
            try addToJSONConfig(
                at: configURL,
                commandPath: commandPath,
                token: token,
                fileManager: fileManager
            )
        case let .codexCLI(executableURL, _):
            try addViaCLI(
                executableURL: executableURL,
                scopeArguments: [],
                environmentArguments: ["--env", "TORROMAIL_TOKEN=\(token)"],
                commandPath: commandPath
            )
        case let .claudeCodeCLI(executableURL, _):
            // `-s user` registers globally; the default scope is the current
            // project, which for a background app would be nowhere useful.
            try addViaCLI(
                executableURL: executableURL,
                scopeArguments: ["-s", "user"],
                environmentArguments: ["-e", "TORROMAIL_TOKEN=\(token)"],
                commandPath: commandPath
            )
        }
    }

    private static func addToJSONConfig(
        at target: URL,
        commandPath: String,
        token: String,
        fileManager: FileManager
    ) throws {
        var root: [String: Any] = [:]
        if let data = try? Data(contentsOf: target), !data.isEmpty {
            // A client may ship an empty placeholder file (Cursor does); that
            // is a fresh start, not a corrupt config.
            guard let existing = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
                throw Failure("The existing configuration could not be read.")
            }
            root = existing
        }
        var servers = root["mcpServers"] as? [String: Any] ?? [:]
        servers[MCPClientRegistry.serverName] = [
            "command": commandPath,
            "env": ["TORROMAIL_TOKEN": token]
        ]
        root["mcpServers"] = servers

        try fileManager.createDirectory(
            at: target.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let data = try JSONSerialization.data(withJSONObject: root, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: target, options: .atomic)
    }

    /// Runs `<cli> mcp add <name> [extraArguments] -- <commandPath>`. The tool
    /// that owns the config does the merge, so other servers are safe.
    ///
    /// An existing entry is removed first: the CLIs refuse to add a name that
    /// already exists (`claude mcp add` exits with "already exists"), and
    /// reconnecting or healing a key *is* replacing the entry. The remove may
    /// fail freely — a missing entry is exactly the state it aims for.
    private static func addViaCLI(
        executableURL: URL,
        scopeArguments: [String],
        environmentArguments: [String],
        commandPath: String
    ) throws {
        _ = try? runCLI(
            executableURL: executableURL,
            arguments: ["mcp", "remove", MCPClientRegistry.serverName] + scopeArguments
        )
        try runCLI(
            executableURL: executableURL,
            arguments: ["mcp", "add", MCPClientRegistry.serverName]
                + scopeArguments + environmentArguments + ["--", commandPath]
        )
    }

    private static func runCLI(executableURL: URL, arguments: [String]) throws {
        let process = Process()
        process.executableURL = executableURL
        process.arguments = arguments
        process.standardOutput = Pipe()
        process.standardError = Pipe()

        do {
            try process.run()
        } catch {
            throw Failure("The assistant's setup tool could not be started.")
        }
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw Failure("The assistant's setup tool reported an error.")
        }
    }
}

/// A client TorroMail knows how to wire itself into, described independently
/// of whether it is installed on this Mac. The catalog drives the MCP Server
/// list: every known assistant shows up — installed or not — so the user can
/// see the full field and pick from it, the way the account list shows the
/// accounts they added.
public struct MCPClientDescriptor: Identifiable, Hashable, Sendable {
    /// How TorroMail can set the client up.
    public enum Kind: Hashable, Sendable {
        /// TorroMail knows where the client keeps its config and can write
        /// itself in with one click, then read the file back to confirm.
        case automatic
        /// TorroMail cannot (or does not yet) edit this client's config, so it
        /// only offers the snippet to paste. Covers clients whose config path
        /// TorroMail has not learned, and the catch-all "any other client".
        case manual
    }

    /// The shape the snippet has to take for this client — they do not all read
    /// the same file format.
    public enum SnippetFormat: Hashable, Sendable {
        /// Top-level `mcpServers` JSON object (Claude Desktop, Cursor, and the
        /// generic default).
        case mcpServersJSON
        /// Hermes' `~/.hermes/config.yaml`: YAML under `mcp_servers`.
        case hermesYAML
        /// OpenClaw's `~/.openclaw/openclaw.json`: JSON nested under
        /// `mcp.servers`.
        case openClawJSON
    }

    public let id: String
    public let displayName: String
    /// SF Symbol for the client's card badge.
    public let symbol: String
    public let kind: Kind
    public let snippetFormat: SnippetFormat
    /// For a manual client whose config file TorroMail knows: where the snippet
    /// goes, shown next to it. Nil for the fully generic "other client".
    public let manualConfigPath: String?
    /// For an automatic client: how to confirm, inside the client itself, that
    /// it actually loaded TorroMail — the one step TorroMail cannot observe
    /// from the outside. For a manual client: where to paste the snippet.
    /// English key; the app localizes it.
    public let verificationHintKey: String

    public init(
        id: String,
        displayName: String,
        symbol: String,
        kind: Kind = .automatic,
        snippetFormat: SnippetFormat = .mcpServersJSON,
        manualConfigPath: String? = nil,
        verificationHintKey: String
    ) {
        self.id = id
        self.displayName = displayName
        self.symbol = symbol
        self.kind = kind
        self.snippetFormat = snippetFormat
        self.manualConfigPath = manualConfigPath
        self.verificationHintKey = verificationHintKey
    }
}

extension MCPClientRegistry {
    /// Every client TorroMail can set up, whether or not it is on this Mac.
    /// Ids match the ones `installed()` produces, so a descriptor and its
    /// resolved client line up.
    public static let catalog: [MCPClientDescriptor] = [
        MCPClientDescriptor(
            id: "claude-desktop",
            displayName: "Claude Desktop",
            symbol: "sparkles",
            verificationHintKey: "Restart Claude Desktop. A tools icon appears below the message box; TorroMail’s tools sit under “torromail”. Ask it: “List my mail accounts.”"
        ),
        MCPClientDescriptor(
            id: "claude-code",
            displayName: "Claude Code",
            symbol: "terminal",
            verificationHintKey: "In a new Claude Code session run “/mcp” — “torromail” must show as connected. Or ask it: “List my mail accounts.”"
        ),
        MCPClientDescriptor(
            id: "chatgpt",
            displayName: "ChatGPT",
            symbol: "bubble.left.and.text.bubble.right",
            verificationHintKey: "Restart the Codex CLI; “torromail” appears in its MCP list. Then ask: “List my mail accounts.”"
        ),
        MCPClientDescriptor(
            id: "gemini-cli",
            displayName: "Gemini CLI",
            symbol: "diamond",
            verificationHintKey: "Restart the Gemini CLI and run “/mcp”; “torromail” must be listed."
        ),
        MCPClientDescriptor(
            id: "cursor",
            displayName: "Cursor",
            symbol: "cursorarrow.rays",
            verificationHintKey: "Open Cursor → Settings → MCP; “torromail” must show as active. Then in chat: “List my mail accounts.”"
        ),
        MCPClientDescriptor(
            id: "clawbot",
            displayName: "Clawbot",
            symbol: "pawprint",
            kind: .manual,
            snippetFormat: .openClawJSON,
            manualConfigPath: "~/.openclaw/openclaw.json",
            verificationHintKey: "Add the snippet below under “mcp.servers” in Clawbot’s config (or run “openclaw mcp add”), then restart it and ask it to list your mail accounts."
        ),
        MCPClientDescriptor(
            id: "hermes",
            displayName: "Hermes",
            symbol: "paperplane.circle",
            kind: .manual,
            snippetFormat: .hermesYAML,
            manualConfigPath: "~/.hermes/config.yaml",
            verificationHintKey: "Add the snippet below under “mcp_servers” in Hermes’ config, then restart it and ask it to list your mail accounts."
        ),
        MCPClientDescriptor(
            id: "other",
            displayName: "Other client",
            symbol: "puzzlepiece.extension",
            kind: .manual,
            verificationHintKey: "Paste the snippet below into your client’s MCP configuration — any client that speaks MCP over stdio can run TorroMail. Restart it afterwards."
        )
    ]

    public static func descriptor(id: String) -> MCPClientDescriptor? {
        catalog.first { $0.id == id }
    }

    /// The installed, path-resolved client for a catalog id, or nil when the
    /// assistant is not on this Mac.
    public static func installedClient(id: String, fileManager: FileManager = .default) -> MCPClient? {
        installed(fileManager: fileManager).first { $0.id == id }
    }
}

/// Where a client stands relative to TorroMail: whether it is here at all,
/// whether its config points at the server, and — when it does — whether the
/// server binary actually answers. The first two are cheap file facts; the
/// third is a real launch of the bundled server.
public struct MCPClientSetupStatus: Hashable, Sendable {
    public enum Server: Hashable, Sendable {
        case unknown
        case responds(toolCount: Int)
        case failed(String)
    }

    public var isInstalled: Bool
    public var isConfigured: Bool
    /// Whether the config carries the client's current access key. A client
    /// that is configured without one points at the server but will be
    /// refused — the state the UI has to call out for re-connecting.
    public var hasCurrentKey: Bool
    public var server: Server

    public init(
        isInstalled: Bool,
        isConfigured: Bool,
        hasCurrentKey: Bool = false,
        server: Server = .unknown
    ) {
        self.isInstalled = isInstalled
        self.isConfigured = isConfigured
        self.hasCurrentKey = hasCurrentKey
        self.server = server
    }
}

/// Launches the bundled server with `--list-tools` and reads back the tool
/// catalog. A green result means the exact binary a client would spawn starts
/// and speaks — the half of "is it set up?" TorroMail can prove by itself.
public enum MCPServerSelfTest {
    public static func run(executableName: String) -> MCPClientSetupStatus.Server {
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        guard let command = locator.resolve() else {
            return .failed("MCP executable not found")
        }

        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments + ["--list-tools"]
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        process.standardOutput = outputPipe
        process.standardError = errorPipe

        do {
            try process.run()
        } catch {
            return .failed(error.localizedDescription)
        }
        let data = outputPipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()

        guard process.terminationStatus == 0 else {
            let detail = String(
                data: errorPipe.fileHandleForReading.readDataToEndOfFile(),
                encoding: .utf8
            )?.trimmingCharacters(in: .whitespacesAndNewlines)
            return .failed(detail?.isEmpty == false ? detail ?? "" : "self-test failed")
        }
        // `--list-tools` prints the MCP tools document: `{"tools":[…]}`.
        guard
            let root = try? JSONSerialization.jsonObject(with: data),
            let tools = (root as? [String: Any])?["tools"] as? [Any]
        else {
            return .failed("unexpected server output")
        }
        return .responds(toolCount: tools.count)
    }
}

extension MCPClientSetup {
    /// The full standing of one catalog client, ready for the detail view. The
    /// server self-test only runs when the client is actually configured —
    /// there is nothing to confirm on the wire before then.
    public static func status(
        for descriptor: MCPClientDescriptor,
        executableName: String,
        runServerTest: Bool = true,
        fileManager: FileManager = .default
    ) -> MCPClientSetupStatus {
        guard let client = MCPClientRegistry.installedClient(id: descriptor.id, fileManager: fileManager) else {
            return MCPClientSetupStatus(isInstalled: false, isConfigured: false)
        }
        let configured = isConfigured(client)
        let keyed = configured && hasCurrentKey(client)
        guard configured, runServerTest else {
            return MCPClientSetupStatus(
                isInstalled: true,
                isConfigured: configured,
                hasCurrentKey: keyed
            )
        }
        let server = MCPServerSelfTest.run(executableName: executableName)
        return MCPClientSetupStatus(
            isInstalled: true,
            isConfigured: configured,
            hasCurrentKey: keyed,
            server: server
        )
    }

    /// Removes TorroMail from a client's configuration — the counterpart to
    /// `add`. JSON configs are edited in place; the CLI-owned configs hand the
    /// removal back to the tool that owns them.
    public static func remove(
        from client: MCPClient,
        fileManager: FileManager = .default
    ) throws {
        switch client.setup {
        case let .mcpServersJSON(configURL):
            try removeFromJSONConfig(at: configURL, fileManager: fileManager)
        case let .codexCLI(executableURL, _):
            try removeViaCLI(executableURL: executableURL, extraArguments: [])
        case let .claudeCodeCLI(executableURL, _):
            try removeViaCLI(executableURL: executableURL, extraArguments: ["-s", "user"])
        }
    }

    private static func removeFromJSONConfig(at target: URL, fileManager: FileManager) throws {
        guard let data = try? Data(contentsOf: target), !data.isEmpty else { return }
        guard var root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw Failure("The existing configuration could not be read.")
        }
        guard var servers = root["mcpServers"] as? [String: Any] else { return }
        servers[MCPClientRegistry.serverName] = nil
        root["mcpServers"] = servers
        let updated = try JSONSerialization.data(withJSONObject: root, options: [.prettyPrinted, .sortedKeys])
        try updated.write(to: target, options: .atomic)
    }

    private static func removeViaCLI(executableURL: URL, extraArguments: [String]) throws {
        try runCLI(
            executableURL: executableURL,
            arguments: ["mcp", "remove", MCPClientRegistry.serverName] + extraArguments
        )
    }
}

/// What TorroMail remembers between launches: the accounts you configured
/// and where the app shows up. Passwords stay in the keychain; connection
/// state and pending approvals are runtime facts and start fresh.
public enum AppStateStore {
    public static let version = 1

    public struct State: Codable, Hashable {
        public var version: Int
        public var accounts: [MailAccount]
        public var settings: GeneralSettings

        public init(
            version: Int = AppStateStore.version,
            accounts: [MailAccount] = [],
            settings: GeneralSettings = GeneralSettings()
        ) {
            self.version = version
            self.accounts = accounts
            self.settings = settings
        }
    }

    /// `~/Library/Application Support/TorroMail/state.json` — next to the
    /// policy document the MCP server reads.
    public static func defaultURL(fileManager: FileManager = .default) throws -> URL {
        try fileManager
            .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("TorroMail", isDirectory: true)
            .appendingPathComponent("state.json")
    }

    public static func encode(_ state: State) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return try encoder.encode(state)
    }

    public static func decode(_ data: Data) throws -> State {
        try JSONDecoder().decode(State.self, from: data)
    }

    /// Never throws: a first launch has no file, and losing the window over
    /// a read error would help nobody. A file that exists but cannot be
    /// decoded is moved aside rather than silently overwritten later.
    public static func load(from url: URL? = nil, fileManager: FileManager = .default) -> State {
        guard let target = try? url ?? defaultURL(fileManager: fileManager),
              let data = try? Data(contentsOf: target) else {
            return State()
        }
        do {
            return try decode(data)
        } catch {
            let broken = target.appendingPathExtension("broken")
            try? fileManager.removeItem(at: broken)
            try? fileManager.moveItem(at: target, to: broken)
            NSLog("TorroMail: unreadable state file moved to %@", broken.path)
            return State()
        }
    }

    /// Writes atomically — a crash mid-save must not cost the accounts.
    public static func save(
        _ state: State,
        to url: URL? = nil,
        fileManager: FileManager = .default
    ) throws {
        let target = try url ?? defaultURL(fileManager: fileManager)
        try fileManager.createDirectory(
            at: target.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try encode(state).write(to: target, options: .atomic)
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

    /// `clients` is deliberately not defaulted: the allowlist is what stands
    /// between the accounts and any process that spawns the server, so every
    /// caller has to say who is allowed — an empty list means "nobody yet".
    public static func data(
        for accounts: [MailAccount],
        clients: [MCPClientKeyStore.Pairing]
    ) throws -> Data {
        let document: [String: Any] = [
            "version": version,
            "accounts": accounts.map(accountObject(for:)),
            "clients": clients.map { pairing in
                [
                    "id": pairing.clientID,
                    "name": pairing.name,
                    "token_sha256": pairing.tokenSHA256
                ]
            }
        ]
        return try JSONSerialization.data(withJSONObject: document, options: [.sortedKeys])
    }

    /// Writes atomically so a reloading server never sees a half document.
    public static func publish(
        accounts: [MailAccount],
        clients: [MCPClientKeyStore.Pairing],
        to url: URL? = nil,
        fileManager: FileManager = .default
    ) throws {
        let target = try url ?? defaultURL(fileManager: fileManager)
        try fileManager.createDirectory(
            at: target.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try data(for: accounts, clients: clients).write(to: target, options: .atomic)
        // Connection facts and the allowlist are nobody else's read: the
        // document stays owner-only, like the configs that carry the keys.
        try fileManager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: target.path)
    }

    private static func accountObject(for account: MailAccount) -> [String: Any] {
        var object: [String: Any] = [
            "id": account.id,
            // Labels, not rights: `mail_list_accounts` has to name the
            // account the way the user does, or nobody can pick one.
            "name": account.name,
            "email": account.email,
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
            },
            // The server reports these rather than acting on them, so an
            // assistant can say why a body is missing or a search is slow.
            "cache": [
                "local_cache_enabled": account.searchCache.localCacheEnabled,
                "mode": account.searchCache.cacheMode.rawValue,
                "index_bodies": account.searchCache.indexBodies,
                "index_attachments": account.searchCache.indexAttachments,
                "storage": account.searchCache.storage
            ]
        ]
        // Connection facts travel once the account has them, whatever the
        // login method — the secret itself stays in the keychain, only the
        // reference moves. An account without this block has no mail to give:
        // the server refuses it rather than inventing any.
        if !account.imapHost.isEmpty, !account.username.isEmpty {
            var imap: [String: Any] = [
                "host": account.imapHost,
                "port": account.imapPort,
                "username": account.username,
                "secret_ref": KeychainStore.secretReference(forAccount: account.id)
            ]
            // OAuth accounts keep a token set behind that reference instead of
            // a password. The server renews it on its own, which is why the
            // endpoint and client id have to travel with the facts: an MCP
            // client can spawn the server while TorroMail is closed.
            if account.loginMethod == .oauth, let issuer = account.oauthIssuer {
                imap["auth"] = "xoauth2"
                imap["token_endpoint"] = issuer.tokenEndpoint.absoluteString
                imap["client_id"] = issuer.clientID
            }
            object["imap"] = imap
        }
        // Submission facts travel the same way, so mail_prepare_send can reach
        // the outgoing server. Port 465 is implicit TLS; 587 gets STARTTLS.
        if !account.smtpHost.isEmpty, !account.username.isEmpty {
            var smtp: [String: Any] = [
                "host": account.smtpHost,
                "port": account.smtpPort,
                "username": account.username,
                "secret_ref": KeychainStore.secretReference(forAccount: account.id)
            ]
            if account.loginMethod == .oauth, let issuer = account.oauthIssuer {
                smtp["auth"] = "xoauth2"
                smtp["token_endpoint"] = issuer.tokenEndpoint.absoluteString
                smtp["client_id"] = issuer.clientID
            }
            object["smtp"] = smtp
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

public enum Provider: String, CaseIterable, Identifiable, Hashable, Sendable, Codable {
    case imapSmtp = "IMAP/SMTP"
    case gmail = "Gmail"
    case microsoft = "Microsoft 365"
    case jmap = "JMAP"

    public var id: Self { self }
}

public enum LoginMethod: String, CaseIterable, Identifiable, Hashable, Sendable, Codable {
    case password
    case oauth

    public var id: Self { self }
}

public enum CacheMode: String, CaseIterable, Identifiable, Hashable, Sendable, Codable {
    case metadata
    case headers
    case body
    case fullText

    public var id: Self { self }
}

/// Connection health of one account. The UI stays quiet while everything is
/// fine and only surfaces states that need the user's attention.
public enum ConnectionState: Hashable, Sendable {
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
public enum ReadAccess: Int, CaseIterable, Identifiable, Hashable, Comparable, Sendable, Codable {
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
public struct WriteAccess: Hashable, Sendable, Codable {
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
public struct FolderRule: Hashable, Sendable, Codable {
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
public struct PermissionSet: Hashable, Sendable, Codable {
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

public struct SearchCacheSettings: Hashable, Sendable, Codable {
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

public struct PendingAction: Identifiable, Hashable, Sendable {
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

public struct MailAccount: Identifiable, Hashable, Sendable {
    public var id: String
    public var name: String
    public var email: String
    public var provider: Provider
    public var loginMethod: LoginMethod
    /// Who issued the token behind this account's keychain entry. Only set
    /// for OAuth accounts, and the reason the policy document can name a
    /// token endpoint the server renews against.
    public var oauthIssuer: OAuthIssuer?
    public var imapHost: String
    /// Autodiscovery fills these in; the wizard only shows them when the user
    /// opens the manual details. 993/587 are the defaults, not a rule —
    /// providers do differ.
    public var imapPort: Int
    public var smtpHost: String
    public var smtpPort: Int
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
        oauthIssuer: OAuthIssuer? = nil,
        imapHost: String = "",
        imapPort: Int = 993,
        smtpHost: String = "",
        smtpPort: Int = 587,
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
        self.oauthIssuer = oauthIssuer
        self.imapHost = imapHost
        self.imapPort = imapPort
        self.smtpHost = smtpHost
        self.smtpPort = smtpPort
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

/// What survives a launch, stated explicitly. The password is not here — it
/// lives in the keychain. Neither are connection state and pending
/// approvals: both are facts about right now, so they start fresh, and a
/// verified account is remembered as verified.
extension MailAccount: Codable {
    private enum CodingKeys: String, CodingKey {
        case id, name, email, provider, loginMethod, imapHost, smtpHost
        case username, knownMailboxes, permissions, searchCache, isVerified
        case imapPort, smtpPort, oauthIssuer
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let username = try container.decode(String.self, forKey: .username)
        let imapHost = try container.decode(String.self, forKey: .imapHost)
        let isVerified = try container.decodeIfPresent(Bool.self, forKey: .isVerified) ?? false

        self.init(
            id: try container.decode(String.self, forKey: .id),
            name: try container.decode(String.self, forKey: .name),
            email: try container.decode(String.self, forKey: .email),
            provider: try container.decode(Provider.self, forKey: .provider),
            loginMethod: try container.decode(LoginMethod.self, forKey: .loginMethod),
            oauthIssuer: try container.decodeIfPresent(OAuthIssuer.self, forKey: .oauthIssuer),
            imapHost: imapHost,
            // Accounts written before ports were configurable carry neither
            // key; they were all implicitly 993/587.
            imapPort: try container.decodeIfPresent(Int.self, forKey: .imapPort) ?? 993,
            smtpHost: try container.decode(String.self, forKey: .smtpHost),
            smtpPort: try container.decodeIfPresent(Int.self, forKey: .smtpPort) ?? 587,
            username: username,
            connectionState: MailAccount.restoredState(
                isVerified: isVerified,
                username: username,
                imapHost: imapHost
            ),
            knownMailboxes: try container.decode([String].self, forKey: .knownMailboxes),
            permissions: try container.decode(PermissionSet.self, forKey: .permissions),
            searchCache: try container.decode(SearchCacheSettings.self, forKey: .searchCache)
        )
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(name, forKey: .name)
        try container.encode(email, forKey: .email)
        try container.encode(provider, forKey: .provider)
        try container.encode(loginMethod, forKey: .loginMethod)
        try container.encodeIfPresent(oauthIssuer, forKey: .oauthIssuer)
        try container.encode(imapHost, forKey: .imapHost)
        try container.encode(imapPort, forKey: .imapPort)
        try container.encode(smtpHost, forKey: .smtpHost)
        try container.encode(smtpPort, forKey: .smtpPort)
        try container.encode(username, forKey: .username)
        try container.encode(knownMailboxes, forKey: .knownMailboxes)
        try container.encode(permissions, forKey: .permissions)
        try container.encode(searchCache, forKey: .searchCache)
        try container.encode(connectionState == .connected, forKey: .isVerified)
    }

    /// A previously verified account keeps its green dot; anything with
    /// credentials to try asks for a test; an empty shell says so.
    private static func restoredState(
        isVerified: Bool,
        username: String,
        imapHost: String
    ) -> ConnectionState {
        if isVerified { return .connected }
        return username.isEmpty && imapHost.isEmpty ? .notConfigured : .needsTest
    }
}

public struct GeneralSettings: Hashable, Codable {
    /// The one lifecycle decision the user makes: TorroMail (and with it the
    /// MCP server) starts automatically at login. Everything else is derived.
    public var launchAtLogin: Bool

    /// Whether TorroMail keeps a Dock tile while it has no window open. On by
    /// default: with it off and no window open the app is unreachable, so the
    /// Dock tile is the way back in until the user opts into the menu bar.
    public var showDockIcon: Bool

    /// Whether TorroMail sits in the menu bar. Off by default for the same
    /// reason — presence is something the user opts into.
    public var showMenuBarIcon: Bool

    /// Internal: which executable the supervisor launches. Never shown in the
    /// UI — diagnostics belong in the log.
    public var mcpExecutable: String

    public init(
        launchAtLogin: Bool = true,
        showDockIcon: Bool = true,
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
        // The app ships its own server: the bundled copy wins, so nothing
        // depends on the working directory or a shell PATH.
        if let bundled = Bundle.main.url(forAuxiliaryExecutable: executableName),
           fileManager.isExecutableFile(atPath: bundled.path) {
            return MCPLaunchCommand(
                executableURL: bundled,
                arguments: [],
                displayPath: bundled.path
            )
        }

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
        // The app's own instance is a paired client like any other.
        if let appToken = try? MCPClientKeyStore.appToken() {
            environment["TORROMAIL_TOKEN"] = appToken
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
    /// Drill-down inside the MCP destination. Empty shows the client list; one
    /// entry (a client id) shows that client's setup.
    @Published public var mcpPath: [String]
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
        mcpPath: [String] = [],
        showAccountWizard: Bool = false,
        generalSettings: GeneralSettings,
        audit: [AuditEntry],
        connectedClients: [String] = [],
        news: [NewsItem] = []
    ) {
        self.accounts = accounts
        self.selectedSidebarItem = selectedSidebarItem
        self.accountPath = accountPath
        self.mcpPath = mcpPath
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

    public func openClient(id: String) {
        selectedSidebarItem = .mcp
        mcpPath = [id]
    }

    public func beginAccountWizard() {
        showAccountWizard = true
    }

    /// Takes an account the wizard has already proven: connected, with rights
    /// chosen. Nothing half-configured reaches this list.
    public func addAccount(_ account: MailAccount) {
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
    /// The real app: your accounts and settings as you left them. Empty on
    /// first launch — that is what the dashboard's getting-started card is
    /// for. Demo accounts live in `preview()` and never reach the app.
    public static func stored() -> TorroMailModel {
        let state = AppStateStore.load()
        return TorroMailModel(
            accounts: state.accounts,
            selectedSidebarItem: .dashboard,
            generalSettings: state.settings,
            // Audit entries need real logging; until then the log is honest
            // about being empty.
            audit: [],
            connectedClients: MCPClientSetup.configuredClientNames(),
            news: releaseNotes()
        )
    }

    /// Local notes from Torro — not account data, so they are not stored.
    static func releaseNotes() -> [NewsItem] {
        [
            NewsItem(
                id: "news-oauth",
                title: "Gmail und Microsoft 365 ohne Passwort",
                detail: "Die Einrichtung findet den Anbieter selbst und meldet dich per OAuth direkt bei ihm an.",
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
    }

    /// Demo data for the contract test and SwiftUI previews only.
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
