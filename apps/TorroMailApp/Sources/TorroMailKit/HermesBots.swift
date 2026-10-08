import Foundation
import Yams

public struct HermesBot: Codable, Identifiable, Hashable, Sendable {
    public let id: String
    public let name: String
    public let clientID: String
    public var configured: Bool
    public var authorized: Bool
    public var access: ClientAccountAccess
    public var problem: String?
    public var needsReload = false
    public var isReady: Bool { configured && authorized && problem == nil }
}

public struct HermesSharedAccount: Codable, Identifiable, Hashable, Sendable {
    public let id: String
    public let name: String
    public let email: String
}

public struct HermesBotSelection: Codable, Sendable {
    public let id: String
    public let access: ClientAccountAccess

    public init(id: String, access: ClientAccountAccess) {
        self.id = id
        self.access = access
    }
}

public struct HermesControlRequest: Codable, Sendable {
    public enum Action: String, Codable, Sendable { case list, connect, disconnect, test }
    public var action: Action
    public var selections: [HermesBotSelection]

    public init(action: Action, selections: [HermesBotSelection] = []) {
        self.action = action
        self.selections = selections
    }
}

public struct HermesBotResult: Codable, Identifiable, Sendable {
    public let id: String
    public let success: Bool
    public let needsReload: Bool
    public let problem: String?
}

public struct HermesControlResponse: Codable, Sendable {
    public var version = 1
    public var bots: [HermesBot] = []
    public var accounts: [HermesSharedAccount] = []
    public var results: [HermesBotResult] = []
    public var problem: String?

    public init(problem: String? = nil) { self.problem = problem }
}

/// Files are inspected without launching Hermes. Profiles are explicit targets,
/// independent of the sticky active profile, which can change between clicks.
public enum HermesBots {
    public static func root(home: URL, overrideHome: String? = nil) -> URL {
        guard let value = overrideHome?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty else {
            return home.appendingPathComponent(".hermes", isDirectory: true)
        }
        let path = value.hasPrefix("~/") ? home.appendingPathComponent(String(value.dropFirst(2))).path : value
        let selected = URL(fileURLWithPath: path, isDirectory: true).standardizedFileURL
        return selected.deletingLastPathComponent().lastPathComponent == "profiles"
            ? selected.deletingLastPathComponent().deletingLastPathComponent() : selected
    }

    public static func validProfile(_ name: String) -> Bool {
        guard let first = name.first, first.isASCII, first.isLetter || first.isNumber, name.count <= 64 else { return false }
        return name.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_" || $0 == "-") }
    }

    public static func profileHomes(root: URL, fileManager: FileManager = .default) throws -> [(String, URL)] {
        var isDirectory: ObjCBool = false
        guard fileManager.fileExists(atPath: root.path, isDirectory: &isDirectory), isDirectory.boolValue else {
            throw MCPClientSetup.Failure("Hermes is not installed on this computer.")
        }
        var homes = [("default", root)]
        let profiles = root.appendingPathComponent("profiles", isDirectory: true)
        if fileManager.fileExists(atPath: profiles.path) {
            for url in try fileManager.contentsOfDirectory(at: profiles, includingPropertiesForKeys: [.isDirectoryKey]).sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
                let name = url.lastPathComponent
                guard name != "default", validProfile(name), (try url.resourceValues(forKeys: [.isDirectoryKey])).isDirectory == true else { continue }
                // A directory symlink outside this installation must not make
                // a selected bot write an unrelated Hermes installation.
                guard url.resolvingSymlinksInPath().deletingLastPathComponent() == profiles.resolvingSymlinksInPath() else { continue }
                homes.append((name, url))
            }
        }
        return homes
    }

    public static func clientID(root: URL, profile: String) -> String {
        let installation = MCPClientKeyStore.sha256Hex(root.resolvingSymlinksInPath().path).prefix(12)
        return "hermes-bot-\(installation)-\(profile)"
    }

    static func displayName(profile: String, home: URL) -> String {
        guard let text = try? String(contentsOf: home.appendingPathComponent("profile.yaml"), encoding: .utf8),
              let meta = try? Yams.load(yaml: text) as? [String: Any],
              let value = meta["display_name"] as? String, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return profile }
        return value
    }
}
