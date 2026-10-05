import Foundation
import Yams

/// Hermes' CLI adds servers interactively. Its YAML file is the supported
/// noninteractive setup surface; never run Hermes just to render client status.
public enum HermesClientConfiguration {
    public static func configURL(home: URL, overrideHome: String? = nil) -> URL {
        let override = overrideHome?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !override.isEmpty {
            let path = override.hasPrefix("~/")
                ? home.appendingPathComponent(String(override.dropFirst(2))).path : override
            return URL(fileURLWithPath: path, isDirectory: true).appendingPathComponent("config.yaml")
        }
        let root = home.appendingPathComponent(".hermes", isDirectory: true)
        let profile = (try? String(contentsOf: root.appendingPathComponent("active_profile"), encoding: .utf8))?
            .trimmingCharacters(in: CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\u{FEFF}"))) ?? ""
        // A selector is one path component, never an arbitrary destination.
        if !profile.isEmpty, profile != "default", profile != ".", profile != "..",
           profile.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_" || $0 == "-") }) {
            return root.appendingPathComponent("profiles/\(profile)/config.yaml")
        }
        return root.appendingPathComponent("config.yaml")
    }

    public static func isConfigured(at url: URL) -> Bool {
        guard let root = try? read(at: url), let servers = root["mcp_servers"] as? [String: Any] else { return false }
        return servers[MCPClientRegistry.serverName] is [String: Any]
    }

    public static func token(at url: URL) -> String? {
        guard let root = try? read(at: url),
              let servers = root["mcp_servers"] as? [String: Any],
              let server = servers[MCPClientRegistry.serverName] as? [String: Any],
              let env = server["env"] as? [String: Any] else { return nil }
        return env["TORROMAIL_TOKEN"] as? String
    }

    public static func set(at url: URL, commandPath: String, token: String, fileManager: FileManager = .default) throws {
        try update(at: url, definition: ["command": commandPath, "env": ["TORROMAIL_TOKEN": token]], fileManager: fileManager)
    }

    public static func remove(at url: URL, fileManager: FileManager = .default) throws {
        try update(at: url, definition: nil, fileManager: fileManager)
    }

    private static func parse(_ text: String) throws -> [String: Any] {
        if text.components(separatedBy: .newlines).allSatisfy({
            let line = $0.trimmingCharacters(in: .whitespaces)
            return line.isEmpty || line.hasPrefix("#")
        }) { return [:] }
        do {
            // Yams' constructor otherwise silently collapses duplicate keys.
            if let node = try Yams.compose(yaml: text) { try validateKeys(node) }
            guard let root = try Yams.load(yaml: text) as? [String: Any] else { throw unreadable() }
            if let servers = root["mcp_servers"], !(servers is NSNull), !(servers is [String: Any]) { throw unreadable() }
            return root
        } catch { throw unreadable() }
    }

    private static func validateKeys(_ node: Node) throws {
        if let mapping = node.mapping {
            var keys = Set<Node>()
            for (key, value) in mapping {
                guard keys.insert(key).inserted else { throw unreadable() }
                try validateKeys(value)
            }
        } else if let sequence = node.sequence {
            for child in sequence { try validateKeys(child) }
        }
    }

    private static func read(at url: URL) throws -> [String: Any] {
        try parse(source(at: url))
    }

    private static func source(at url: URL) throws -> String {
        do { return try String(contentsOf: url, encoding: .utf8) }
        catch let error as NSError where error.domain == NSCocoaErrorDomain && error.code == NSFileReadNoSuchFileError {
            if (try? FileManager.default.destinationOfSymbolicLink(atPath: url.path)) != nil { throw unreadable() }
            return ""
        }
        catch { throw unreadable() }
    }

    private static func unreadable() -> MCPClientSetup.Failure {
        MCPClientSetup.Failure("The existing configuration could not be read.")
    }

    private static func update(at url: URL, definition: [String: Any]?, fileManager: FileManager) throws {
        let original = try source(at: url)
        var root = try parse(original)
        var servers = root["mcp_servers"] as? [String: Any] ?? [:]
        if definition == nil, servers[MCPClientRegistry.serverName] == nil { return }
        servers[MCPClientRegistry.serverName] = definition
        root["mcp_servers"] = servers
        // Keep ordinary block YAML, including other servers and comments, byte
        // for byte. A real parser checks the candidate against the intended
        // document; flow mappings/aliases use a semantic YAML rewrite instead.
        let entry = try definition.map { try Yams.dump(object: [MCPClientRegistry.serverName: $0]) } ?? ""
        let candidate = patchBlock(original, entry: entry)
        let text: String
        if let candidate, let parsed = try? parse(candidate), NSDictionary(dictionary: parsed).isEqual(to: root) {
            text = candidate
        } else {
            text = try Yams.dump(object: root, sortKeys: true)
        }
        let target = url.resolvingSymlinksInPath()
        try fileManager.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
        // Set permissions on the temporary file before renaming, so a new key
        // is never exposed through a world-readable intermediate config.
        let temporary = target.deletingLastPathComponent().appendingPathComponent(".torromail-hermes-\(UUID().uuidString)")
        defer { try? fileManager.removeItem(at: temporary) }
        guard fileManager.createFile(atPath: temporary.path, contents: Data(text.utf8), attributes: [.posixPermissions: 0o600]) else {
            throw MCPClientSetup.Failure("Could not update the configuration.")
        }
        if fileManager.fileExists(atPath: target.path) {
            _ = try fileManager.replaceItemAt(target, withItemAt: temporary, options: .usingNewMetadataOnly)
        } else {
            try fileManager.moveItem(at: temporary, to: target)
        }
    }

    /// A conservative formatting optimization, never a YAML parser.
    private static func patchBlock(_ original: String, entry: String) -> String? {
        var lines = original.components(separatedBy: "\n")
        func content(_ line: String) -> String { line.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false)[0].trimmingCharacters(in: .whitespacesAndNewlines) }
        func significant(_ line: String) -> Bool { !content(line).isEmpty }
        func indent(_ line: String) -> Int { line.prefix(while: { $0 == " " }).count }
        guard let start = lines.firstIndex(where: { indent($0) == 0 && content($0) == "mcp_servers:" }) else {
            guard !original.contains("mcp_servers"), !entry.isEmpty else { return nil }
            return original + (original.isEmpty || original.hasSuffix("\n") ? "" : "\n") + "mcp_servers:\n" + entry.split(separator: "\n").map { "  " + $0 }.joined(separator: "\n") + "\n"
        }
        let end = ((start + 1)..<lines.count).first(where: { significant(lines[$0]) && indent(lines[$0]) == 0 }) ?? lines.count
        let children = ((start + 1)..<end).filter { significant(lines[$0]) }
        let level = children.first.map { indent(lines[$0]) } ?? 2
        guard level > 0 else { return nil }
        let own = children.first(where: { indent(lines[$0]) == level && content(lines[$0]).hasPrefix("torromail:") })
        var lower = own ?? end
        if own == nil {
            while lower > start + 1, !significant(lines[lower - 1]) { lower -= 1 }
        }
        var upper = own.map { index in
            ((index + 1)..<end).first(where: { significant(lines[$0]) && indent(lines[$0]) <= level }) ?? end
        } ?? lower
        // Comments immediately before the next server belong to that server.
        while upper > lower + 1, !significant(lines[upper - 1]) { upper -= 1 }
        let replacement = entry.split(separator: "\n").map { String(repeating: " ", count: level) + $0 }
        lines.replaceSubrange(lower..<upper, with: replacement)
        return lines.joined(separator: "\n") + (lines.last == "" ? "" : "\n")
    }
}
