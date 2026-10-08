import Foundation
import Darwin

/// Bounded subprocess I/O. Keys travel in stdin or environment, never argv or
/// error output. Both output pipes are drained even when a peer misbehaves.
enum HermesProcess {
    static func run(executable: URL, arguments: [String], input: Data, environment: [String: String]? = nil,
                    timeout: TimeInterval = 30) throws -> Data {
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        if let environment { process.environment = environment }
        let stdin = Pipe(), stdout = Pipe(), stderr = Pipe()
        _ = fcntl(stdin.fileHandleForWriting.fileDescriptor, F_SETNOSIGPIPE, 1)
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = stderr
        let finished = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in finished.signal() }
        do { try process.run() }
        catch { throw MCPClientSetup.Failure("Could not start the Hermes setup helper.") }
        let output = PipeCapture(limit: 2 * 1024 * 1024)
        let errors = PipeCapture(limit: 16 * 1024)
        let drained = DispatchGroup()
        for (capture, handle) in [(output, stdout.fileHandleForReading), (errors, stderr.fileHandleForReading)] {
            drained.enter()
            DispatchQueue.global().async { capture.consume(handle); drained.leave() }
        }
        drained.enter()
        DispatchQueue.global().async {
            try? stdin.fileHandleForWriting.write(contentsOf: input)
            try? stdin.fileHandleForWriting.close()
            drained.leave()
        }
        if finished.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            if finished.wait(timeout: .now() + 1) == .timedOut { kill(process.processIdentifier, SIGKILL) }
            _ = finished.wait(timeout: .now() + 1)
            throw MCPClientSetup.Failure("The Hermes setup helper did not respond in time.")
        }
        guard drained.wait(timeout: .now() + 2) == .success, process.terminationStatus == 0 else {
            // A remote shell may echo a malformed config containing secrets.
            throw MCPClientSetup.Failure("Could not reach the Hermes setup helper. Check the SSH connection and update TorroMail on the target computer.")
        }
        return output.data
    }

    static func probe(command: URL, token: String) throws {
        let messages: [[String: Any]] = [
            ["jsonrpc": "2.0", "id": 1, "method": "initialize", "params": ["protocolVersion": "2024-11-05", "capabilities": [:], "clientInfo": ["name": "torromail-setup-check", "version": "1"]]],
            ["jsonrpc": "2.0", "method": "notifications/initialized"],
            ["jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": ["name": "mail_list_accounts", "arguments": [:]]]
        ]
        var input = Data()
        for message in messages { input.append(try JSONSerialization.data(withJSONObject: message)); input.append(0x0A) }
        var env = ProcessInfo.processInfo.environment
        env["TORROMAIL_POLICY_PATH"] = nil
        env["TORROMAIL_TOKEN"] = token
        let data = try run(executable: command, arguments: [], input: input, environment: env, timeout: 15)
        for line in data.split(separator: 0x0A) {
            guard let response = try? JSONSerialization.jsonObject(with: Data(line)) as? [String: Any], response["id"] as? Int == 2 else { continue }
            guard response["error"] == nil, let result = response["result"] as? [String: Any], result["isError"] as? Bool != true else {
                throw MCPClientSetup.Failure("TorroMail refused this bot’s access key.")
            }
            guard let content = result["content"] as? [[String: Any]],
                  let text = content.first(where: { $0["type"] as? String == "text" })?["text"] as? String,
                  let accounts = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any],
                  accounts["accounts"] is [[String: Any]] else {
                throw MCPClientSetup.Failure("The authenticated TorroMail connection test failed.")
            }
            return
        }
        throw MCPClientSetup.Failure("The authenticated TorroMail connection test failed.")
    }
}

/// The app and helper serialize policy publication on the target computer.
enum HermesControlLock {
    static func withLock<T>(directory: URL, _ operation: () throws -> T) throws -> T {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let fd = open(directory.appendingPathComponent(".control.lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw MCPClientSetup.Failure("Could not lock the TorroMail configuration.") }
        defer { close(fd) }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else { throw MCPClientSetup.Failure("TorroMail is updating its configuration. Please try again.") }
        defer { flock(fd, LOCK_UN) }
        return try operation()
    }
}
