import Foundation

/// A headless setup operation, never an MCP tool. The policy on this computer
/// supplies both the account picker and the allowlist; remote keys stay here.
public struct HermesBotControl {
    public let root: URL
    public let policyURL: URL
    public let command: URL
    public let storage: MCPClientKeyStore.Storage
    private let probe: (URL, String) throws -> Void

    public init(root: URL, policyURL: URL, command: URL, storage: MCPClientKeyStore.Storage = .keychain,
                probe: ((URL, String) throws -> Void)? = nil) {
        self.root = root
        self.policyURL = policyURL
        self.command = command
        self.storage = storage
        self.probe = probe ?? HermesProcess.probe
    }

    public func run(_ request: HermesControlRequest) throws -> HermesControlResponse {
        try HermesControlLock.withLock(directory: policyURL.deletingLastPathComponent()) {
            guard Set(request.selections.map(\.id)).count == request.selections.count, request.selections.count <= 100 else {
                throw MCPClientSetup.Failure("Invalid Hermes bot selection.")
            }
            var results: [HermesBotResult] = []
            if request.action != .list {
                guard !request.selections.isEmpty else { throw MCPClientSetup.Failure("Select at least one Hermes bot.") }
                for selection in request.selections {
                    do {
                        let reloaded = try apply(request.action, selection: selection)
                        results.append(HermesBotResult(id: selection.id, success: true, needsReload: reloaded, problem: nil))
                    } catch {
                        results.append(HermesBotResult(id: selection.id, success: false, needsReload: false, problem: Self.reason(error)))
                    }
                }
            }
            var response = try list()
            response.results = results
            return response
        }
    }

    private func document() throws -> [String: Any] {
        guard let data = try? Data(contentsOf: policyURL),
              let doc = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let accounts = doc["accounts"] as? [[String: Any]],
              accounts.allSatisfy({ $0["id"] is String }),
              let clients = doc["clients"] as? [[String: Any]] else {
            throw MCPClientSetup.Failure("The TorroMail configuration could not be read. Open TorroMail on this computer first.")
        }
        // Refuse malformed grants rather than silently treating them as all.
        for client in clients {
            guard client["id"] is String, client["name"] is String, client["token_sha256"] is String else {
                throw MCPClientSetup.Failure("The existing configuration could not be read.")
            }
            if let access = client["account_access"] { _ = try decodeAccess(access) }
        }
        return doc
    }

    private var accessURL: URL { policyURL.deletingLastPathComponent().appendingPathComponent("client-account-access.json") }
    private var reloadURL: URL { policyURL.deletingLastPathComponent().appendingPathComponent("hermes-reload.json") }
    private var connectionURL: URL { policyURL.deletingLastPathComponent().appendingPathComponent("connections.jsonl") }

    private struct ReloadMarker: Codable {
        let logOffset: Int
        let changedAt: TimeInterval
    }

    private func reloadMarkers() throws -> [String: ReloadMarker] {
        guard FileManager.default.fileExists(atPath: reloadURL.path) else { return [:] }
        return try JSONDecoder().decode([String: ReloadMarker].self, from: Data(contentsOf: reloadURL))
    }

    private func needsReload(_ marker: ReloadMarker?, clientID: String, log: Data) -> Bool {
        guard let marker else { return false }
        // The server records an attributed handshake only for an allowed key.
        // Setup probes must never clear a notice meant for the real client.
        let rotated = log.count < marker.logOffset
        let suffix = rotated ? log : Data(log.dropFirst(max(0, marker.logOffset)))
        for line in suffix.split(separator: 0x0A) {
            guard let row = try? JSONSerialization.jsonObject(with: Data(line)) as? [String: Any],
                  row["client_id"] as? String == clientID,
                  let name = row["client_name"] as? String, !name.isEmpty, name != "torromail-setup-check",
                  let timestamp = row["ts"] as? Double,
                  !rotated || timestamp >= marker.changedAt.rounded(.up) else { continue }
            return false
        }
        return true
    }

    private func decodeAccess(_ object: Any) throws -> ClientAccountAccess {
        try JSONDecoder().decode(ClientAccountAccess.self, from: JSONSerialization.data(withJSONObject: object))
    }

    private func grants(document: [String: Any]) throws -> [String: ClientAccountAccess] {
        if FileManager.default.fileExists(atPath: accessURL.path) {
            return try JSONDecoder().decode([String: ClientAccountAccess].self, from: Data(contentsOf: accessURL))
        }
        var result: [String: ClientAccountAccess] = [:]
        for client in document["clients"] as? [[String: Any]] ?? [] {
            let id = client["id"] as! String
            result[id] = try client["account_access"].map(decodeAccess) ?? .all
        }
        return result
    }

    private func existingAccess(clientID: String, token: String?, document: [String: Any]) throws -> ClientAccountAccess {
        let entries = document["clients"] as? [[String: Any]] ?? []
        let saved = try grants(document: document)
        if let value = saved[clientID] { return value }
        // Migration copies grants, never keys. A legacy Hermes entry is one
        // shared identity; each opted-in profile gets its own identity now.
        let previous = entries.first { $0["id"] as? String == clientID }
            ?? entries.first { token != nil && $0["token_sha256"] as? String == MCPClientKeyStore.sha256Hex(token!) }
            ?? entries.first { $0["id"] as? String == "hermes" }
        guard let previous else { return .selected([]) }
        let oldID = previous["id"] as! String
        return try saved[oldID] ?? previous["account_access"].map(decodeAccess) ?? .all
    }

    private func list() throws -> HermesControlResponse {
        let doc = try document()
        let clients = doc["clients"] as? [[String: Any]] ?? []
        // Validate the separate grants file even when all bots are new.
        _ = try grants(document: doc)
        let markers = try reloadMarkers()
        let connectionLog = (try? Data(contentsOf: connectionURL)) ?? Data()
        var response = HermesControlResponse()
        response.accounts = (doc["accounts"] as? [[String: Any]] ?? []).map {
            HermesSharedAccount(id: $0["id"] as! String, name: $0["name"] as? String ?? "", email: $0["email"] as? String ?? "")
        }
        for (profile, home) in try HermesBots.profileHomes(root: root) {
            let id = HermesBots.clientID(root: root, profile: profile)
            let url = home.appendingPathComponent("config.yaml")
            do {
                let entry = try HermesClientConfiguration.definition(at: url)
                let token = HermesClientConfiguration.token(at: url)
                let authorized = token.map { key in clients.contains { $0["token_sha256"] as? String == MCPClientKeyStore.sha256Hex(key) } } ?? false
                let problem: String?
                if entry != nil && !authorized { problem = "TorroMail refused this bot’s access key." }
                else if let entry, entry["enabled"] as? Bool == false || entry["command"] as? String != command.path {
                    problem = "This bot’s TorroMail server entry needs to be repaired."
                } else { problem = nil }
                response.bots.append(HermesBot(id: profile, name: HermesBots.displayName(profile: profile, home: home), clientID: id,
                    configured: entry != nil, authorized: authorized, access: try existingAccess(clientID: id, token: token, document: doc),
                    problem: problem, needsReload: needsReload(markers[id], clientID: id, log: connectionLog)))
            } catch {
                response.bots.append(HermesBot(id: profile, name: HermesBots.displayName(profile: profile, home: home), clientID: id,
                    configured: false, authorized: false, access: .selected([]), problem: Self.reason(error)))
            }
        }
        return response
    }

    private func apply(_ action: HermesControlRequest.Action, selection: HermesBotSelection) throws -> Bool {
        guard let (_, home) = try HermesBots.profileHomes(root: root).first(where: { $0.0 == selection.id }) else {
            throw MCPClientSetup.Failure("This Hermes bot no longer exists. Refresh the bot list.")
        }
        let configURL = home.appendingPathComponent("config.yaml")
        let existingDefinition = try HermesClientConfiguration.definition(at: configURL) // Preflight before touching keys.
        let clientID = HermesBots.clientID(root: root, profile: selection.id)
        var doc = try document()
        if action == .test {
            guard existingDefinition?["command"] as? String == command.path, existingDefinition?["enabled"] as? Bool != false,
                  (existingDefinition?["args"] as? [String] ?? []).isEmpty else {
                throw MCPClientSetup.Failure("This bot’s TorroMail server entry needs to be repaired.")
            }
            guard let token = HermesClientConfiguration.token(at: configURL) else { throw MCPClientSetup.Failure("This bot is not configured yet.") }
            try probe(command, token)
            return false
        }
        var access = try grants(document: doc)
        var markers = try reloadMarkers()
        let knownAccounts = Set((doc["accounts"] as? [[String: Any]] ?? []).compactMap { $0["id"] as? String })
        if case .selected(let ids) = selection.access, !ids.isSubset(of: knownAccounts) {
            throw MCPClientSetup.Failure("A selected mail account no longer exists. Refresh the bot list.")
        }
        var clients = doc["clients"] as? [[String: Any]] ?? []
        let keyAccount = "client-key-\(clientID)"
        let previousKey = storage.read(keyAccount)
        let previousConfigToken = HermesClientConfiguration.token(at: configURL)
        let pairingURL = policyURL.deletingLastPathComponent().appendingPathComponent("clients.json")
        if FileManager.default.fileExists(atPath: pairingURL.path) {
            guard let saved = try JSONSerialization.jsonObject(with: Data(contentsOf: pairingURL)) as? [String: Any],
                  saved["clients"] is [[String: Any]] else { throw MCPClientSetup.Failure("The existing configuration could not be read.") }
        }
        let paths = [configURL.resolvingSymlinksInPath(), accessURL, policyURL, reloadURL]
            + (FileManager.default.fileExists(atPath: pairingURL.path) ? [pairingURL] : [])
        let snapshots = try paths.map { try FileSnapshot($0.resolvingSymlinksInPath()) }
        var touched: [FileSnapshot] = []
        func record(_ url: URL) { if let snapshot = snapshots.first(where: { $0.url == url.resolvingSymlinksInPath() }) { touched.append(snapshot) } }
        do {
            if action == .connect {
                guard FileManager.default.isExecutableFile(atPath: command.path) else { throw MCPClientSetup.Failure("The MCP server binary was not found.") }
                let token = try MCPClientKeyStore.tokenCreatingIfNeeded(forClient: clientID, storage: storage)
                if token != previousConfigToken {
                    markers[clientID] = ReloadMarker(logOffset: (try? Data(contentsOf: connectionURL).count) ?? 0,
                                                     changedAt: Date().timeIntervalSince1970)
                }
                clients.removeAll { $0["id"] as? String == clientID }
                clients.append(["id": clientID, "name": "Hermes · \(HermesBots.displayName(profile: selection.id, home: home))",
                                "token_sha256": MCPClientKeyStore.sha256Hex(token), "account_access": selection.access.policyObject])
                access[clientID] = selection.access
                try HermesClientConfiguration.set(at: configURL, commandPath: command.path, token: token)
                record(configURL)
            } else {
                // Also remove a legacy-configured profile, but never revoke a
                // shared legacy key that another unselected bot may still use.
                try HermesClientConfiguration.remove(at: configURL)
                record(configURL)
                clients.removeAll { $0["id"] as? String == clientID }
                access[clientID] = nil
                markers[clientID] = nil
            }
            try Self.write(try JSONEncoder().encode(markers), to: reloadURL); record(reloadURL)
            try Self.write(try JSONEncoder().encode(access), to: accessURL); record(accessURL)
            doc["clients"] = clients
            if paths.contains(pairingURL) {
                try Self.write(try JSONSerialization.data(withJSONObject: ["version": 1, "clients": clients], options: [.sortedKeys]), to: pairingURL)
                record(pairingURL)
            }
            try Self.write(try JSONSerialization.data(withJSONObject: doc, options: [.prettyPrinted, .sortedKeys]), to: policyURL); record(policyURL)
            if action == .connect {
                let token = HermesClientConfiguration.token(at: configURL)!
                try probe(command, token)
                return token != previousConfigToken
            }
            storage.delete(keyAccount)
            return false
        } catch {
            // A failed probe is a failed setup, not a green configuration.
            var restorationFailed = false
            for snapshot in touched.reversed() { do { try snapshot.restore() } catch { restorationFailed = true } }
            if action == .connect, storage.read(keyAccount) != previousKey {
                storage.delete(keyAccount)
                if let previousKey { do { try storage.write(previousKey, keyAccount) } catch { restorationFailed = true } }
            }
            if restorationFailed { throw MCPClientSetup.Failure("Setup failed and could not be fully restored. Check this bot’s configuration before retrying.") }
            throw error
        }
    }

    static func reason(_ error: Error) -> String {
        if let failure = error as? KeychainStore.Failure {
            NSLog("TorroMail Hermes keychain operation failed (status %d)", failure.status)
            return "The keychain is not available in this login session. Open TorroMail on the target computer and try again."
        }
        return (error as? MCPClientSetup.Failure)?.reason ?? "The existing configuration could not be read."
    }

    static func write(_ data: Data, to url: URL) throws {
        let target = url.resolvingSymlinksInPath()
        let temporary = target.deletingLastPathComponent().appendingPathComponent(".torromail-control-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: temporary) }
        guard FileManager.default.createFile(atPath: temporary.path, contents: data, attributes: [.posixPermissions: 0o600]) else {
            throw MCPClientSetup.Failure("Could not update the configuration.")
        }
        guard rename(temporary.path, target.path) == 0 else { throw MCPClientSetup.Failure("Could not update the configuration.") }
    }

    private struct FileSnapshot {
        let url: URL
        let data: Data?
        init(_ url: URL) throws {
            self.url = url
            do { data = try Data(contentsOf: url) }
            catch CocoaError.fileReadNoSuchFile { data = nil }
        }
        func restore() throws {
            if let data { try HermesBotControl.write(data, to: url) }
            else if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
        }
    }
}
