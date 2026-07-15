import TorroMailKit
import Foundation

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        FileHandle.standardError.write(Data("Contract failed: \(message)\n".utf8))
        Foundation.exit(1)
    }
}

let model = TorroMailModel.preview()

// Decisions, not options: the sidebar carries exactly three destinations —
// Mail Accounts, Settings, Log. Individual accounts live one level deeper,
// as cards inside the accounts destination.
require(
    model.accounts.map(\.name) == ["Work", "Personal"],
    "accounts are the primary navigation"
)
require(
    model.selectedSidebarItem == .accounts,
    "the account list should be shown by default"
)
require(
    model.accountPath.isEmpty,
    "the account list starts without a drill-down"
)
require(
    model.pendingActionCount == 1,
    "the accounts item badges every waiting approval"
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
    !model.accounts[1].connectionState.isBroken,
    "untested credentials are not broken credentials"
)
require(
    ConnectionState.failed("auth rejected").isBroken,
    "rejected credentials show the red dot"
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

// Account lifecycle stays in the GUI: add opens the new account, remove
// drops back to the account list.
model.addAccount(name: "Club", email: "club@example.org", provider: .imapSmtp, loginMethod: .password)
require(model.accounts.count == 3, "wizard adds an account")
let newID = model.accounts[2].id
require(
    model.selectedSidebarItem == .accounts && model.accountPath == [newID],
    "adding an account opens its settings"
)
require(
    model.accounts[2].connectionState == .notConfigured,
    "a fresh account starts unconfigured"
)

model.removeAccount(id: newID)
require(model.accounts.count == 2, "remove deletes the configuration")
require(
    model.accountPath.isEmpty,
    "removing the open account falls back to the account list"
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
