import Foundation
import Darwin

/// SSH has a separate security session. A short-lived agent in this user's
/// existing GUI domain can reach the unlocked login keychain by signing team.
/// It neither unlocks a keychain nor enables consent dialogs. No agent survives
/// the request; request and response files are in a private temporary folder.
public enum HermesGUISession {
    public static func run(_ request: HermesControlRequest, executable: URL) throws -> HermesControlResponse {
        let manager = FileManager.default
        let directory = manager.temporaryDirectory.appendingPathComponent("torromail-hermes-\(UUID().uuidString)")
        try manager.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? manager.removeItem(at: directory) }
        let input = directory.appendingPathComponent("request.json")
        let output = directory.appendingPathComponent("response.json")
        let agentURL = directory.appendingPathComponent("agent.plist")
        let label = "com.torromail.hermes.\(UUID().uuidString)"
        let domain = "gui/\(getuid())"
        try HermesBotControl.write(try JSONEncoder().encode(request), to: input)
        var agent: [String: Any] = [
            "Label": label, "ProgramArguments": [executable.path], "RunAtLoad": true, "ProcessType": "Interactive",
            "StandardInPath": input.path, "StandardOutPath": output.path,
            "StandardErrorPath": directory.appendingPathComponent("errors.log").path
        ]
        if let home = ProcessInfo.processInfo.environment["HERMES_HOME"], !home.isEmpty {
            agent["EnvironmentVariables"] = ["HERMES_HOME": home]
        }
        try HermesBotControl.write(try PropertyListSerialization.data(fromPropertyList: agent, format: .xml, options: 0), to: agentURL)
        let launchctl = URL(fileURLWithPath: "/bin/launchctl")
        defer { _ = try? HermesProcess.run(executable: launchctl, arguments: ["bootout", "\(domain)/\(label)"], input: Data(), timeout: 5) }
        do {
            _ = try HermesProcess.run(executable: launchctl, arguments: ["bootstrap", domain, agentURL.path], input: Data(), timeout: 10)
        } catch {
            throw MCPClientSetup.Failure("The macOS login session is not available. Sign in on the target computer and try again.")
        }
        let timeout = request.action == .list ? 15 : Double(max(1, request.selections.count)) * 17 + 5
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let data = try? Data(contentsOf: output), data.count <= 2 * 1024 * 1024,
               let response = try? JSONDecoder().decode(HermesControlResponse.self, from: data) { return response }
            Thread.sleep(forTimeInterval: 0.05)
        }
        throw MCPClientSetup.Failure("The Hermes setup helper did not respond in time.")
    }
}
