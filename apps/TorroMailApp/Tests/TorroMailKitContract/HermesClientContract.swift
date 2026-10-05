import Foundation
import TorroMailKit
import Yams

func hermesClientContract() throws {
    let fixtureURL = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().appendingPathComponent("contracts/hermes-client-config.json")
    let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf: fixtureURL)) as! [String: Any]
    let command = fixture["command_path"] as! String
    let token = fixture["token"] as! String
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("torromail-hermes-contract-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    for (index, test) in (fixture["cases"] as! [[String: Any]]).enumerated() {
        let name = test["name"] as! String
        let url = directory.appendingPathComponent("case-\(index).yaml")
        if let original = test["existing"] as? String { try original.write(to: url, atomically: true, encoding: .utf8) }
        let client = MCPClient(id: "hermes", displayName: "Hermes", setup: .hermesYAML(configURL: url))
        let removing = test["action"] as? String == "remove"
        do {
            if removing { try MCPClientSetup.remove(from: client) }
            else { try MCPClientSetup.add(to: client, commandPath: command, token: token) }
            require(test["error"] as? Bool != true, "Hermes rejects \(name)")
        } catch let error as MCPClientSetup.Failure {
            require(test["error"] as? Bool == true && error.reason == "The existing configuration could not be read.", "Hermes case \(name): \(error.reason)")
            require((try? String(contentsOf: url, encoding: .utf8)) == test["existing"] as? String, "Hermes leaves invalid config untouched: \(name)")
            require(!MCPClientSetup.isConfigured(client), "Hermes invalid config is not configured: \(name)")
            continue
        }
        if removing, test["existing"] is NSNull {
            require(!FileManager.default.fileExists(atPath: url.path), "Hermes disconnect does not create a file")
            continue
        }
        let text = try String(contentsOf: url, encoding: .utf8)
        var actual = try Yams.load(yaml: text) as! [String: Any]
        var original = (try Yams.load(yaml: test["existing"] as? String ?? "{}") as? [String: Any]) ?? [:]
        var actualServers = actual["mcp_servers"] as? [String: Any] ?? [:]
        let own = actualServers.removeValue(forKey: "torromail") as? [String: Any]
        actual["mcp_servers"] = actualServers
        var originalServers = original["mcp_servers"] as? [String: Any] ?? [:]
        originalServers["torromail"] = nil
        original["mcp_servers"] = originalServers
        require(NSDictionary(dictionary: actual).isEqual(to: original), "Hermes preserves all unrelated values: \(name)")
        require(MCPClientSetup.isConfigured(client) == !removing, "Hermes configured status: \(name)")
        if !removing {
            require(own?["command"] as? String == command, "Hermes command: \(name)")
            require(HermesClientConfiguration.token(at: url) == token, "Hermes reads the exact entry's token: \(name)")
        }
        for expected in test["contains"] as? [String] ?? [] {
            require(text.contains(expected), "Hermes preserves text: \(name): \(expected)")
        }
        if !removing || own != nil {
            let mode = try FileManager.default.attributesOfItem(atPath: url.path)[.posixPermissions] as? Int
            require(mode == 0o600, "Hermes config is private: \(name)")
        }
    }

    let home = directory.appendingPathComponent("home")
    let manager = AntigravityTestFileManager(home: home)
    let root = home.appendingPathComponent(".hermes")
    try manager.createDirectory(at: root.appendingPathComponent("profiles/work"), withIntermediateDirectories: true)
    try "work\n".write(to: root.appendingPathComponent("active_profile"), atomically: true, encoding: .utf8)
    let profileURL = root.appendingPathComponent("profiles/work/config.yaml")
    require(HermesClientConfiguration.configURL(home: home) == profileURL, "Hermes follows sticky active profile")
    let custom = home.appendingPathComponent("custom")
    require(HermesClientConfiguration.configURL(home: home, overrideHome: custom.path) == custom.appendingPathComponent("config.yaml"), "Hermes explicit home takes precedence over sticky profile")
    require(HermesClientConfiguration.configURL(home: home, overrideHome: "~/custom") == custom.appendingPathComponent("config.yaml"), "Hermes expands a custom home under the supplied user's home")
    try "../../outside".write(to: root.appendingPathComponent("active_profile"), atomically: true, encoding: .utf8)
    require(HermesClientConfiguration.configURL(home: home) == root.appendingPathComponent("config.yaml"), "Hermes profile selector cannot escape its profile directory")
    // File-only detection never invokes an interactive Hermes CLI.
    require(MCPClientRegistry.installed(fileManager: manager).first(where: { $0.id == "hermes" })?.configURL == root.appendingPathComponent("config.yaml"), "Hermes profile directory is enough to offer automatic setup")
    require(MCPClientRegistry.descriptor(id: "hermes")?.kind == .automatic, "Hermes uses the automatic client UI")
    let actual = directory.appendingPathComponent("dotfiles.yaml")
    try "model: hermes\n".write(to: actual, atomically: true, encoding: .utf8)
    let link = root.appendingPathComponent("config.yaml")
    try manager.createSymbolicLink(at: link, withDestinationURL: actual)
    let quotedCommand = "/path/with \"quotes\"/mail\\server"
    try HermesClientConfiguration.set(at: link, commandPath: quotedCommand, token: token)
    let value = try Yams.load(yaml: String(contentsOf: actual, encoding: .utf8)) as! [String: Any]
    let server = (value["mcp_servers"] as! [String: Any])["torromail"] as! [String: Any]
    require(server["command"] as? String == quotedCommand, "Hermes YAML escapes executable paths")
    require((try? manager.destinationOfSymbolicLink(atPath: link.path)) == actual.path, "Hermes setup preserves a config symlink")
    let mode = try manager.attributesOfItem(atPath: actual.path)[.posixPermissions] as? Int
    require(mode == 0o600, "Hermes symlink target is private")
    try HermesClientConfiguration.remove(at: link)
    require((try? manager.destinationOfSymbolicLink(atPath: link.path)) == actual.path, "Hermes disconnect preserves a config symlink")
    require(!HermesClientConfiguration.isConfigured(at: link), "Hermes disconnect removes its entry")
    let dangling = directory.appendingPathComponent("dangling.yaml")
    let missing = directory.appendingPathComponent("missing.yaml")
    try manager.createSymbolicLink(at: dangling, withDestinationURL: missing)
    do {
        try HermesClientConfiguration.set(at: dangling, commandPath: command, token: token)
        require(false, "Hermes must not replace a dangling symlink")
    } catch let failure as MCPClientSetup.Failure {
        require(failure.reason == "The existing configuration could not be read.", "Hermes reports a dangling symlink")
    }
    require((try? manager.destinationOfSymbolicLink(atPath: dangling.path)) == missing.path, "Hermes leaves a dangling symlink untouched")
}
