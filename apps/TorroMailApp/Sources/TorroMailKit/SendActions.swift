import Foundation
import Combine

public struct SendApproval: Codable, Identifiable, Hashable, Sendable {
    public struct Attachment: Codable, Hashable, Sendable {
        public let filename: String
        public let media_type: String
        public let size_bytes: Int
    }
    public let id: String
    public let accountID: String
    public let code: String
    public let from: String
    public let recipients: [String]
    public let subject: String
    public let body: String
    public let attachments: [Attachment]
    public let expiresAt: UInt64
    public let submissionStatus: String
    public let sentCopyStatus: String
    public var needsApproval: Bool { submissionStatus == "prepared" }
}

/// Both UI approval and MCP confirmation run the Rust submission service.
public enum SendActions {
    public struct Failure: Error, Sendable { public let reason: String }
    public static func request(_ request: [String: String], executableName: String, timeout: TimeInterval = 120) -> Result<Data, Failure> {
        guard let command = MCPExecutableLocator(executableName: executableName, workspaceRoot: FileManager.default.currentDirectoryPath).resolve(),
              let token = try? MCPClientKeyStore.appToken() else {
            return .failure(Failure(reason: "send service unavailable"))
        }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("torromail-send-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: url) }
        do {
            let bytes = try JSONEncoder().encode(request)
            guard FileManager.default.createFile(atPath: url.path, contents: bytes, attributes: [.posixPermissions: 0o600]) else {
                return .failure(Failure(reason: "send request could not be saved"))
            }
        } catch { return .failure(Failure(reason: "invalid send request")) }
        let process = Process()
        process.executableURL = command.executableURL
        process.arguments = command.arguments + ["--send-action", url.path]
        var environment = ProcessInfo.processInfo.environment
        environment["TORROMAIL_TOKEN"] = token
        if let policy = try? PolicyDocument.defaultURL() { environment["TORROMAIL_POLICY_PATH"] = policy.path }
        process.environment = environment
        let outputPipe = Pipe(), errorPipe = Pipe()
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        let finished = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in finished.signal() }
        do { try process.run() } catch { return .failure(Failure(reason: "send service could not be started")) }
        let output = PipeCapture(limit: 64 * 1024 * 1024), errors = PipeCapture(limit: 8 * 1024)
        let drained = DispatchGroup()
        for (capture, handle) in [(output, outputPipe.fileHandleForReading), (errors, errorPipe.fileHandleForReading)] {
            drained.enter()
            DispatchQueue.global(qos: .utility).async { capture.consume(handle); drained.leave() }
        }
        if finished.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            return .failure(Failure(reason: "send status is uncertain; do not send again before checking the operation"))
        }
        _ = drained.wait(timeout: .now() + 2)
        guard process.terminationStatus == 0 else { return .failure(Failure(reason: errors.text)) }
        return .success(output.data)
    }
}

@MainActor
public final class SendActionStore: ObservableObject {
    @Published public private(set) var actions: [SendApproval] = []
    @Published public private(set) var busy = false
    @Published public var notice: String?
    public init() {}
    public func refresh(executableName: String) async {
        guard !busy else { return }
        let result = await Task.detached(priority: .utility) { SendActions.request(["action": "list"], executableName: executableName, timeout: 10) }.value
        if case let .success(bytes) = result, let updated = try? JSONDecoder().decode([SendApproval].self, from: bytes), updated != actions {
            actions = updated
        }
    }
    public func decide(_ action: SendApproval, approve: Bool, executableName: String) async {
        guard !busy else { return }
        busy = true
        let request = ["action": approve ? "confirm" : "reject", "id": action.id, "code": action.code]
        let result = await Task.detached(priority: .userInitiated) { SendActions.request(request, executableName: executableName) }.value
        switch result {
        case let .success(bytes):
            if let payload = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any] {
                let status = payload["submission_status"] as? String
                if approve {
                    if status == "accepted" {
                        notice = payload["sent_copy_status"] as? String == "pending" ? "Accepted by SMTP; Sent copy pending." : "Accepted by SMTP."
                    } else if status == "unknown" { notice = "SMTP acceptance is unknown. TorroMail will not send again automatically." }
                    else { notice = "SMTP did not accept the message." }
                }
            }
        case let .failure(error):
            NSLog("TorroMail: send action failed: %@", error.reason)
            notice = "The send status could not be checked. Review the operation before sending another message."
        }
        busy = false
        await refresh(executableName: executableName)
    }
}
