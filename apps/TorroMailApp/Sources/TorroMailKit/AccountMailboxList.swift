import Foundation

/// Reads the account's actual IMAP folders for the setup pickers. The server
/// performs the login behind the app's pairing key and returns wire names;
/// those exact names are later published as manual choices.
public enum AccountMailboxList {
    public struct Failure: Error, Sendable {
        public let reason: String
    }

    /// IMAP4rev1 uses modified UTF-7 on the wire. The picker shows readable
    /// text but keeps the original name as its value for later IMAP commands.
    public static func displayName(_ wireName: String) -> String {
        var output = ""
        var rest = wireName[...]
        while let ampersand = rest.firstIndex(of: "&") {
            output += rest[..<ampersand]
            let shifted = rest[rest.index(after: ampersand)...]
            guard let end = shifted.firstIndex(of: "-") else {
                output += rest[ampersand...]
                return output
            }
            let encoded = String(shifted[..<end])
            if encoded.isEmpty {
                output += "&"
            } else {
                let base64 = encoded.replacingOccurrences(of: ",", with: "/")
                let padded = base64 + String(repeating: "=", count: (4 - base64.count % 4) % 4)
                if let bytes = Data(base64Encoded: padded),
                   let decoded = String(data: bytes, encoding: .utf16BigEndian) {
                    output += decoded
                } else {
                    output += "&\(encoded)-"
                }
            }
            rest = shifted[shifted.index(after: end)...]
        }
        output += rest
        return output
    }

    public static func load(
        accountID: String,
        executableName: String,
        timeout: TimeInterval = 30
    ) -> Result<[String], Failure> {
        let locator = MCPExecutableLocator(
            executableName: executableName,
            workspaceRoot: FileManager.default.currentDirectoryPath
        )
        guard let command = locator.resolve() else {
            return .failure(Failure(reason: "MCP executable not found"))
        }
        guard let token = try? MCPClientKeyStore.appToken() else {
            return .failure(Failure(reason: "app access key unavailable"))
        }

        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments + ["--list-account-mailboxes", accountID]
        var environment = ProcessInfo.processInfo.environment
        if let url = try? PolicyDocument.defaultURL() {
            environment["TORROMAIL_POLICY_PATH"] = url.path
        }
        environment["TORROMAIL_TOKEN"] = token
        process.environment = environment

        let outputPipe = Pipe()
        let errorPipe = Pipe()
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        let finished = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in finished.signal() }
        do {
            try process.run()
        } catch {
            return .failure(Failure(reason: error.localizedDescription))
        }

        let output = MailboxPipeCapture(limit: 1024 * 1024)
        let errors = MailboxPipeCapture(limit: 8 * 1024)
        let drained = DispatchGroup()
        for (capture, handle) in [
            (output, outputPipe.fileHandleForReading),
            (errors, errorPipe.fileHandleForReading)
        ] {
            drained.enter()
            DispatchQueue.global(qos: .utility).async {
                capture.consume(handle)
                drained.leave()
            }
        }

        if finished.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            return .failure(Failure(reason: "folder listing timed out"))
        }
        _ = drained.wait(timeout: .now() + 2)
        if process.terminationStatus != 0 {
            return .failure(Failure(reason: errors.text))
        }
        guard let names = try? JSONDecoder().decode([String].self, from: output.data) else {
            return .failure(Failure(reason: "invalid folder list from MCP server"))
        }
        return .success(names)
    }
}

private final class MailboxPipeCapture: @unchecked Sendable {
    let limit: Int
    private let lock = NSLock()
    private var bytes = Data()

    init(limit: Int) {
        self.limit = limit
    }

    func consume(_ handle: FileHandle) {
        while true {
            let chunk = handle.availableData
            if chunk.isEmpty { return }
            lock.lock()
            if bytes.count < limit {
                bytes.append(chunk.prefix(limit - bytes.count))
            }
            lock.unlock()
        }
    }

    var data: Data {
        lock.lock()
        defer { lock.unlock() }
        return bytes
    }

    var text: String { String(decoding: data, as: UTF8.self) }
}
