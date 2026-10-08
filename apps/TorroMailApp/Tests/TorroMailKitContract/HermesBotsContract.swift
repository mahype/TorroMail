import Foundation
import TorroMailKit
import Yams

func hermesBotsContract() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("torromail-bots-\(UUID().uuidString)")
    defer { try? FileManager.default.removeItem(at: directory) }
    let root = directory.appendingPathComponent(".hermes")
    let state = directory.appendingPathComponent("TorroMail")
    try FileManager.default.createDirectory(at: state, withIntermediateDirectories: true)
    for name in ["rechnungen", "tagesplaner", ".deleted"] {
        try FileManager.default.createDirectory(at: root.appendingPathComponent("profiles/\(name)"), withIntermediateDirectories: true)
    }
    let defaultConfig = root.appendingPathComponent("config.yaml")
    let invoiceConfig = root.appendingPathComponent("profiles/rechnungen/config.yaml")
    let dayConfig = root.appendingPathComponent("profiles/tagesplaner/config.yaml")
    let original = "# Keep comments\nmodel: chosen\nmcp_servers:\n  other:\n    command: keep\n  torromail:\n    command: old\n    enabled: false\n    tools:\n      include: [mail_search]\n    env:\n      TORROMAIL_TOKEN: stale\n      EXTRA: kept\n"
    for url in [defaultConfig, invoiceConfig, dayConfig] { try original.write(to: url, atomically: true, encoding: .utf8) }
    try "display_name: Rechnungsbot\n".write(to: root.appendingPathComponent("profiles/rechnungen/profile.yaml"), atomically: true, encoding: .utf8)
    try "tagesplaner\n".write(to: root.appendingPathComponent("active_profile"), atomically: true, encoding: .utf8)
    let outside = directory.appendingPathComponent("outside")
    try FileManager.default.createDirectory(at: outside, withIntermediateDirectories: true)
    try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("profiles/escaped"), withDestinationURL: outside)
    let homes = try HermesBots.profileHomes(root: root)
    require(homes.map { $0.0 } == ["default", "rechnungen", "tagesplaner"], "bot discovery ignores tombstones and escaping directory symlinks")
    require(!HermesBots.validProfile("../other") && !HermesBots.validProfile("-options") && !HermesBots.validProfile("a/b"), "profile selectors cannot become paths or command options")
    require(HermesBots.root(home: directory, overrideHome: root.appendingPathComponent("profiles/rechnungen").path).path == root.path, "profile-shaped HERMES_HOME enumerates its installation")
    let invoiceID = HermesBots.clientID(root: root, profile: "rechnungen")
    let dayID = HermesBots.clientID(root: root, profile: "tagesplaner")
    require(invoiceID != dayID && invoiceID != HermesBots.clientID(root: outside, profile: "rechnungen"), "bot identities distinguish both profiles and installations")
    let policyURL = state.appendingPathComponent("policy.json")
    let accessURL = state.appendingPathComponent("client-account-access.json")
    let baseline: [String: Any] = [
        "accounts": [["id": "work", "name": "Work", "email": "work@example.test"], ["id": "private", "name": "Private", "email": "private@example.test"]],
        "oauth": ["google_client_id": "keep-registration"],
        "clients": [["id": "hermes", "name": "Hermes", "token_sha256": MCPClientKeyStore.sha256Hex("old-legacy"), "account_access": ClientAccountAccess.selected(["work"]).policyObject],
                    ["id": "other", "name": "Other", "token_sha256": MCPClientKeyStore.sha256Hex("other"), "account_access": ClientAccountAccess.all.policyObject]]
    ]
    try JSONSerialization.data(withJSONObject: baseline, options: [.sortedKeys]).write(to: policyURL)
    let keys = KeyStorageProbe()
    let storage = probeStorage(keys)
    var probed: [String] = []
    let controller = HermesBotControl(root: root, policyURL: policyURL, command: URL(fileURLWithPath: "/bin/echo"), storage: storage, probe: { _, token in probed.append(token) })
    let beforeList = try Data(contentsOf: policyURL)
    let first = try controller.run(HermesControlRequest(action: .list))
    require(first.bots.map(\.id) == ["default", "rechnungen", "tagesplaner"] && first.accounts.count == 2, "discovery returns the accounts of this TorroMail instance")
    require(first.bots[1].name == "Rechnungsbot" && first.bots.allSatisfy { $0.access == .selected(["work"]) }, "legacy bot grants migrate without widening access")
    require(first.bots.allSatisfy { !$0.authorized && $0.problem != nil }, "a stale configured key is an actionable error")
    require(keys.stored.isEmpty && !FileManager.default.fileExists(atPath: accessURL.path) && (try? Data(contentsOf: policyURL)) == beforeList, "listing never mints keys or migrates grants on disk")

    let invoice = HermesBotSelection(id: "rechnungen", access: .selected(["work"]))
    let connectionURL = state.appendingPathComponent("connections.jsonl")
    let reloadURL = state.appendingPathComponent("hermes-reload.json")
    func handshake(_ name: String) throws -> Data {
        var line = try JSONSerialization.data(withJSONObject: ["client_id": invoiceID, "client_name": name, "ts": Date().timeIntervalSince1970])
        line.append(0x0A)
        return line
    }
    let oldConnection = try handshake("hermes")
    try oldConnection.write(to: connectionURL)
    let installed = try controller.run(HermesControlRequest(action: .connect, selections: [invoice]))
    require(installed.results.first?.success == true && installed.results.first?.needsReload == true, "connect verifies an authenticated call and reports a new key")
    require(installed.bots[1].needsReload, "a previous handshake cannot clear a new key's reload notice")
    let invoiceKey = HermesClientConfiguration.token(at: invoiceConfig)!
    require(probed == [invoiceKey] && keys.stored["client-key-\(invoiceID)"] == invoiceKey, "probe uses exactly the profile's key")
    require((try? String(contentsOf: defaultConfig, encoding: .utf8)) == original && (try? String(contentsOf: dayConfig, encoding: .utf8)) == original, "unselected bots are unchanged")
    let parsed = try Yams.load(yaml: String(contentsOf: invoiceConfig, encoding: .utf8)) as! [String: Any]
    let entry = (parsed["mcp_servers"] as! [String: Any])["torromail"] as! [String: Any]
    require(entry["enabled"] as? Bool == true && (entry["env"] as? [String: Any])?["EXTRA"] as? String == "kept" && entry["tools"] != nil, "repair enables TorroMail while preserving custom environment and tool selection")
    require((try? String(contentsOf: invoiceConfig, encoding: .utf8))?.contains("# Keep comments") == true, "bot setup retains ordinary YAML comments")
    let again = try controller.run(HermesControlRequest(action: .connect, selections: [invoice]))
    require(HermesClientConfiguration.token(at: invoiceConfig) == invoiceKey && again.results.first?.needsReload == false, "repeated setup keeps a working key stable")
    require(again.bots[1].needsReload, "repeated setup preserves the pending reload notice")
    var log = oldConnection
    log.append(try handshake("torromail-setup-check"))
    log.append(Data("unfinished log entry\n".utf8))
    try log.write(to: connectionURL)
    let tested = try controller.run(HermesControlRequest(action: .test, selections: [invoice]))
    require(tested.results.first?.success == true && tested.bots[1].needsReload, "setup probes and torn log lines cannot claim that Hermes has reloaded")
    log.append(try handshake("hermes"))
    try log.write(to: connectionURL)
    let reloaded = try controller.run(HermesControlRequest(action: .list))
    require(!reloaded.bots[1].needsReload, "a subsequent attributed Hermes connection clears the reload notice")

    // The second bot receives a distinct key, and an explicit empty selection.
    _ = try controller.run(HermesControlRequest(action: .connect, selections: [HermesBotSelection(id: "tagesplaner", access: .selected([]))]))
    let dayKey = HermesClientConfiguration.token(at: dayConfig)!
    require(dayKey != invoiceKey, "bots never share newly minted keys")
    let doc = try JSONSerialization.jsonObject(with: Data(contentsOf: policyURL)) as! [String: Any]
    require((doc["oauth"] as? [String: Any])?["google_client_id"] as? String == "keep-registration", "bot pairing preserves account facts and OAuth registrations")
    let entries = doc["clients"] as! [[String: Any]]
    require(entries.first(where: { $0["id"] as? String == dayID })?["account_access"] as? NSDictionary == ClientAccountAccess.selected([]).policyObject as NSDictionary, "no accounts means no grant, never all accounts")

    let oldPolicy = try Data(contentsOf: policyURL), oldAccess = try Data(contentsOf: accessURL), oldYAML = try Data(contentsOf: defaultConfig), oldReload = try Data(contentsOf: reloadURL)
    let failing = HermesBotControl(root: root, policyURL: policyURL, command: URL(fileURLWithPath: "/bin/echo"), storage: storage, probe: { _, _ in throw MCPClientSetup.Failure("The authenticated TorroMail connection test failed.") })
    let failure = try failing.run(HermesControlRequest(action: .connect, selections: [HermesBotSelection(id: "default", access: .all)]))
    require(failure.results.first?.success == false && keys.stored["client-key-\(HermesBots.clientID(root: root, profile: "default"))"] == nil, "a failed probe removes the unused new key")
    require((try? Data(contentsOf: policyURL)) == oldPolicy && (try? Data(contentsOf: accessURL)) == oldAccess && (try? Data(contentsOf: defaultConfig)) == oldYAML, "failed setup restores policy, grants and original config bytes")
    require((try? Data(contentsOf: reloadURL)) == oldReload, "failed setup restores pending reload notices too")
    let unknown = try controller.run(HermesControlRequest(action: .connect, selections: [HermesBotSelection(id: "default", access: .selected(["removed"]))]))
    require(unknown.results.first?.success == false && (try? Data(contentsOf: policyURL)) == oldPolicy, "removed accounts cannot be granted")
    let traversal = try controller.run(HermesControlRequest(action: .connect, selections: [HermesBotSelection(id: "../outside", access: .all)]))
    require(traversal.results.first?.success == false, "selection must still be present in the current bot roster")

    try "mcp_servers: [broken".write(to: defaultConfig, atomically: true, encoding: .utf8)
    let broken = try controller.run(HermesControlRequest(action: .connect, selections: [HermesBotSelection(id: "default", access: .all)]))
    require(broken.results.first?.success == false && (try? String(contentsOf: defaultConfig, encoding: .utf8)) == "mcp_servers: [broken", "invalid YAML is not overwritten")
    let disconnected = try controller.run(HermesControlRequest(action: .disconnect, selections: [invoice]))
    require(disconnected.results.first?.success == true && !HermesClientConfiguration.isConfigured(at: invoiceConfig) && keys.stored["client-key-\(invoiceID)"] == nil, "disconnect revokes only that bot")
    require(HermesClientConfiguration.token(at: dayConfig) == dayKey && keys.stored["client-key-\(dayID)"] == dayKey, "disconnect does not disturb another bot")

    // The desktop target selector follows an explicit SSH target, and refuses
    // to silently configure local Hermes for an unrelated remote URL.
    let desktop = directory.appendingPathComponent("Library/Application Support/Hermes")
    try FileManager.default.createDirectory(at: desktop, withIntermediateDirectories: true)
    let desktopURL = desktop.appendingPathComponent("connections.json")
    let ssh: [String: Any] = ["id": "mini", "kind": "ssh", "host": "127.0.0.1", "user": "fixture", "label": "Mac mini", "keyPath": "~/.ssh/fixture"]
    var connectionDoc: [String: Any] = ["primary": "mini", "connections": [ssh]]
    try JSONSerialization.data(withJSONObject: connectionDoc).write(to: desktopURL)
    require((try? HermesConnections.discover(home: directory))?.preferredID == "mini", "saved SSH target is preferred over local config")
    connectionDoc = ["primary": "url", "connections": [ssh, ["id": "url", "kind": "url", "url": "http://127.0.0.1:9119"]]]
    try JSONSerialization.data(withJSONObject: connectionDoc).write(to: desktopURL)
    require((try? HermesConnections.discover(home: directory))?.preferredID == "mini", "URL backend is matched to SSH by its network address")
    connectionDoc = ["primary": "url", "connections": [ssh, ["id": "url", "kind": "url", "url": "http://192.0.2.99:9119"]]]
    try JSONSerialization.data(withJSONObject: connectionDoc).write(to: desktopURL)
    let unmatched = try HermesConnections.discover(home: directory)
    require(unmatched.preferredID == nil && unmatched.remoteWithoutSSH, "unmatched remote backend requires an explicit target")

    try "{\"broken\":{\"mode\":\"unknown\"}}".write(to: accessURL, atomically: true, encoding: .utf8)
    var invalidGrantsFailed = false
    do { _ = try controller.run(HermesControlRequest(action: .list)) } catch { invalidGrantsFailed = true }
    require(invalidGrantsFailed, "malformed grants fail closed even on discovery")
}
