import TorroMailKit
import Foundation

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        FileHandle.standardError.write(Data("Contract failed: \(message)\n".utf8))
        Foundation.exit(1)
    }
}

let model = TorroMailModel.preview()

// Decisions, not options: the sidebar carries accounts plus exactly two
// general destinations (Settings, Log). There is no Diagnostics surface.
require(
    model.accounts.map(\.name) == ["Work", "Personal"],
    "accounts are the primary navigation"
)
require(
    model.selectedSidebarItem == .account("work"),
    "first account should be selected by default"
)

// Status only on exception: healthy accounts stay quiet, unverified ones ask
// for attention.
require(
    !model.accounts[0].connectionState.needsAttention,
    "a connected account must not demand attention"
)
require(
    model.accounts[1].connectionState.needsAttention,
    "an untested connection must ask for attention"
)
require(
    model.accounts[0].pendingActions.count == 1,
    "pending approvals belong to their account"
)

// The one lifecycle decision: start at login. Executable and transport are
// internal details and never user-facing.
require(
    model.generalSettings.launchAtLogin,
    "launch at login is the default"
)
require(
    model.generalSettings.mcpExecutable == "torromail-mcp",
    "supervisor knows its executable without exposing it in the UI"
)

// Account lifecycle stays in the GUI: add selects the new account, remove
// falls back to the first remaining one.
model.addAccount(name: "Club", email: "club@example.org", provider: .imapSmtp, loginMethod: .password)
require(model.accounts.count == 3, "wizard adds an account")
guard case let .account(newID) = model.selectedSidebarItem else {
    require(false, "adding an account selects it")
    fatalError("unreachable")
}
require(newID == model.accounts[2].id, "adding an account selects it")
require(
    model.accounts[2].connectionState == .notConfigured,
    "a fresh account starts unconfigured"
)

model.removeAccount(id: newID)
require(model.accounts.count == 2, "remove deletes the configuration")
require(
    model.selectedSidebarItem == .account("work"),
    "removing the selected account falls back to the first account"
)

// MCP executable resolution prefers the dev workspace before PATH.
let locator = MCPExecutableLocator(
    executableName: "torromail-mcp",
    workspaceRoot: "/repo"
)
require(
    locator.candidatePaths == ["/repo/target/debug/torromail-mcp", "torromail-mcp"],
    "MCP executable should be resolved from the dev workspace before PATH"
)
require(
    !MCPServerStatus.notFound("torromail-mcp").isRunning,
    "a missing executable is not a running server"
)
require(
    MCPServerStatus.running("torromail-mcp").isRunning,
    "running state should be observable"
)

print("TorroMailKit control-surface contract passed")
