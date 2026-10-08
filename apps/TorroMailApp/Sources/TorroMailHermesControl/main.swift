import Foundation
import TorroMailKit

// One bounded JSON request over stdin; never print config, keys or child logs.
// This executable is signed with the app's team and runs as the target user.
KeychainStore.silenceInteractivePrompts()
let response: HermesControlResponse
do {
    var input = Data()
    while let chunk = try FileHandle.standardInput.read(upToCount: 8192), !chunk.isEmpty {
        input.append(chunk)
        guard input.count <= 256 * 1024 else { throw MCPClientSetup.Failure("Invalid Hermes bot selection.") }
    }
    let request = try JSONDecoder().decode(HermesControlRequest.self, from: input)
    let executable = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL
    if CommandLine.arguments.dropFirst().contains("--gui-session") {
        response = try HermesGUISession.run(request, executable: executable)
    } else {
        let root = HermesBots.root(home: FileManager.default.homeDirectoryForCurrentUser,
                                  overrideHome: ProcessInfo.processInfo.environment["HERMES_HOME"])
        response = try HermesBotControl(root: root, policyURL: PolicyDocument.defaultURL(),
                                       command: executable.deletingLastPathComponent().appendingPathComponent("torromail-mcp")).run(request)
    }
} catch {
    response = HermesControlResponse(problem: (error as? MCPClientSetup.Failure)?.reason ?? "The existing configuration could not be read.")
}
let output = try JSONEncoder().encode(response)
FileHandle.standardOutput.write(output)
FileHandle.standardOutput.write(Data([0x0A]))
