import CryptoKit
import Foundation
import Security
import SwiftUI

public enum TorroMailSidebarSelection: Hashable {
    case dashboard
    case accounts
    case mcp
    case settings
    case updates
    case log
    case help
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
/// resolves itself. Each item's access list is scoped to TorroMail's signing
/// Team ID rather than to a particular binary: the app and the bundled server
/// share one team, so both read the secret without a consent dialog — which
/// matters because the server runs headless under an assistant and could
/// never answer one — and the grant outlives a rebuild, because a Team ID
/// does not change when a binary's hash does.
public enum KeychainStore {
    public static let service = "TorroMail"

    public struct Failure: Error {
        public let status: OSStatus
    }

    public static func secretReference(forAccount accountID: String) -> String {
        "keychain://\(service)/\(accountID)"
    }

    /// Silences the keychain consent dialog for the whole process. TorroMail
    /// reaches its own items by Team ID and never needs to ask the user; a read
    /// that cannot be satisfied that way — a legacy item an old build wrote with
    /// a binary-pinned list — should fail quietly and be skipped or re-minted,
    /// not put a keychain-password dialog in front of someone who did nothing to
    /// invite it. Process-wide and thread-global, so one call at launch covers
    /// the background enumerations too. Call before any keychain access.
    public static func silenceInteractivePrompts() {
        SecKeychainSetUserInteractionAllowed(false)
    }

    /// The four item operations `savePassword` needs. Injectable so the repair
    /// path below — an item that is there but cannot be read — can be checked
    /// without a real keychain, which no test may write to.
    public struct ItemStore: Sendable {
        /// Creates the item with a current access list. False when one is
        /// already there; the caller then decides whether to refresh or
        /// replace it.
        public var add: @Sendable (_ account: String, _ secret: String) throws -> Bool
        public var read: @Sendable (_ account: String) -> String?
        /// Whether an item is there at all, asked by attributes alone. The
        /// access list gates the value, not the attributes, so this still
        /// answers for an item nothing can decrypt any more.
        public var exists: @Sendable (_ account: String) -> Bool
        public var update: @Sendable (_ account: String, _ secret: String) throws -> Void
        public var delete: @Sendable (_ account: String) -> Void

        public init(
            add: @escaping @Sendable (_ account: String, _ secret: String) throws -> Bool,
            read: @escaping @Sendable (_ account: String) -> String?,
            exists: @escaping @Sendable (_ account: String) -> Bool,
            update: @escaping @Sendable (_ account: String, _ secret: String) throws -> Void,
            delete: @escaping @Sendable (_ account: String) -> Void
        ) {
            self.add = add
            self.read = read
            self.exists = exists
            self.update = update
            self.delete = delete
        }

        public static let keychain = ItemStore(
            add: { account, secret in
                var attributes = KeychainStore.itemQuery(forAccount: account)
                attributes[kSecValueData as String] = Data(secret.utf8)
                // A fresh item is scoped to the signing Team ID, so the app and
                // the bundled server both read it without a dialog. An unsigned
                // local build has no team to scope to and stores it with the
                // keychain's default.
                if let access = KeychainStore.teamScopedAccess() {
                    attributes[kSecAttrAccess as String] = access
                }
                let status = SecItemAdd(attributes as CFDictionary, nil)
                if status == errSecSuccess { return true }
                guard status == errSecDuplicateItem else { throw Failure(status: status) }
                return false
            },
            read: { KeychainStore.readPassword(forAccount: $0) },
            exists: {
                var query = KeychainStore.itemQuery(forAccount: $0)
                query[kSecReturnAttributes as String] = true
                var result: CFTypeRef?
                return SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess
            },
            update: { account, secret in
                let status = SecItemUpdate(
                    KeychainStore.itemQuery(forAccount: account) as CFDictionary,
                    [kSecValueData as String: Data(secret.utf8)] as CFDictionary
                )
                guard status == errSecSuccess else { throw Failure(status: status) }
            },
            delete: { KeychainStore.deletePassword(forAccount: $0) }
        )
    }

    public static func savePassword(
        _ password: String,
        forAccount accountID: String,
        store: ItemStore = .keychain
    ) throws {
        if try store.add(accountID, password) {
            return
        }
        // The item is already there. While this build can still read it, the
        // value is refreshed in place and the item's access list is left
        // alone: that list is what lets the bundled server in, and rewriting
        // it buys nothing.
        //
        // An item it cannot read is a different animal — a legacy one, whose
        // list names a build that no longer exists. Its secret is unreachable
        // for good, which is exactly why replacing it loses nothing: there is
        // no value left to protect, and the caller is holding the one that
        // takes its place. Only a fresh add attaches a current list, so
        // without this the account could never be repaired from inside the
        // app, however often the password was typed in again.
        if store.read(accountID) == nil {
            store.delete(accountID)
            guard try store.add(accountID, password) else {
                throw Failure(status: errSecDuplicateItem)
            }
            return
        }
        try store.update(accountID, password)
    }

    /// Whether this account has a password on file. Deliberately not "can it
    /// be read": a legacy item holds one that nothing can decrypt any more,
    /// and those are precisely the accounts waiting to be repaired. Telling
    /// their owner the field is empty would read as TorroMail having thrown
    /// the password away.
    public static func hasPassword(
        forAccount accountID: String,
        store: ItemStore = .keychain
    ) -> Bool {
        store.exists(accountID)
    }

    fileprivate static func itemQuery(forAccount accountID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: accountID
        ]
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

    /// An access list that grants read and write to any binary signed with
    /// TorroMail's Team ID and nothing else. The application list is left empty
    /// ("any application"); the partition list — the gate macOS actually
    /// enforces — is pinned to this one team. Both the app and the bundled
    /// server carry that team, so both get in without a dialog, and a rebuild
    /// keeps working because the team outlives any single binary hash.
    ///
    /// Returns nil for an unsigned local build, which has no team to scope to.
    /// The SecAccess/partition APIs are deprecated but remain the only way to
    /// do this on the file keychain without a provisioning profile — which the
    /// bundled server, a bare executable the assistants launch directly, cannot
    /// carry, and which the data-protection keychain would otherwise require.
    private static func teamScopedAccess() -> SecAccess? {
        guard let team = ownTeamIdentifier() else { return nil }
        var created: SecAccess?
        guard SecAccessCreate(service as CFString, [] as CFArray, &created) == errSecSuccess,
              let created else {
            return nil
        }
        var aclList: CFArray?
        guard SecAccessCopyACLList(created, &aclList) == errSecSuccess,
              let acls = aclList as? [SecACL] else {
            return nil
        }
        for acl in acls {
            let auths = SecACLCopyAuthorizations(acl) as? [String] ?? []
            var apps: CFArray?
            var description: CFString?
            var prompt = SecKeychainPromptSelector()
            SecACLCopyContents(acl, &apps, &description, &prompt)
            let label = (description as String?) ?? service
            if auths.contains("ACLAuthorizationDecrypt") || auths.contains("ACLAuthorizationEncrypt") {
                // Empty application list means "any application"; access is held
                // back to the team by the partition list, not by naming binaries.
                SecACLSetContents(acl, nil, label as CFString, SecKeychainPromptSelector(rawValue: 0))
            }
            if auths.contains("ACLAuthorizationPartitionID") {
                let partitions = ["Partitions": ["teamid:\(team)"]]
                guard let data = try? PropertyListSerialization.data(
                    fromPropertyList: partitions, format: .xml, options: 0) else {
                    continue
                }
                let hex = data.map { String(format: "%02x", $0) }.joined()
                SecACLSetContents(acl, apps, hex as CFString, prompt)
            }
        }
        return created
    }

    /// The Team ID in the running binary's own code signature, or nil if it
    /// carries none (an unsigned local build). Everything TorroMail ships is
    /// signed with one team, so this is the identity the keychain items scope
    /// to — read once here rather than hardcoded, so it tracks the signature.
    static func ownTeamIdentifier() -> String? {
        var code: SecCode?
        guard SecCodeCopySelf(SecCSFlags(), &code) == errSecSuccess, let code else {
            return nil
        }
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, SecCSFlags(), &staticCode) == errSecSuccess,
              let staticCode else {
            return nil
        }
        var infoRef: CFDictionary?
        let flags = SecCSFlags(rawValue: kSecCSSigningInformation)
        guard SecCodeCopySigningInformation(staticCode, flags, &infoRef) == errSecSuccess,
              let info = infoRef as? [String: Any] else {
            return nil
        }
        return info[kSecCodeInfoTeamIdentifier as String] as? String
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
        public let accountAccess: ClientAccountAccess

        public init(clientID: String, name: String, tokenSHA256: String, accountAccess: ClientAccountAccess = .all) {
            self.clientID = clientID
            self.name = name
            self.tokenSHA256 = tokenSHA256
            self.accountAccess = accountAccess
        }
    }

    /// The three keychain operations a client key needs. Injectable so the
    /// heal path below — an item that is there but cannot be read — can be
    /// exercised without a real keychain, which no test may write to.
    public struct Storage: Sendable {
        public var read: @Sendable (_ account: String) -> String?
        public var write: @Sendable (_ password: String, _ account: String) throws -> Void
        public var delete: @Sendable (_ account: String) -> Void

        public init(
            read: @escaping @Sendable (_ account: String) -> String?,
            write: @escaping @Sendable (_ password: String, _ account: String) throws -> Void,
            delete: @escaping @Sendable (_ account: String) -> Void
        ) {
            self.read = read
            self.write = write
            self.delete = delete
        }

        public static let keychain = Storage(
            read: { KeychainStore.readPassword(forAccount: $0) },
            write: { try KeychainStore.savePassword($0, forAccount: $1) },
            delete: { KeychainStore.deletePassword(forAccount: $0) }
        )
    }

    /// The stored key for a client, or nil when it was never connected — or
    /// when this build may not read it.
    public static func token(
        forClient clientID: String,
        storage: Storage = .keychain
    ) -> String? {
        storage.read(accountPrefix + clientID)
    }

    /// The client's key, minting one on first use — and on an item this build
    /// cannot read. Reconnecting keeps a working key stable, because the
    /// client's config still carries it; rotation is otherwise an explicit
    /// renewal, never a side effect.
    ///
    /// The unreadable case is a legacy item: one an older build wrote with an
    /// access list naming that binary rather than the signing team. Its value
    /// is gone for good, and `savePassword` refreshes an existing item in
    /// place — which would keep that list, and with it the lockout. So the
    /// item is dropped and written fresh. That is safe here in a way it never
    /// is for a mail password: a client key is regenerable, and every caller
    /// writes the replacement straight into the client's own config.
    public static func tokenCreatingIfNeeded(
        forClient clientID: String,
        storage: Storage = .keychain
    ) throws -> String {
        if let existing = token(forClient: clientID, storage: storage) {
            return existing
        }
        storage.delete(accountPrefix + clientID)
        return try mintAndStore(forClient: clientID, storage: storage)
    }

    /// Renewal: the old key dies with its keychain item, the new one only
    /// starts working once the caller republishes the policy document. The
    /// item is deleted rather than overwritten, so the replacement takes the
    /// add path — the only one that attaches a current access list.
    public static func renewToken(
        forClient clientID: String,
        storage: Storage = .keychain
    ) throws -> String {
        storage.delete(accountPrefix + clientID)
        return try mintAndStore(forClient: clientID, storage: storage)
    }

    public static func revokeToken(
        forClient clientID: String,
        storage: Storage = .keychain
    ) {
        storage.delete(accountPrefix + clientID)
    }

    private static func mintAndStore(
        forClient clientID: String,
        storage: Storage
    ) throws -> String {
        let minted = mintToken(forClient: clientID)
        try storage.write(minted, accountPrefix + clientID)
        return minted
    }

    /// Whether the client still has to be restarted before the key TorroMail
    /// wrote takes effect. An assistant reads `TORROMAIL_TOKEN` once, when it
    /// spawns the server, so a session that started before the change keeps
    /// presenting the old key and every tool call it makes fails — with an
    /// error that reads like a setup problem, which by then it no longer is.
    ///
    /// A connection older than the change proves nothing; one after it is the
    /// proof, which is why this clears itself instead of asking to be
    /// dismissed.
    public static func restartPending(keyChangedAt: Date?, lastConnected: Date?) -> Bool {
        guard let keyChangedAt else { return false }
        guard let lastConnected else { return true }
        return lastConnected < keyChangedAt
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

/// What one run of the connection check found: the classified outcome, the
/// reason in words, and the state a caller acting on this single check alone
/// should show.
public struct AccountCheckResult: Hashable, Sendable {
    public var outcome: HealthOutcome
    public var detail: String

    public init(outcome: HealthOutcome, detail: String) {
        self.outcome = outcome
        self.detail = detail
    }

    /// For the manual test button: the user asked right now, so any failure
    /// is worth showing right now. The grace period for unreachable servers
    /// belongs to the background monitor, which has a log to judge from.
    public var state: ConnectionState {
        outcome == .ok ? .connected : .failed(detail)
    }

    /// What a *failing* check's stderr said, in the two parts the binary
    /// writes it: the outcome word alone on the first line, the reason after
    /// it.
    ///
    /// A binary older than that contract writes only prose, whose first line
    /// parses as no outcome word — and unreachable is the safe reading,
    /// because it earns a grace period rather than accusing the password.
    /// `ok` on the first line is refused for the same reason in reverse: this
    /// is only ever handed the words of a check that failed, and believing it
    /// would turn a failed check green, which is the bug the health log
    /// exists to remove.
    ///
    /// Split out from `run` because it is the part most likely to break
    /// silently — a misparse still moves the dot, it just blames the wrong
    /// thing — and it is pure string handling, so it can be pinned by tests.
    public static func parsing(stderr: String) -> AccountCheckResult {
        var lines = stderr
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .split(separator: "\n", omittingEmptySubsequences: false)
            .map(String.init)
        var outcome = HealthOutcome.unreachable
        if let first = lines.first, let word = HealthOutcome(rawValue: first), word != .ok {
            outcome = word
            lines.removeFirst()
        }
        let detail = lines
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return AccountCheckResult(
            outcome: outcome,
            // A silent failure still owes the user a sentence: `.failed("")`
            // would be just as red and tell nobody why.
            detail: detail.isEmpty ? "connection check failed" : detail
        )
    }
}

/// Collects a subprocess's stderr on a thread of its own.
///
/// Separate from the wait because the reading has to happen *while* the child
/// runs: a pipe's buffer is finite, and a child blocked on a full one never
/// exits — a stall the caller's timeout would then report as an unreachable
/// server rather than a talkative one.
private final class StderrDrain: @unchecked Sendable {
    /// Room for any reason worth showing, and a ceiling on what a runaway
    /// child can push into memory and from there into a `health.jsonl` line
    /// the monitor appends to every fifteen minutes.
    private static let limit = 8 * 1024
    private let lock = NSLock()
    private var bytes = Data()

    /// Reads until the last write end of the pipe closes. Blocking on
    /// purpose — the caller runs this on a queue of its own.
    func consume(_ handle: FileHandle) {
        while true {
            let chunk = handle.availableData
            if chunk.isEmpty { return }
            append(chunk)
        }
    }

    /// One chunk as a readability handler delivers it — the non-blocking
    /// sibling of `consume`, same ceiling.
    func append(_ chunk: Data) {
        lock.lock()
        if bytes.count < Self.limit {
            bytes.append(chunk.prefix(Self.limit - bytes.count))
        }
        lock.unlock()
    }

    /// Whatever has arrived so far. Decoded leniently: the cap can cut a
    /// multi-byte character in half, and a replacement character beats losing
    /// the whole reason.
    var text: String {
        lock.lock()
        defer { lock.unlock() }
        return String(decoding: bytes, as: UTF8.self)
    }
}

/// Runs the MCP binary's `--check-account`: the same secret resolution,
/// TLS and LOGIN the tools use — so a green dot means the real path works.
public enum AccountCheck {
    /// Checks an account the app has already published. `policyURL` overrides
    /// which document to check against — the wizard points it at a throwaway
    /// document so a candidate account can be proven before it joins the list.
    ///
    /// Never blocks longer than `timeout`: a background monitor calls this on
    /// a queue that other accounts are waiting on, and one hung TLS handshake
    /// must not be able to wedge it.
    public static func run(
        accountID: String,
        executableName: String,
        policyURL: URL? = nil,
        timeout: TimeInterval = 30
    ) -> AccountCheckResult {
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        // Everything that goes wrong before the server gets a word in is
        // unreachable, never rejected: a missing binary is not the user's
        // password, and calling it one would turn the dot red for good.
        guard let command = locator.resolve() else {
            return AccountCheckResult(outcome: .unreachable, detail: "MCP executable not found")
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
        // Nobody has ever read this check's stdout, and an undrained pipe is
        // one the child eventually blocks on mid-write. The null device
        // cannot fill.
        process.standardOutput = FileHandle.nullDevice
        process.standardError = errorPipe

        let finished = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in finished.signal() }
        do {
            try process.run()
        } catch {
            return AccountCheckResult(outcome: .unreachable, detail: error.localizedDescription)
        }

        // Started only once the child exists, so a spawn that threw cannot
        // leave a thread reading a pipe whose write end will never close.
        let drain = StderrDrain()
        let drained = DispatchGroup()
        drained.enter()
        DispatchQueue.global(qos: .utility).async {
            drain.consume(errorPipe.fileHandleForReading)
            drained.leave()
        }

        // A hung TLS handshake must not wedge the monitor's queue forever.
        // A check that never answers is a server we could not reach, which is
        // exactly what the grace period is for.
        if finished.wait(timeout: .now() + timeout) == .timedOut {
            // SIGTERM and go. Nothing waits for the child to actually die, and
            // nothing waits on the drain either: a check that ignored the
            // signal would hold this thread just as effectively as the hang we
            // are walking away from. The read ends by itself whenever the
            // child does.
            process.terminate()
            return AccountCheckResult(
                outcome: .unreachable,
                detail: "the connection check timed out"
            )
        }
        // The child is gone, so its end of the pipe is closed and the drain is
        // a moment from finishing — but bounded anyway, because a grandchild
        // holding the same pipe open is the one way this could still wait
        // forever, and stderr arrives outcome word first, so a partial read
        // still classifies.
        _ = drained.wait(timeout: .now() + 2)

        if process.terminationStatus == 0 {
            return AccountCheckResult(outcome: .ok, detail: "")
        }
        return .parsing(stderr: drain.text)
    }
}

/// The real on-disk footprint of one account's cache: the SQLite database
/// (its WAL sibling included) plus the retained attachment files. Measured,
/// never stored — a stored figure is how "0 MB" lies happen.
public enum CacheStorage {
    static func supportDirectory() -> URL? {
        (try? PolicyDocument.defaultURL())?.deletingLastPathComponent()
    }

    /// Everything the account keeps on disk, in bytes.
    public static func sizeBytes(accountID: String) -> Int64 {
        guard let support = supportDirectory() else { return 0 }
        var total: Int64 = 0
        let database = support.appendingPathComponent("cache/\(accountID).sqlite")
        for candidate in [database, URL(fileURLWithPath: database.path + "-wal")] {
            let size = (try? FileManager.default.attributesOfItem(atPath: candidate.path))?[.size]
            total += (size as? Int64) ?? 0
        }
        total += directorySize(support.appendingPathComponent("attachments/\(accountID)"))
        return total
    }

    private static func directorySize(_ directory: URL) -> Int64 {
        let manager = FileManager.default
        guard
            let walker = manager.enumerator(
                at: directory,
                includingPropertiesForKeys: [.fileSizeKey],
                options: [.skipsHiddenFiles]
            )
        else { return 0 }
        var total: Int64 = 0
        for case let file as URL in walker {
            let size = (try? file.resourceValues(forKeys: [.fileSizeKey]))?.fileSize
            total += Int64(size ?? 0)
        }
        return total
    }

    /// Delete is honest: the database files and the attachment directory go;
    /// the server recreates the schema lazily on its next write.
    public static func delete(accountID: String) throws {
        guard let support = supportDirectory() else { return }
        let manager = FileManager.default
        let database = support.appendingPathComponent("cache/\(accountID).sqlite")
        for path in [database.path, database.path + "-wal", database.path + "-shm"] {
            if manager.fileExists(atPath: path) {
                try manager.removeItem(atPath: path)
            }
        }
        let attachments = support.appendingPathComponent("attachments/\(accountID)")
        if manager.fileExists(atPath: attachments.path) {
            try manager.removeItem(at: attachments)
        }
    }

    public static func label(bytes: Int64) -> String {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter.string(fromByteCount: bytes)
    }
}

/// Fire-and-forget `--trim-cache`: the disk follows a lowered level or read
/// permission. Local housekeeping only — no login, no output anyone reads.
public enum CacheTrim {
    public static func run(accountID: String, executableName: String) {
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        guard let command = locator.resolve() else { return }
        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments + ["--trim-cache", accountID]
        var environment = ProcessInfo.processInfo.environment
        if let url = try? PolicyDocument.defaultURL() {
            environment["TORROMAIL_POLICY_PATH"] = url.path
        }
        process.environment = environment
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try? process.run()
    }
}

/// Runs `torromail-mcp --rebuild-cache <id>` and relays its JSON progress
/// lines — the same locator, policy path and app key `AccountCheck` uses,
/// because a rebuild logs into the real mailbox.
public final class CacheRebuild: @unchecked Sendable {
    public struct Progress: Sendable {
        public var phase: String
        public var mailbox: String
        public var done: Int
        public var total: Int

        public init(phase: String, mailbox: String, done: Int, total: Int) {
            self.phase = phase
            self.mailbox = mailbox
            self.done = done
            self.total = total
        }

        public var fraction: Double {
            total > 0 ? Double(done) / Double(total) : 0
        }
    }

    private let process: Process

    private init(process: Process) {
        self.process = process
    }

    /// `nil` when the MCP executable cannot be found or spawned — the caller
    /// shows that as the failure it is. Callbacks arrive on a background
    /// queue; hop to the main actor before touching UI state.
    public static func start(
        accountID: String,
        executableName: String,
        onProgress: @escaping @Sendable (Progress) -> Void,
        onFinish: @escaping @Sendable (Result<Void, Error>) -> Void
    ) -> CacheRebuild? {
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        guard let command = locator.resolve() else { return nil }

        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments + ["--rebuild-cache", accountID]
        var environment = ProcessInfo.processInfo.environment
        if let url = try? PolicyDocument.defaultURL() {
            environment["TORROMAIL_POLICY_PATH"] = url.path
        }
        if let appToken = try? MCPClientKeyStore.appToken() {
            environment["TORROMAIL_TOKEN"] = appToken
        }
        process.environment = environment

        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        // One JSON object per line; partial reads are buffered until their
        // newline arrives.
        let buffer = LineBuffer()
        stdout.fileHandleForReading.readabilityHandler = { handle in
            for line in buffer.consume(handle.availableData) {
                guard
                    let object = try? JSONSerialization.jsonObject(with: Data(line.utf8)),
                    let fields = object as? [String: Any]
                else { continue }
                onProgress(
                    Progress(
                        phase: fields["phase"] as? String ?? "",
                        mailbox: fields["mailbox"] as? String ?? "",
                        done: fields["done"] as? Int ?? 0,
                        total: fields["total"] as? Int ?? 0
                    )
                )
            }
        }
        let failureText = StderrDrain()
        stderr.fileHandleForReading.readabilityHandler = { handle in
            failureText.append(handle.availableData)
        }

        process.terminationHandler = { finished in
            stdout.fileHandleForReading.readabilityHandler = nil
            stderr.fileHandleForReading.readabilityHandler = nil
            if finished.terminationStatus == 0 {
                onFinish(.success(()))
            } else {
                let reason = failureText.text.trimmingCharacters(in: .whitespacesAndNewlines)
                onFinish(
                    .failure(
                        NSError(
                            domain: "TorroMail.CacheRebuild",
                            code: Int(finished.terminationStatus),
                            userInfo: [
                                NSLocalizedDescriptionKey: reason.isEmpty
                                    ? "the rebuild was interrupted" : reason
                            ]
                        )
                    )
                )
            }
        }

        do {
            try process.run()
        } catch {
            return nil
        }
        return CacheRebuild(process: process)
    }

    /// SIGTERM; batches are transactional, so a canceled rebuild leaves a
    /// valid partial cache behind.
    public func cancel() {
        process.terminate()
    }
}

/// Splits an incoming byte stream into complete lines, keeping the tail
/// until its newline arrives. Confined by the serial delivery of one pipe's
/// readability handler.
private final class LineBuffer: @unchecked Sendable {
    private var pending = Data()
    private let lock = NSLock()

    func consume(_ data: Data) -> [String] {
        lock.lock()
        defer { lock.unlock() }
        pending.append(data)
        var lines: [String] = []
        while let newline = pending.firstIndex(of: 0x0A) {
            let line = pending[pending.startIndex..<newline]
            lines.append(String(decoding: line, as: UTF8.self))
            pending = Data(pending[pending.index(after: newline)...])
        }
        return lines
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
        /// VS Code keeps the same shape under a top-level `servers` key instead
        /// (its `mcp.json`, alongside an `inputs` array we leave untouched), and
        /// tags each stdio server with `"type": "stdio"`. Same merge as
        /// `mcpServersJSON`, one different root key.
        case serversJSON(configURL: URL)
        /// OpenCode's `opencode.json`: the same merge under a top-level `mcp`
        /// key, with its own entry shape — `"type": "local"`, the command as
        /// a list, and the variables under `environment`.
        case openCodeJSON(configURL: URL)
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
        case let .mcpServersJSON(url), let .serversJSON(url), let .openCodeJSON(url),
             let .codexCLI(_, url), let .claudeCodeCLI(_, url):
            return url
        }
    }
}

extension MCPClient.Setup {
    /// For the JSON-file clients: the top-level key their servers live under.
    /// VS Code uses `servers`; everyone else with a JSON file — and Claude
    /// Code's `~/.claude.json`, which we only read — uses `mcpServers`.
    var jsonRootKey: String {
        switch self {
        case .serversJSON: return "servers"
        case .openCodeJSON: return "mcp"
        case .mcpServersJSON, .codexCLI, .claudeCodeCLI: return "mcpServers"
        }
    }

    /// Where a server entry keeps its variables: OpenCode calls the object
    /// `environment`, everyone else `env`.
    var jsonEnvironmentKey: String {
        if case .openCodeJSON = self { return "environment" }
        return "env"
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

        // OpenCode keeps its config under ~/.config on every platform. A
        // `.jsonc` is used when it is the only one there; comments in it make
        // it unreadable to a strict parser — an error, never an overwrite.
        let openCodeDirectory = home.appendingPathComponent(".config/opencode", isDirectory: true)
        if fileManager.fileExists(atPath: openCodeDirectory.path) {
            let json = openCodeDirectory.appendingPathComponent("opencode.json")
            let jsonc = openCodeDirectory.appendingPathComponent("opencode.jsonc")
            let jsoncOnly = !fileManager.fileExists(atPath: json.path) && fileManager.fileExists(atPath: jsonc.path)
            clients.append(MCPClient(
                id: "opencode",
                displayName: "OpenCode",
                setup: .openCodeJSON(configURL: jsoncOnly ? jsonc : json)
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

        // LM Studio keeps the identical `mcpServers` JSON at `~/.lmstudio/mcp.json`.
        let lmStudioDirectory = home.appendingPathComponent(".lmstudio", isDirectory: true)
        if fileManager.fileExists(atPath: lmStudioDirectory.path) {
            clients.append(MCPClient(
                id: "lm-studio",
                displayName: "LM Studio",
                setup: .mcpServersJSON(
                    configURL: lmStudioDirectory.appendingPathComponent("mcp.json")
                )
            ))
        }

        // Windsurf (Codeium) — same `mcpServers` schema, under its own directory.
        let windsurfDirectory = home
            .appendingPathComponent(".codeium/windsurf", isDirectory: true)
        if fileManager.fileExists(atPath: windsurfDirectory.path) {
            clients.append(MCPClient(
                id: "windsurf",
                displayName: "Windsurf",
                setup: .mcpServersJSON(
                    configURL: windsurfDirectory.appendingPathComponent("mcp_config.json")
                )
            ))
        }

        // VS Code keeps its servers under `servers` (not `mcpServers`) in the
        // user-profile `mcp.json`. The `User` directory is the app's marker.
        let vsCodeUserDirectory = home
            .appendingPathComponent("Library/Application Support/Code/User", isDirectory: true)
        if fileManager.fileExists(atPath: vsCodeUserDirectory.path) {
            clients.append(MCPClient(
                id: "vscode",
                displayName: "VS Code",
                setup: .serversJSON(
                    configURL: vsCodeUserDirectory.appendingPathComponent("mcp.json")
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
            try PolicyDocument.data(for: [account], clients: [appPairing], executableName: executableName)
                .write(to: url, options: .atomic)
        } catch {
            return .failed(error.localizedDescription)
        }
        // A trial is a manual check: the user is standing in the wizard, so it
        // wants this one check's verdict, not the monitor's patience. Nothing
        // is logged — the account has no id in the list yet, and a record
        // naming one that never joins would haunt every later derivation.
        return AccountCheck.run(
            accountID: account.id,
            executableName: executableName,
            policyURL: url
        ).state
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
        case let .mcpServersJSON(configURL), let .serversJSON(configURL), let .openCodeJSON(configURL),
             let .claudeCodeCLI(_, configURL):
            return jsonConfigHasServer(at: configURL, rootKey: client.setup.jsonRootKey)
        case let .codexCLI(_, configURL):
            // Reading the file beats launching the CLI on every refresh, and
            // a TOML table header is unambiguous enough to scan for.
            guard let text = try? String(contentsOf: configURL, encoding: .utf8) else {
                return false
            }
            return text.contains("[mcp_servers.\(MCPClientRegistry.serverName)]")
        }
    }

    /// Whether the top-level server object (`mcpServers`, or `servers` for VS
    /// Code) names our server. Shared by the clients whose config is JSON,
    /// including Claude Code's `~/.claude.json`.
    private static func jsonConfigHasServer(at configURL: URL, rootKey: String) -> Bool {
        guard let data = try? Data(contentsOf: configURL),
              let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let servers = root[rootKey] as? [String: Any] else {
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
        case let .mcpServersJSON(configURL), let .serversJSON(configURL), let .openCodeJSON(configURL),
             let .claudeCodeCLI(_, configURL):
            return jsonConfigToken(
                at: configURL,
                rootKey: client.setup.jsonRootKey,
                environmentKey: client.setup.jsonEnvironmentKey
            ) == token
        case let .codexCLI(_, configURL):
            // The key is high-entropy, so plain containment on the TOML is
            // unambiguous — better than parsing a format we never write.
            guard let text = try? String(contentsOf: configURL, encoding: .utf8) else {
                return false
            }
            return text.contains(token)
        }
    }

    private static func jsonConfigToken(at configURL: URL, rootKey: String, environmentKey: String) -> String? {
        guard let data = try? Data(contentsOf: configURL),
              let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let servers = root[rootKey] as? [String: Any],
              let server = servers[MCPClientRegistry.serverName] as? [String: Any],
              let env = server[environmentKey] as? [String: Any] else {
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
        case .serversJSON:
            return """
            {
              "servers": {
                "\(name)": {
                  "type": "stdio",
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
        case .openCodeJSON:
            return """
            {
              "mcp": {
                "\(name)": {
                  "type": "local",
                  "command": ["\(commandPath)"],
                  "environment": {
                    "TORROMAIL_TOKEN": "\(token)"
                  },
                  "enabled": true
                }
              }
            }
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
        case let .mcpServersJSON(configURL), let .serversJSON(configURL), let .openCodeJSON(configURL):
            try addToJSONConfig(
                at: configURL,
                rootKey: client.setup.jsonRootKey,
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
        rootKey: String,
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
        var servers = root[rootKey] as? [String: Any] ?? [:]
        var entry: [String: Any] = [
            "command": commandPath,
            "env": ["TORROMAIL_TOKEN": token]
        ]
        // OpenCode's schema: a `local` server whose command is a list and
        // whose variables are called `environment`.
        if rootKey == "mcp" {
            entry = [
                "type": "local",
                "command": [commandPath],
                "environment": ["TORROMAIL_TOKEN": token],
                "enabled": true
            ]
        }
        // VS Code's `servers` schema tags the transport; the `mcpServers`
        // clients infer stdio from `command` and reject an unknown key here.
        if rootKey == "servers" { entry["type"] = "stdio" }
        servers[MCPClientRegistry.serverName] = entry
        root[rootKey] = servers

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
        /// VS Code's `mcp.json`: the same entry under a `servers` key, tagged
        /// with `"type": "stdio"`.
        case serversJSON
        /// Hermes' `~/.hermes/config.yaml`: YAML under `mcp_servers`.
        case hermesYAML
        /// OpenClaw's `~/.openclaw/openclaw.json`: JSON nested under
        /// `mcp.servers`.
        case openClawJSON
        /// OpenCode's `opencode.json`: a `local` entry under `mcp`.
        case openCodeJSON
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
            id: "opencode",
            displayName: "OpenCode",
            symbol: "curlybraces",
            snippetFormat: .openCodeJSON,
            verificationHintKey: "Restart OpenCode, or run “opencode mcp list” in a terminal; “torromail” must be listed as connected. Then ask: “List my mail accounts.”"
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
            id: "lm-studio",
            displayName: "LM Studio",
            symbol: "cpu",
            verificationHintKey: "Restart LM Studio and open its MCP panel (the tools/plug icon); “torromail” must appear. Then ask a model: “List my mail accounts.”"
        ),
        MCPClientDescriptor(
            id: "vscode",
            displayName: "VS Code",
            symbol: "chevron.left.forwardslash.chevron.right",
            snippetFormat: .serversJSON,
            verificationHintKey: "Reload VS Code, open the MCP view (Command Palette → “MCP: List Servers”), and start “torromail” — trust it if asked. Then in Chat: “List my mail accounts.”"
        ),
        MCPClientDescriptor(
            id: "windsurf",
            displayName: "Windsurf",
            symbol: "wind",
            verificationHintKey: "Restart Windsurf and open Settings → MCP (or the Cascade MCP panel); “torromail” must show as active. Then ask: “List my mail accounts.”"
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
        case let .mcpServersJSON(configURL), let .serversJSON(configURL), let .openCodeJSON(configURL):
            try removeFromJSONConfig(
                at: configURL,
                rootKey: client.setup.jsonRootKey,
                fileManager: fileManager
            )
        case let .codexCLI(executableURL, _):
            try removeViaCLI(executableURL: executableURL, extraArguments: [])
        case let .claudeCodeCLI(executableURL, _):
            try removeViaCLI(executableURL: executableURL, extraArguments: ["-s", "user"])
        }
    }

    private static func removeFromJSONConfig(
        at target: URL,
        rootKey: String,
        fileManager: FileManager
    ) throws {
        guard let data = try? Data(contentsOf: target), !data.isEmpty else { return }
        guard var root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw Failure("The existing configuration could not be read.")
        }
        guard var servers = root[rootKey] as? [String: Any] else { return }
        servers[MCPClientRegistry.serverName] = nil
        root[rootKey] = servers
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

    /// What went wrong asking the server binary for the document. Shown in
    /// the log only — a failed publication is a diagnostic, not a decision.
    public struct Failure: LocalizedError, Hashable, Sendable {
        public let reason: String
        public var errorDescription: String? { reason }
    }

    /// The document for these accounts, as `torromail-mcp --policy-document`
    /// writes it. The writer lives in Rust (`torromail-control`) so every
    /// configuration surface publishes the same document from the same code;
    /// this side only states the facts: the accounts as `state.json` stores
    /// them, who is paired, and what belongs to this platform and build.
    ///
    /// `clients` is deliberately not defaulted: the allowlist is what stands
    /// between the accounts and any process that spawns the server, so every
    /// caller has to say who is allowed — an empty list means "nobody yet".
    public static func data(
        for accounts: [MailAccount],
        clients: [MCPClientKeyStore.Pairing],
        executableName: String = GeneralSettings().mcpExecutable,
        timeout: TimeInterval = 10
    ) throws -> Data {
        let request: [String: Any] = [
            "accounts": try JSONSerialization.jsonObject(with: JSONEncoder().encode(accounts)),
            "clients": clients.map { pairing in
                [
                    "id": pairing.clientID,
                    "name": pairing.name,
                    "token_sha256": pairing.tokenSHA256,
                    "account_access": pairing.accountAccess.policyObject
                ]
            },
            "context": [
                "secret_ref_prefix": KeychainStore.secretReference(forAccount: ""),
                "google_client_id": OAuthIssuer.google.clientID,
                "microsoft_client_id": OAuthIssuer.microsoft.clientID
            ]
        ]
        // Handed over as a file rather than through a pipe: nothing here has
        // to feed a child's stdin while also draining its stdout. It names
        // hosts and usernames, so it is owner-only and gone when we return.
        let requestURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("torromail-policy-request-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: requestURL) }
        try JSONSerialization.data(withJSONObject: request).write(to: requestURL, options: .atomic)
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: requestURL.path)

        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        guard let command = locator.resolve() else {
            throw Failure(reason: "MCP executable “\(executableName)” not found")
        }
        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments + ["--policy-document", requestURL.path]
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        let finished = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in finished.signal() }
        do {
            try process.run()
        } catch {
            throw Failure(reason: error.localizedDescription)
        }

        let output = PipeCapture(limit: 8 * 1024 * 1024)
        let errors = PipeCapture(limit: 8 * 1024)
        let drained = DispatchGroup()
        for (capture, handle) in [
            (output, outputPipe.fileHandleForReading),
            (errors, errorPipe.fileHandleForReading)
        ] {
            drained.enter()
            DispatchQueue.global(qos: .userInitiated).async {
                capture.consume(handle)
                drained.leave()
            }
        }
        if finished.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            throw Failure(reason: "the policy document was not written in time")
        }
        _ = drained.wait(timeout: .now() + 2)
        guard process.terminationStatus == 0 else {
            throw Failure(reason: errors.text.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        // An exit of zero with nothing usable behind it must not reach the
        // file the server reads its rights from.
        guard (try? JSONSerialization.jsonObject(with: output.data)) is [String: Any] else {
            throw Failure(reason: "the MCP executable returned no policy document")
        }
        return output.data
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
}

/// The other half of the bridge: the MCP server appends one line per tool call
/// to `audit.jsonl`, and the app reads it here to show what assistants have
/// been doing. Read-only — the app never writes this file.
public enum AuditLog {
    /// `~/Library/Application Support/TorroMail/audit.jsonl` — the sibling of
    /// the policy document the server writes to.
    public static func defaultURL(fileManager: FileManager = .default) throws -> URL {
        try fileManager
            .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("TorroMail", isDirectory: true)
            .appendingPathComponent("audit.jsonl")
    }

    /// One line exactly as the server writes it: machine fields the app turns
    /// into the display strings `AuditEntry` carries. A field an older server
    /// build omitted defaults rather than dropping the whole line.
    private struct RawEntry: Decodable {
        var ts: TimeInterval
        var client: String?
        var account: String?
        var tool: String?
        var detail: String?
        var result: String?
    }

    /// The most recent `limit` entries, newest last. Never throws: a missing
    /// file (nothing has happened yet) or a half-written trailing line yields
    /// what it can, because a broken log must not take the window down.
    ///
    /// `accountNames` maps account ids to the names the user gave them, so the
    /// log reads "Work" rather than the internal id; an id with no match (a
    /// since-removed account) falls back to the id itself.
    public static func load(
        limit: Int = 500,
        accountNames: [String: String] = [:],
        from url: URL? = nil,
        fileManager: FileManager = .default
    ) -> [AuditEntry] {
        guard let target = try? url ?? defaultURL(fileManager: fileManager),
              let text = try? String(contentsOf: target, encoding: .utf8) else {
            return []
        }
        let decoder = JSONDecoder()
        return text
            .split(separator: "\n", omittingEmptySubsequences: true)
            .suffix(limit)
            .compactMap { line -> AuditEntry? in
                guard let data = line.data(using: .utf8),
                      let raw = try? decoder.decode(RawEntry.self, from: data) else {
                    return nil
                }
                let accountID = raw.account ?? ""
                let account = accountID.isEmpty
                    ? ""
                    : (accountNames[accountID] ?? accountID)
                let date = Date(timeIntervalSince1970: raw.ts)
                return AuditEntry(
                    timestamp: date,
                    time: displayTime(for: date),
                    client: raw.client ?? "",
                    account: account,
                    event: raw.tool ?? "",
                    detail: raw.detail ?? "",
                    result: raw.result ?? ""
                )
            }
    }

    /// Compact for the activity card, dated once the entry is not from today —
    /// a log spanning days must not show two "09:14"s that are a week apart.
    private static func displayTime(for date: Date, now: Date = Date()) -> String {
        let formatter = DateFormatter()
        if Calendar.current.isDate(date, inSameDayAs: now) {
            formatter.dateFormat = "HH:mm"
        } else {
            formatter.dateFormat = "dd.MM. HH:mm"
        }
        return formatter.string(from: date)
    }
}

/// Watches `audit.jsonl` and fires when it grows, so the log and activity view
/// update while the app is open rather than only on relaunch. Polls the file's
/// size and modification date: the server only ever appends, the file may not
/// exist yet, and a poll sidesteps the descriptor churn a vnode source would
/// need to survive that.
public final class AuditWatcher {
    private let url: URL?
    private let interval: TimeInterval
    private var timer: DispatchSourceTimer?
    private var lastSignature: String?

    public init(url: URL? = nil, interval: TimeInterval = 2) {
        self.url = (try? url ?? AuditLog.defaultURL())
        self.interval = interval
    }

    /// Calls `onChange` on the main queue whenever the file changes. The first
    /// tick establishes a baseline without firing — the caller already loaded
    /// the log once at launch.
    public func start(onChange: @escaping @Sendable () -> Void) {
        stop()
        lastSignature = currentSignature()
        let timer = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
        timer.schedule(deadline: .now() + interval, repeating: interval)
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            let signature = self.currentSignature()
            guard signature != self.lastSignature else { return }
            self.lastSignature = signature
            DispatchQueue.main.async(execute: onChange)
        }
        timer.resume()
        self.timer = timer
    }

    public func stop() {
        timer?.cancel()
        timer = nil
    }

    deinit { stop() }

    /// Size and mtime together: either changing means the server touched the
    /// file. A missing file has no signature, so its later creation reads as a
    /// change and triggers the first real load.
    private func currentSignature() -> String? {
        guard let url,
              let attributes = try? FileManager.default.attributesOfItem(atPath: url.path) else {
            return nil
        }
        let size = (attributes[.size] as? Int) ?? 0
        let modified = (attributes[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
        return "\(size)-\(modified)"
    }
}

/// One MCP client's most recent handshake with the server. This is the signal
/// a config file cannot give: proof the client actually reached the server, not
/// merely that it is set up to. The server records it on every `initialize`,
/// attributed to the paired key the client presented.
public struct ClientConnection: Hashable, Sendable {
    public var clientID: String
    public var lastConnected: Date
    /// What the client called itself in the handshake, and its version — empty
    /// when it sent none. Flavour for the UI, not identity: attribution is the
    /// paired key, not this self-reported label.
    public var reportedName: String
    public var reportedVersion: String

    public init(
        clientID: String,
        lastConnected: Date,
        reportedName: String = "",
        reportedVersion: String = ""
    ) {
        self.clientID = clientID
        self.lastConnected = lastConnected
        self.reportedName = reportedName
        self.reportedVersion = reportedVersion
    }
}

/// Reads `connections.jsonl` — the log the server appends to on every
/// `initialize` handshake — and reduces it to each paired client's most recent
/// connection. Sibling of `audit.jsonl`, and shares its append-only,
/// never-throw contract: a missing file or a half-written trailing line yields
/// what it can rather than taking the window down.
public enum ClientConnectionLog {
    /// `~/Library/Application Support/TorroMail/connections.jsonl` — beside the
    /// policy document and the audit log the server also writes.
    public static func defaultURL(fileManager: FileManager = .default) throws -> URL {
        try fileManager
            .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("TorroMail", isDirectory: true)
            .appendingPathComponent("connections.jsonl")
    }

    /// One line as the server writes it. Snake-case keys are mapped by hand,
    /// mirroring `AuditLog.RawEntry`; a field an older build omitted defaults
    /// rather than dropping the whole line.
    private struct RawEntry: Decodable {
        var ts: TimeInterval
        var clientID: String?
        var clientName: String?
        var clientVersion: String?

        enum CodingKeys: String, CodingKey {
            case ts
            case clientID = "client_id"
            case clientName = "client_name"
            case clientVersion = "client_version"
        }
    }

    /// Each client's latest connection, keyed by the client id the server
    /// attributed it to. A client reconnects on every launch, so the newest
    /// line for an id wins.
    public static func latestByClient(
        from url: URL? = nil,
        fileManager: FileManager = .default
    ) -> [String: ClientConnection] {
        guard let target = try? url ?? defaultURL(fileManager: fileManager),
              let text = try? String(contentsOf: target, encoding: .utf8) else {
            return [:]
        }
        let decoder = JSONDecoder()
        var latest: [String: ClientConnection] = [:]
        for line in text.split(separator: "\n", omittingEmptySubsequences: true) {
            guard let data = line.data(using: .utf8),
                  let raw = try? decoder.decode(RawEntry.self, from: data),
                  let id = raw.clientID, !id.isEmpty else {
                continue
            }
            let date = Date(timeIntervalSince1970: raw.ts)
            if let existing = latest[id], existing.lastConnected >= date {
                continue
            }
            latest[id] = ClientConnection(
                clientID: id,
                lastConnected: date,
                reportedName: raw.clientName ?? "",
                reportedVersion: raw.clientVersion ?? ""
            )
        }
        return latest
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

/// How much of an account may rest on this Mac, in the language of the read
/// permissions. One decision — indexing always covers exactly what is
/// stored, so there is no separate index switch.
public enum CacheLevel: String, CaseIterable, Identifiable, Hashable, Sendable, Codable, Comparable {
    case off
    case headers
    case bodies
    case attachments

    public var id: Self { self }

    private var rank: Int {
        switch self {
        case .off: 0
        case .headers: 1
        case .bodies: 2
        case .attachments: 3
        }
    }

    public static func < (lhs: Self, rhs: Self) -> Bool {
        lhs.rank < rhs.rank
    }

    /// The most the read permission justifies keeping on disk: nothing is
    /// stored that no assistant may read.
    public static func ceiling(for read: ReadAccess) -> CacheLevel {
        switch read {
        case .none: .off
        case .headers: .headers
        case .fullMessage: .bodies
        case .withAttachments: .attachments
        }
    }
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
    public var level: CacheLevel

    public init(level: CacheLevel = .headers) {
        self.level = level
    }

    private enum CodingKeys: String, CodingKey {
        case level
    }

    /// The previous build stored four switches; they map exactly the way the
    /// server maps a legacy policy document, so an upgrade keeps the user's
    /// decision without asking again.
    private enum LegacyKeys: String, CodingKey {
        case localCacheEnabled
        case cacheMode
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        if let level = try container.decodeIfPresent(CacheLevel.self, forKey: .level) {
            self.level = level
            return
        }
        let legacy = try decoder.container(keyedBy: LegacyKeys.self)
        let enabled = try legacy.decodeIfPresent(Bool.self, forKey: .localCacheEnabled) ?? true
        let mode = try legacy.decodeIfPresent(String.self, forKey: .cacheMode) ?? "metadata"
        if !enabled {
            self.level = .off
        } else if mode == "body" || mode == "fullText" {
            self.level = .bodies
        } else {
            self.level = .headers
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(level, forKey: .level)
    }
}

/// Per-account manual choices for the five IMAP special-use folders. An empty
/// name keeps that role automatic even while the manual section is open.
public struct SpecialMailboxSettings: Hashable, Sendable, Codable {
    public var manual: Bool
    public var drafts: String
    public var sent: String
    public var archive: String
    public var junk: String
    public var trash: String

    public init(
        manual: Bool = false,
        drafts: String = "",
        sent: String = "",
        archive: String = "",
        junk: String = "",
        trash: String = ""
    ) {
        self.manual = manual
        self.drafts = drafts
        self.sent = sent
        self.archive = archive
        self.junk = junk
        self.trash = trash
    }

    public var policyOverrides: [String: String] {
        guard manual else { return [:] }
        let choices = [
            "drafts": drafts, "sent": sent, "archive": archive,
            "junk": junk, "trash": trash
        ]
        return choices.filter { !$0.value.isEmpty }
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

/// How a connection gets its TLS. A fact about the server, not about the
/// port: 993 and 465 are conventions, and providers do stray from them — an
/// implicit-TLS server on an unusual port used to be unreachable because the
/// port number was the only thing deciding this.
public enum ConnectionSecurity: String, Codable, CaseIterable, Identifiable, Sendable {
    /// TLS from the first byte.
    case tls
    /// Plaintext until the `STARTTLS` upgrade, which happens before any
    /// credential moves.
    case startTLS = "starttls"

    public var id: String { rawValue }

    /// What the port used to imply, for records written before this was
    /// stated. Keeps an existing account connecting exactly as it did.
    public static func impliedByIMAPPort(_ port: Int) -> ConnectionSecurity {
        port == 993 ? .tls : .startTLS
    }

    public static func impliedBySMTPPort(_ port: Int) -> ConnectionSecurity {
        port == 465 ? .tls : .startTLS
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
    public var imapSecurity: ConnectionSecurity
    public var smtpHost: String
    public var smtpPort: Int
    public var smtpSecurity: ConnectionSecurity
    public var username: String
    public var connectionState: ConnectionState
    /// Mailboxes the server reported (`mail_list_mailboxes`), in display
    /// order. Rights are decided in `permissions`; folders that appear here
    /// later automatically follow the account-wide standard.
    public var knownMailboxes: [String]
    public var permissions: PermissionSet
    public var specialMailboxes: SpecialMailboxSettings
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
        imapSecurity: ConnectionSecurity = .tls,
        smtpHost: String = "",
        smtpPort: Int = 587,
        smtpSecurity: ConnectionSecurity = .startTLS,
        username: String = "",
        connectionState: ConnectionState = .notConfigured,
        knownMailboxes: [String] = ["INBOX"],
        permissions: PermissionSet = PermissionSet(),
        specialMailboxes: SpecialMailboxSettings = SpecialMailboxSettings(),
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
        self.imapSecurity = imapSecurity
        self.smtpHost = smtpHost
        self.smtpPort = smtpPort
        self.smtpSecurity = smtpSecurity
        self.username = username
        self.connectionState = connectionState
        self.knownMailboxes = knownMailboxes
        self.permissions = permissions
        self.specialMailboxes = specialMailboxes
        self.searchCache = searchCache
        self.pendingActions = pendingActions
    }

    public var needsAttention: Bool {
        connectionState.needsAttention || !pendingActions.isEmpty
    }

    /// Whether this account carries the facts a mailbox can be opened from —
    /// the exact test `PolicyDocument` applies before it writes the `imap`
    /// block, and therefore the line between an account the server can serve
    /// and one it refuses as unconfigured.
    ///
    /// Named rather than repeated because a second copy of the rule drifting
    /// from this one is not a cosmetic problem: the health monitor asks about
    /// what this says is connectable, and an account the server has no
    /// connection facts for answers `unreachable` every time it is asked.
    public var hasIMAPConnection: Bool {
        !imapHost.isEmpty && !username.isEmpty
    }
}

/// What survives a launch, stated explicitly. The password is not here — it
/// lives in the keychain. Neither are connection state and pending
/// approvals: both are facts about right now, so they start fresh, and a
/// verified account is remembered as verified.
extension MailAccount: Codable {
    private enum CodingKeys: String, CodingKey {
        case id, name, email, provider, loginMethod, imapHost, smtpHost
        case username, knownMailboxes, permissions, specialMailboxes, searchCache, isVerified
        case imapPort, smtpPort, oauthIssuer
        case imapSecurity, smtpSecurity
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let username = try container.decode(String.self, forKey: .username)
        let imapHost = try container.decode(String.self, forKey: .imapHost)
        let isVerified = try container.decodeIfPresent(Bool.self, forKey: .isVerified) ?? false
        // Ports first: a record from before the encryption was stated has to
        // fall back to what its port implied, or a 587 account would suddenly
        // be asked for implicit TLS.
        let imapPort = try container.decodeIfPresent(Int.self, forKey: .imapPort) ?? 993
        let smtpPort = try container.decodeIfPresent(Int.self, forKey: .smtpPort) ?? 587

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
            imapPort: imapPort,
            imapSecurity: try container.decodeIfPresent(
                ConnectionSecurity.self,
                forKey: .imapSecurity
            ) ?? .impliedByIMAPPort(imapPort),
            smtpHost: try container.decode(String.self, forKey: .smtpHost),
            smtpPort: smtpPort,
            smtpSecurity: try container.decodeIfPresent(
                ConnectionSecurity.self,
                forKey: .smtpSecurity
            ) ?? .impliedBySMTPPort(smtpPort),
            username: username,
            connectionState: MailAccount.restoredState(
                isVerified: isVerified,
                username: username,
                imapHost: imapHost
            ),
            knownMailboxes: try container.decode([String].self, forKey: .knownMailboxes),
            permissions: try container.decode(PermissionSet.self, forKey: .permissions),
            specialMailboxes: try container.decodeIfPresent(SpecialMailboxSettings.self, forKey: .specialMailboxes) ?? SpecialMailboxSettings(),
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
        try container.encode(imapSecurity, forKey: .imapSecurity)
        try container.encode(smtpHost, forKey: .smtpHost)
        try container.encode(smtpPort, forKey: .smtpPort)
        try container.encode(smtpSecurity, forKey: .smtpSecurity)
        try container.encode(username, forKey: .username)
        try container.encode(knownMailboxes, forKey: .knownMailboxes)
        try container.encode(permissions, forKey: .permissions)
        try container.encode(specialMailboxes, forKey: .specialMailboxes)
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

/// What to do about the system's login-item registration so it matches the
/// launch-at-login setting. A stored `true` on its own starts nothing — the
/// system's list is the only thing `loginwindow` reads — so the two are
/// reconciled towards the setting on every launch and on every flip of the
/// toggle, whichever side drifted. The ServiceManagement calls live in the
/// app; this is only the decision.
public enum LoginItemSync: Hashable {
    case register
    case unregister
    case inSync

    public static func resolve(wantsLaunchAtLogin: Bool, systemHasLoginItem: Bool) -> LoginItemSync {
        switch (wantsLaunchAtLogin, systemHasLoginItem) {
        case (true, false): .register
        case (false, true): .unregister
        default: .inSync
        }
    }
}

/// What a reopen — Dock click, launching the already-running app again —
/// should do. Two actors can answer it: this app, and AppKit's own reopen
/// handling. The rule is mutual exclusion. When TorroMail opens the window
/// itself it must also tell AppKit "handled", or both act and every reopen
/// from the background shows the window twice.
public enum ReopenResponse: Hashable {
    /// No window on screen: open ours, and silence the system's handling.
    case showMainWindow
    /// A window is up: let the system bring it forward, touch nothing.
    case letSystemProceed

    public static func resolve(hasVisibleWindows: Bool) -> ReopenResponse {
        hasVisibleWindows ? .letSystemProceed : .showMainWindow
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
    /// When the action happened — the sortable source of truth. `time` is only
    /// its human-facing rendering, which is not orderable on its own (a compact
    /// "HH:mm" for today sits next to a dated "dd.MM. HH:mm" for older entries).
    public var timestamp: Date
    public var time: String
    public var client: String
    public var account: String
    public var event: String
    /// What the call acted on — the query searched, the message read, the
    /// recipients a draft names, the count and destination of a move or delete.
    /// Empty for calls that name nothing (listing accounts, reading the policy).
    public var detail: String
    public var result: String

    public init(
        id: UUID = UUID(),
        timestamp: Date = Date(),
        time: String,
        client: String,
        account: String,
        event: String,
        detail: String = "",
        result: String
    ) {
        self.id = id
        self.timestamp = timestamp
        self.time = time
        self.client = client
        self.account = account
        self.event = event
        self.detail = detail
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
    /// Each MCP client's most recent handshake, keyed by client id — the live
    /// "last connected" the detail view shows. Kept fresh by a watcher on
    /// `connections.jsonl`, so a client restarting to connect turns the screen
    /// green without a reload.
    @Published public var clientConnections: [String: ClientConnection]
    /// When each account was last checked, for the account detail's "last
    /// checked …" line. Not persisted: it is a fact about the log, and the log
    /// is on disk.
    @Published public var lastHealthCheck: [String: Date] = [:]
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
        clientConnections: [String: ClientConnection] = [:],
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
        self.clientConnections = clientConnections
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

    /// The accounts the health monitor may check. An account whose setup is
    /// unfinished is deliberately absent: `--check-account` has no way to say
    /// "nothing to report", so it would answer `unreachable`, three of those
    /// would elapse the grace period, and the app would raise an alarm about
    /// an account the user has not finished creating.
    ///
    /// `hasIMAPConnection` rather than a rule of its own, because the question
    /// is not "does this look configured" but "does the policy document give
    /// the server anything to open" — and that document is written from the
    /// same property. It is the stricter half of what `restoredState` asks:
    /// a host without a username still deserves the wizard's "needs test", but
    /// there is nothing there to test it against yet.
    public var checkableAccountIDs: [String] {
        accounts.filter(\.hasIMAPConnection).map(\.id)
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
            // What the assistants have done, as the MCP server recorded it.
            // Empty until the first tool call — that is what the activity
            // card's empty state is for.
            audit: AuditLog.load(accountNames: accountNames(state.accounts)),
            connectedClients: MCPClientSetup.configuredClientNames(),
            clientConnections: ClientConnectionLog.latestByClient(),
            news: releaseNotes()
        )
    }

    /// Account id → the name the user gave it, so the log can attribute a call
    /// to "Work" rather than its internal id.
    public static func accountNames(_ accounts: [MailAccount]) -> [String: String] {
        Dictionary(accounts.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
    }

    /// Re-read the log — after the watcher reports the file grew, or when the
    /// account names it resolves against may have changed.
    public func reloadAudit() {
        audit = AuditLog.load(accountNames: TorroMailModel.accountNames(accounts))
    }

    /// Re-derive every account's connection state from `health.jsonl` — after
    /// the watcher reports the file grew, or at launch.
    ///
    /// The whole list is derived into a copy and assigned only if the copy
    /// differs. `accounts` is observed: every assignment republishes the policy
    /// document and rewrites the state file, and this runs on every quiet
    /// check, four times an hour, forever. Deriving into a copy also makes the
    /// publish atomic — one change notification per call, whatever the log had
    /// to say about how many accounts.
    ///
    /// An unreadable or empty log deliberately has no guard of its own: it
    /// yields no records, `derive` hands every account back its own state as
    /// the fallback, and the copy compares equal. A log the app cannot read is
    /// thereby unable to move a single dot — which is the property worth
    /// having, and it costs nothing to get it this way. `lastHealthCheck` does
    /// clear, and should: nothing has been checked that we can still see.
    public func applyHealth(from url: URL? = nil) {
        let records = HealthLog.load(from: url)
        var derived = accounts
        for index in derived.indices {
            derived[index].connectionState = HealthLog.derive(
                records: records,
                accountID: derived[index].id,
                fallback: derived[index].connectionState
            )
        }
        if derived != accounts {
            accounts = derived
        }
        lastHealthCheck = HealthLog.lastChecked(records: records)
    }

    /// Re-read the connection log — after its watcher reports the file grew, so
    /// a client that just handshook shows as connected while the app is open.
    public func reloadClientConnections() {
        clientConnections = ClientConnectionLog.latestByClient()
    }

    /// Local notes from Torro — not account data, so they are not stored.
    static func releaseNotes() -> [NewsItem] {
        [
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
                    searchCache: SearchCacheSettings(level: .headers),
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
                    time: "10:42",
                    client: "Claude Desktop",
                    account: "Work",
                    event: "mail_search",
                    detail: "Rechnung · INBOX",
                    result: "ok"
                ),
                AuditEntry(
                    time: "10:43",
                    client: "Claude Desktop",
                    account: "Work",
                    event: "mail_prepare_move",
                    detail: "3 → Archiv",
                    result: "ok"
                ),
                AuditEntry(
                    time: "10:44",
                    client: "Claude Desktop",
                    account: "Work",
                    event: "mail_confirm_action",
                    detail: "moved 3 → Archiv",
                    result: "ok"
                )
            ],
            connectedClients: ["Claude Desktop"],
            news: [
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
