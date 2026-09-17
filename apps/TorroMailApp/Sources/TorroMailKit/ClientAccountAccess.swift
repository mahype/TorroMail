import Foundation

/// Account grants belong to the paired client, independently of key rotation.
public enum ClientAccountAccess: Hashable, Sendable, Codable {
    case all
    case selected(Set<String>)

    private enum CodingKeys: String, CodingKey { case mode, accountIDs = "account_ids" }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(String.self, forKey: .mode) {
        case "all":
            guard !values.contains(.accountIDs) else {
                throw DecodingError.dataCorruptedError(forKey: .accountIDs, in: values, debugDescription: "All accounts cannot also select IDs")
            }
            self = .all
        case "selected":
            let ids = try values.decode(Set<String>.self, forKey: .accountIDs)
            guard !ids.contains("") else {
                throw DecodingError.dataCorruptedError(forKey: .accountIDs, in: values, debugDescription: "Empty account ID")
            }
            self = .selected(ids)
        default:
            throw DecodingError.dataCorruptedError(forKey: .mode, in: values, debugDescription: "Unknown account access mode")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .all: try values.encode("all", forKey: .mode)
        case .selected(let ids):
            try values.encode("selected", forKey: .mode)
            try values.encode(ids.sorted(), forKey: .accountIDs)
        }
    }

    public var policyObject: [String: Any] {
        switch self {
        case .all: ["mode": "all"]
        case .selected(let ids): ["mode": "selected", "account_ids": ids.sorted()]
        }
    }
}

/// Separate from regenerable keys and the general UI state. Invalid data throws
/// so neither loading nor republishing silently broadens a client's access.
public enum ClientAccountAccessStore {
    public static func defaultURL() throws -> URL {
        try PolicyDocument.defaultURL().deletingLastPathComponent()
            .appendingPathComponent("client-account-access.json")
    }

    /// Migration happens once, before minting any new client key. Existing
    /// pairings keep all accounts; clients added afterwards start with none.
    public static func load(
        from url: URL? = nil, legacyClientIDs: [String] = []
    ) throws -> [String: ClientAccountAccess] {
        let target = try url ?? defaultURL()
        do {
            return try JSONDecoder().decode([String: ClientAccountAccess].self, from: Data(contentsOf: target))
        } catch CocoaError.fileReadNoSuchFile {
            let migrated = Dictionary(legacyClientIDs.map { ($0, ClientAccountAccess.all) }, uniquingKeysWith: { first, _ in first })
            try save(migrated, to: target)
            return migrated
        }
    }

    public static func save(_ grants: [String: ClientAccountAccess], to url: URL? = nil) throws {
        let target = try url ?? defaultURL()
        try FileManager.default.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(grants).write(to: target, options: .atomic)
    }

    public static func applying(
        _ grants: [String: ClientAccountAccess], to pairings: [MCPClientKeyStore.Pairing]
    ) -> [MCPClientKeyStore.Pairing] {
        pairings.map { pairing in
            MCPClientKeyStore.Pairing(
                clientID: pairing.clientID, name: pairing.name, tokenSHA256: pairing.tokenSHA256,
                accountAccess: pairing.clientID == MCPClientKeyStore.appClientID
                    ? .all : grants[pairing.clientID] ?? .selected([])
            )
        }
    }
}
