import Foundation
import Darwin

public struct HermesConnection: Identifiable, Hashable, Sendable {
    public let id: String
    public let name: String
    public let host: String?
    public let user: String?
    public let keyPath: String?
    public var isRemote: Bool { host != nil }

    public init(id: String, name: String, host: String? = nil, user: String? = nil, keyPath: String? = nil) {
        self.id = id; self.name = name; self.host = host; self.user = user; self.keyPath = keyPath
    }
}

public struct HermesConnectionDiscovery: Sendable {
    public let connections: [HermesConnection]
    public let preferredID: String?
    public let remoteWithoutSSH: Bool
}

/// Read only the connection facts from Hermes Desktop. Login tokens, cookies,
/// and chat data are neither imported nor needed for SSH-based setup.
public enum HermesConnections {
    public static func discover(home: URL = FileManager.default.homeDirectoryForCurrentUser) throws -> HermesConnectionDiscovery {
        let directory = home.appendingPathComponent("Library/Application Support/Hermes")
        let url = directory.appendingPathComponent("connections.json")
        let local = HermesConnection(id: "local", name: "This computer")
        guard FileManager.default.fileExists(atPath: url.path) else {
            let old = directory.appendingPathComponent("connection.json")
            if let data = try? Data(contentsOf: old), let doc = try? JSONSerialization.jsonObject(with: data) as? [String: Any], doc["mode"] as? String == "remote" {
                return HermesConnectionDiscovery(connections: [local], preferredID: nil, remoteWithoutSSH: true)
            }
            return HermesConnectionDiscovery(connections: [local], preferredID: local.id, remoteWithoutSSH: false)
        }
        guard let doc = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any],
              let entries = doc["connections"] as? [[String: Any]] else { throw MCPClientSetup.Failure("The Hermes connections could not be read.") }
        var connections = [local]
        var seen = Set([local.id])
        for entry in entries where entry["kind"] as? String == "ssh" {
            guard let id = entry["id"] as? String, seen.insert(id).inserted,
                  let host = entry["host"] as? String, validHost(host),
                  let user = entry["user"] as? String, validUser(user) else { continue }
            let name = entry["label"] as? String ?? host
            let key = entry["keyPath"] as? String
            connections.append(HermesConnection(id: id, name: name, host: host, user: user, keyPath: key))
        }
        let primary = doc["primary"] as? String ?? doc["lastUsed"] as? String
        if let primary, connections.contains(where: { $0.id == primary }) {
            return HermesConnectionDiscovery(connections: connections, preferredID: primary, remoteWithoutSSH: false)
        }
        // A URL-connected desktop can still have an SSH entry for the same
        // backend. Require an unambiguous hostname/address match; never pick
        // an arbitrary machine simply because it is the only saved SSH host.
        if let entry = entries.first(where: { $0["id"] as? String == primary }),
           let rawURL = entry["url"] as? String, let host = URL(string: rawURL)?.host {
            let addresses = resolvedAddresses(host)
            let matches = connections.filter { connection in
                guard let candidate = connection.host else { return false }
                return host.lowercased() == candidate.lowercased() || !addresses.intersection(resolvedAddresses(candidate)).isEmpty
            }
            if matches.count == 1 { return HermesConnectionDiscovery(connections: connections, preferredID: matches[0].id, remoteWithoutSSH: false) }
        }
        return HermesConnectionDiscovery(connections: connections, preferredID: nil, remoteWithoutSSH: true)
    }

    public static func run(_ request: HermesControlRequest, on connection: HermesConnection, helperURL: URL? = nil) throws -> HermesControlResponse {
        let executable: URL
        let arguments: [String]
        let timeout: TimeInterval = request.action == .list ? 30 : Double(max(1, request.selections.count)) * 20 + 15
        if let host = connection.host, let user = connection.user {
            guard validHost(host), validUser(user) else { throw MCPClientSetup.Failure("The Hermes connections could not be read.") }
            executable = URL(fileURLWithPath: "/usr/bin/ssh")
            var ssh = ["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2"]
            if let key = connection.keyPath, !key.isEmpty {
                ssh += ["-i", NSString(string: key).expandingTildeInPath]
            }
            // Only a fixed command crosses the remote shell. Profile names,
            // grants and keys never become shell command text.
            arguments = ssh + ["--", "\(user)@\(host)", "/Applications/TorroMail.app/Contents/MacOS/TorroMailHermesControl --gui-session"]
        } else {
            executable = helperURL ?? Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/TorroMailHermesControl")
            guard FileManager.default.isExecutableFile(atPath: executable.path) else { throw MCPClientSetup.Failure("The Hermes setup helper was not found. Update TorroMail on this computer.") }
            arguments = []
        }
        let data = try HermesProcess.run(executable: executable, arguments: arguments, input: JSONEncoder().encode(request), timeout: timeout)
        guard let response = try? JSONDecoder().decode(HermesControlResponse.self, from: data), response.version == 1 else {
            throw MCPClientSetup.Failure("The Hermes setup helper returned an invalid response. Update TorroMail on the target computer.")
        }
        if let problem = response.problem { throw MCPClientSetup.Failure(problem) }
        return response
    }

    private static func validHost(_ value: String) -> Bool {
        !value.isEmpty && !value.hasPrefix("-") && value.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || ".-:[]".contains($0)) }
    }
    private static func validUser(_ value: String) -> Bool {
        !value.isEmpty && !value.hasPrefix("-") && value.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || "_.-".contains($0)) }
    }

    private static func resolvedAddresses(_ host: String) -> Set<String> {
        var head: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(host, nil, nil, &head) == 0, let first = head else { return [] }
        defer { freeaddrinfo(first) }
        var result = Set<String>()
        var current: UnsafeMutablePointer<addrinfo>? = first
        while let node = current {
            let entry = node.pointee
            var name = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(entry.ai_addr, entry.ai_addrlen, &name, socklen_t(name.count), nil, 0, NI_NUMERICHOST) == 0 {
                result.insert(String(decoding: name.prefix(while: { $0 != 0 }).map { UInt8(bitPattern: $0) }, as: UTF8.self))
            }
            current = entry.ai_next
        }
        return result
    }
}
